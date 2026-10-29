//! The single front-end entry point: `rpc(method, params)`. Every domain registers its methods in
//! its own `register(&mut Registry)`, so no shared list has to be edited when methods are added.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::ctx::AppCtx;

pub type HandlerFut = Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>;
pub type Handler = Arc<dyn Fn(Arc<AppCtx>, Value) -> HandlerFut + Send + Sync>;

/// Params type for methods that take none (the front end sends `{}`).
#[derive(Debug, Default, serde::Deserialize)]
pub struct NoParams {}

#[derive(Default)]
pub struct Registry {
    map: HashMap<&'static str, Handler>,
}

impl Registry {
    /// Register `name` → async fn(ctx, params) -> Result<R>.
    pub fn add<P, R, F, Fut>(&mut self, name: &'static str, f: F)
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + 'static,
        F: Fn(Arc<AppCtx>, P) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
    {
        let f = Arc::new(f);
        let h: Handler = Arc::new(move |ctx, raw| {
            let f = f.clone();
            Box::pin(async move {
                let raw = if raw.is_null() { Value::Object(Default::default()) } else { raw };
                let params: P = serde_json::from_value(raw).map_err(|e| crate::user_err!("bad_params", "{name}: {e}"))?;
                let out = f(ctx, params).await?;
                Ok(serde_json::to_value(out)?)
            })
        });
        if self.map.insert(name, h).is_some() {
            panic!("rpc method registered twice: {name}");
        }
    }

    pub fn get(&self, name: &str) -> Option<Handler> {
        self.map.get(name).cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        let mut v: Vec<_> = self.map.keys().copied().collect();
        v.sort();
        v
    }

    pub async fn call(&self, ctx: Arc<AppCtx>, method: &str, params: Value) -> anyhow::Result<Value> {
        let h = self.get(method).ok_or_else(|| crate::user_err!("no_method", "unknown method {method}"))?;
        h(ctx, params).await
    }
}

/// Every domain's registration. The integrator owns this list.
pub fn build() -> Registry {
    let mut r = Registry::default();
    crate::core_api::register(&mut r);
    crate::spare::register(&mut r);
    crate::keyboard::register(&mut r);
    crate::clipboard::register(&mut r);
    crate::mail::register(&mut r);
    crate::mapi::register(&mut r);
    crate::calendar::register(&mut r);
    crate::contacts::register(&mut r);
    crate::tasks::register(&mut r);
    crate::rules::register(&mut r);
    crate::ai::register(&mut r);
    crate::archive::register(&mut r);
    crate::undo::register(&mut r);
    r
}
