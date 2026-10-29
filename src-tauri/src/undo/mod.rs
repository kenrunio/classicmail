//! App-wide Undo / Redo (Ctrl+Z / Ctrl+Y), shared by every window.
//!
//! An in-memory stack of entries `{id, label, at, undo: [op], redo: [op]}`; an op is an rpc call
//! `{method, params, save?}` replayed through the rpc registry, so undoing = performing the inverse
//! rpc(s). Ops can name their result (`save: "x"`) and later ops reference it in params as the
//! string `"$x"` or `"$x.path.0"` — that is how a re-created item's new id reaches redo.
//!
//! Two ways in:
//! * `undo.run {label, method, params}` — perform an action and record its inverse, computed here
//!   from the state before the action (`inverse.rs` knows the reversible mail / calendar / contacts /
//!   tasks / folder / settings methods; anything else just runs).
//! * `undo.runMany {label, calls: [{method, params}]}` — several actions as one undo step.
//! * `undo.push {label, undo, redo}` — the front end supplies the ops itself.
//!
//! `undo.undo` / `undo.redo` / `undo.peek` / `undo.clear`; `undo.changed {undoLabel, redoLabel,
//! message?}` after every change. Size limit: setting `app.undoSteps` (default 20, 1–100).
//! Not persisted: the history is gone when ClassicMail closes.

mod inverse;
#[cfg(test)]
mod tests;

use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    ctx::AppCtx,
    rpc::{NoParams, Registry},
};

pub const DEFAULT_STEPS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Op {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// remember this op's result under a name (`"$name"` in later params)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
}

impl Op {
    pub fn new(method: &str, params: Value) -> Op {
        Op { method: method.into(), params, save: None }
    }
    pub fn saving(mut self, name: &str) -> Op {
        self.save = Some(name.into());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    pub label: String,
    pub at: i64,
    pub undo: Vec<Op>,
    pub redo: Vec<Op>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub vars: Map<String, Value>,
}

#[derive(Default)]
struct Stacks {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_id: u64,
}

/// Lives in `AppCtx` (one per app run).
#[derive(Default)]
pub struct UndoStack {
    st: Mutex<Stacks>,
    /// one undo / redo at a time
    busy: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Peek {
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub undo_count: usize,
    pub redo_count: usize,
}

impl UndoStack {
    pub fn peek(&self) -> Peek {
        let s = self.st.lock();
        Peek {
            undo_label: s.undo.last().map(|e| e.label.clone()),
            redo_label: s.redo.last().map(|e| e.label.clone()),
            undo_count: s.undo.len(),
            redo_count: s.redo.len(),
        }
    }

    /// A new user action: push it, clear redo, trim to `limit` (oldest go first).
    pub fn push(&self, label: String, undo: Vec<Op>, redo: Vec<Op>, vars: Map<String, Value>, limit: usize) -> u64 {
        let mut s = self.st.lock();
        s.next_id += 1;
        let id = s.next_id;
        s.undo.push(Entry { id, label, at: crate::db::util::now_ms(), undo, redo, vars });
        s.redo.clear();
        let limit = limit.clamp(1, 100);
        let n = s.undo.len();
        if n > limit {
            s.undo.drain(..n - limit);
        }
        id
    }

    pub fn clear(&self) {
        let mut s = self.st.lock();
        s.undo.clear();
        s.redo.clear();
    }

    pub fn trim(&self, limit: usize) {
        let mut s = self.st.lock();
        let limit = limit.clamp(1, 100);
        let n = s.undo.len();
        if n > limit {
            s.undo.drain(..n - limit);
        }
        let n = s.redo.len();
        if n > limit {
            s.redo.drain(..n - limit);
        }
    }

    pub fn entries(&self) -> (Vec<Entry>, Vec<Entry>) {
        let s = self.st.lock();
        (s.undo.clone(), s.redo.clone())
    }
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(crate::rpc::build)
}

/// `"$name"` / `"$name.a.0"` → the saved value (left as is when there's no such variable).
pub fn subst(v: &Value, vars: &Map<String, Value>) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') && s.len() > 1 => {
            let mut parts = s[1..].split('.');
            let Some(cur) = parts.next().and_then(|n| vars.get(n)) else { return v.clone() };
            let mut cur = cur;
            for p in parts {
                let next = match cur {
                    Value::Array(a) => p.parse::<usize>().ok().and_then(|i| a.get(i)),
                    Value::Object(o) => o.get(p),
                    _ => None,
                };
                match next {
                    Some(n) => cur = n,
                    None => return v.clone(),
                }
            }
            cur.clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, vars)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), subst(x, vars))).collect()),
        _ => v.clone(),
    }
}

/// Run ops in order through the registry, saving named results into `vars`.
pub async fn run_ops(ctx: &Arc<AppCtx>, reg: &Registry, ops: &[Op], vars: &mut Map<String, Value>) -> anyhow::Result<()> {
    for op in ops {
        let params = subst(&op.params, vars);
        let out = reg.call(ctx.clone(), &op.method, params).await?;
        if let Some(name) = &op.save {
            vars.insert(name.clone(), out);
        }
    }
    Ok(())
}

pub async fn limit(ctx: &Arc<AppCtx>) -> usize {
    let v: Option<i64> = ctx.db.read(|c| crate::db::util::get_setting(c, "app.undoSteps")).await.ok().flatten();
    v.map(|n| n.clamp(1, 100) as usize).unwrap_or(DEFAULT_STEPS)
}

fn changed(ctx: &Arc<AppCtx>, message: Option<String>) {
    let p = ctx.undo.peek();
    ctx.events.emit("undo.changed", json!({ "undoLabel": p.undo_label, "redoLabel": p.redo_label, "message": message }));
}

/// Forget the whole history (e.g. an account was removed: its entries name its items).
pub fn clear(ctx: &Arc<AppCtx>) {
    ctx.undo.clear();
    changed(ctx, None);
}

pub async fn push(ctx: &Arc<AppCtx>, label: String, undo: Vec<Op>, redo: Vec<Op>, vars: Map<String, Value>) -> u64 {
    let n = limit(ctx).await;
    let id = ctx.undo.push(label, undo, redo, vars, n);
    changed(ctx, None);
    id
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    Undo,
    Redo,
}

/// Undo (or redo) the top entry. Ok(None) = nothing to do.
async fn step(ctx: &Arc<AppCtx>, reg: &Registry, dir: Dir) -> anyhow::Result<Option<String>> {
    let _g = ctx.undo.busy.lock().await;
    let entry = {
        let mut s = ctx.undo.st.lock();
        match dir {
            Dir::Undo => s.undo.pop(),
            Dir::Redo => s.redo.pop(),
        }
    };
    let Some(mut entry) = entry else { return Ok(None) };
    let ops = if dir == Dir::Undo { entry.undo.clone() } else { entry.redo.clone() };
    match run_ops(ctx, reg, &ops, &mut entry.vars).await {
        Ok(()) => {
            let label = entry.label.clone();
            {
                let mut s = ctx.undo.st.lock();
                match dir {
                    Dir::Undo => s.redo.push(entry),
                    Dir::Redo => s.undo.push(entry),
                }
            }
            let msg = format!("{}: {label}", if dir == Dir::Undo { "Undid" } else { "Redid" });
            changed(ctx, Some(msg));
            Ok(Some(label))
        }
        Err(e) => {
            // the entry is dropped: its items changed (or vanished) in the meantime
            let what = if dir == Dir::Undo { "undo" } else { "redo" };
            let reason = match e.downcast_ref::<crate::error::UserError>() {
                Some(u) => u.message.clone(),
                None => format!("{e:#}"),
            };
            // the entry id: its label names the items it changed
            log::warn!("{what} of entry {} failed: {}", entry.id, crate::logging::err(&e));
            let msg = format!("Couldn't {what} \"{}\": {reason}", entry.label);
            changed(ctx, Some(msg.clone()));
            Err(crate::user_err!("undo_failed", "{msg}"))
        }
    }
}

// ---- rpc -----------------------------------------------------------------------------------------

pub fn register(r: &mut Registry) {
    r.add("undo.push", push_rpc);
    r.add("undo.run", run_rpc);
    r.add("undo.runMany", run_many_rpc);
    r.add("undo.undo", |ctx: Arc<AppCtx>, _: NoParams| async move { Ok(json!({ "label": step(&ctx, registry(), Dir::Undo).await? })) });
    r.add("undo.redo", |ctx: Arc<AppCtx>, _: NoParams| async move { Ok(json!({ "label": step(&ctx, registry(), Dir::Redo).await? })) });
    r.add("undo.peek", |ctx: Arc<AppCtx>, _: NoParams| async move { Ok(ctx.undo.peek()) });
    r.add("undo.clear", |ctx: Arc<AppCtx>, _: NoParams| async move {
        clear(&ctx);
        Ok(())
    });
    r.add("undo.setLimit", |ctx: Arc<AppCtx>, p: LimitParams| async move {
        ctx.undo.trim(p.steps.clamp(1, 100) as usize);
        changed(&ctx, None);
        Ok(())
    });
    inverse::register(r);
}

#[derive(Deserialize)]
struct LimitParams {
    steps: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushParams {
    label: String,
    undo: Vec<Op>,
    redo: Vec<Op>,
    #[serde(default)]
    vars: Map<String, Value>,
}

async fn push_rpc(ctx: Arc<AppCtx>, p: PushParams) -> anyhow::Result<u64> {
    if p.undo.is_empty() {
        return Err(crate::user_err!("bad_params", "undo.push needs at least one undo op"));
    }
    for op in p.undo.iter().chain(p.redo.iter()) {
        if op.method.starts_with("undo.") && !inverse::is_helper(&op.method) {
            return Err(crate::user_err!("bad_params", "{} can't be an undo op", op.method));
        }
    }
    Ok(push(&ctx, p.label, p.undo, p.redo, p.vars).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunParams {
    label: String,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Perform `method(params)` and record how to undo it. Returns the method's own result.
async fn run_rpc(ctx: Arc<AppCtx>, p: RunParams) -> anyhow::Result<Value> {
    run_with(&ctx, registry(), &p.label, &p.method, p.params).await
}

#[derive(Deserialize)]
struct Call {
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunManyParams {
    label: String,
    calls: Vec<Call>,
}

/// Prefix every variable of one recording (so several can share an entry).
fn prefix_vars(rec: &mut inverse::Recorded, pre: &str) {
    fn walk(v: &mut Value, names: &[String], pre: &str) {
        match v {
            Value::String(s) if s.starts_with('$') => {
                let first = s[1..].split('.').next().unwrap_or("").to_string();
                if names.contains(&first) {
                    *s = format!("${pre}{}", &s[1..]);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, names, pre)),
            Value::Object(o) => o.values_mut().for_each(|x| walk(x, names, pre)),
            _ => {}
        }
    }
    let mut names: Vec<String> = rec.vars.keys().cloned().collect();
    names.extend(rec.undo.iter().chain(rec.redo.iter()).filter_map(|o| o.save.clone()));
    for op in rec.undo.iter_mut().chain(rec.redo.iter_mut()) {
        walk(&mut op.params, &names, pre);
        if let Some(s) = &op.save {
            op.save = Some(format!("{pre}{s}"));
        }
    }
    rec.vars = std::mem::take(&mut rec.vars).into_iter().map(|(k, v)| (format!("{pre}{k}"), v)).collect();
}

/// Several actions as ONE undo step (e.g. "Add 3 contacts to group"). Stops at the first failure;
/// what already ran is still recorded. Returns the results in order.
async fn run_many_rpc(ctx: Arc<AppCtx>, p: RunManyParams) -> anyhow::Result<Vec<Value>> {
    let reg = registry();
    let (mut undo, mut redo, mut vars) = (Vec::new(), Vec::new(), Map::new());
    let mut outs = Vec::new();
    let mut err = None;
    for (i, c) in p.calls.into_iter().enumerate() {
        if c.method.starts_with("undo.") {
            err = Some(crate::user_err!("bad_params", "{} can't be recorded", c.method));
            break;
        }
        let params = if c.params.is_null() { json!({}) } else { c.params };
        let prep = inverse::prepare(&ctx, &c.method, &params).await.ok().flatten();
        let out = match reg.call(ctx.clone(), &c.method, params.clone()).await {
            Ok(o) => o,
            Err(e) => {
                err = Some(e);
                break;
            }
        };
        if let Some(prep) = prep
            && let Ok(Some(mut rec)) = inverse::finish(&ctx, prep, &params, &out).await
        {
            prefix_vars(&mut rec, &format!("k{i}_"));
            // undo runs the inverses newest first
            undo.splice(0..0, rec.undo);
            redo.extend(rec.redo);
            vars.extend(rec.vars);
        }
        outs.push(out);
    }
    if !undo.is_empty() {
        push(&ctx, p.label, undo, redo, vars).await;
    }
    match err {
        Some(e) => Err(e),
        None => Ok(outs),
    }
}

pub async fn run_with(ctx: &Arc<AppCtx>, reg: &Registry, label: &str, method: &str, params: Value) -> anyhow::Result<Value> {
    if method.starts_with("undo.") {
        return Err(crate::user_err!("bad_params", "{method} can't be recorded"));
    }
    let params = if params.is_null() { json!({}) } else { params };
    let prep = match inverse::prepare(ctx, method, &params).await {
        Ok(p) => p,
        Err(e) => {
            log::debug!("undo: no inverse for {method}: {}", crate::logging::err(&e));
            None
        }
    };
    let out = reg.call(ctx.clone(), method, params.clone()).await?;
    if let Some(prep) = prep {
        match inverse::finish(ctx, prep, &params, &out).await {
            Ok(Some(rec)) => {
                push(ctx, label.to_string(), rec.undo, rec.redo, rec.vars).await;
            }
            Ok(None) => {}
            Err(e) => log::warn!("undo: couldn't record {method}: {}", crate::logging::err(&e)),
        }
    }
    Ok(out)
}
