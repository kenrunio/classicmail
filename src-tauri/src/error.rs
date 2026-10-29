use serde::Serialize;

/// Error returned to the front end from `rpc`. Internally everything is `anyhow`; an error whose
/// root cause is a `UserError` keeps its code, anything else becomes `internal`.
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

/// An error meant for the user (bad input, not found, not signed in...). Use `user_err!`.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct UserError {
    pub code: &'static str,
    pub message: String,
}

#[macro_export]
macro_rules! user_err {
    ($code:expr, $($arg:tt)*) => {
        anyhow::Error::new($crate::error::UserError { code: $code, message: format!($($arg)*) })
    };
}

impl From<anyhow::Error> for RpcError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(u) = e.downcast_ref::<UserError>() {
            return RpcError { code: u.code.to_string(), message: u.message.clone() };
        }
        if let Some(g) = e.downcast_ref::<crate::google::client::ApiError>() {
            return RpcError { code: "google".into(), message: g.to_string() };
        }
        RpcError { code: "internal".into(), message: format!("{e:#}") }
    }
}

pub type Result<T> = anyhow::Result<T>;
