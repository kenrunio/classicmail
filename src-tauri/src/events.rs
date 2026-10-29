//! Back end → every window. One Tauri event, `cm-event`, payload `{ kind, ...data }`.
//! Tests construct `Events::capture()` and inspect what was emitted.

use std::{collections::BTreeMap, sync::Arc};

use parking_lot::Mutex;
use serde_json::{Value, json};

type Sink = Arc<dyn Fn(Value) + Send + Sync>;

#[derive(Clone)]
pub struct Events {
    sink: Sink,
    captured: Option<Arc<Mutex<Vec<Value>>>>,
    /// last `sync.status` per account, for windows that start listening after it was sent
    last_status: Arc<Mutex<BTreeMap<i64, Value>>>,
}

impl Events {
    pub fn tauri(app: tauri::AppHandle) -> Self {
        use tauri::Emitter;
        Events {
            sink: Arc::new(move |v| {
                let _ = app.emit("cm-event", v);
            }),
            captured: None,
            last_status: Default::default(),
        }
    }

    /// For tests: keeps everything emitted.
    pub fn capture() -> Self {
        let store = Arc::new(Mutex::new(Vec::new()));
        let s2 = store.clone();
        Events { sink: Arc::new(move |v| s2.lock().push(v)), captured: Some(store), last_status: Default::default() }
    }

    pub fn captured(&self) -> Vec<Value> {
        self.captured.as_ref().map(|c| c.lock().clone()).unwrap_or_default()
    }

    /// Emit `{kind, ...data}`; `data` must be a JSON object (or null).
    pub fn emit(&self, kind: &str, data: Value) {
        let mut obj = match data {
            Value::Object(m) => m,
            _ => Default::default(),
        };
        obj.insert("kind".into(), Value::String(kind.into()));
        (self.sink)(Value::Object(obj));
    }

    pub fn mail_changed(&self, account_id: i64, folder_ids: Option<Vec<i64>>) {
        self.emit("mail.changed", json!({ "accountId": account_id, "folderIds": folder_ids }));
    }

    pub fn status(&self, account_id: i64, phase: &str, text: &str, progress: Option<f64>) {
        let v = json!({ "accountId": account_id, "phase": phase, "text": text, "progress": progress });
        self.last_status.lock().insert(account_id, v.clone());
        self.emit("sync.status", v);
    }

    /// The latest `sync.status` of every account (the status bar asks on start-up).
    pub fn statuses(&self) -> Vec<Value> {
        self.last_status.lock().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_the_last_status_per_account() {
        let ev = Events::capture();
        ev.status(1, "sync", "Receiving…", Some(0.5));
        ev.status(1, "idle", "All folders are up to date.", None);
        ev.status(2, "error", "Send/Receive error", None);
        let s = ev.statuses();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["text"], "All folders are up to date.");
        assert_eq!(s[1]["phase"], "error");
        assert_eq!(ev.captured().len(), 3);
    }
}
