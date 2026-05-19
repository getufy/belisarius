//! Single error type for the service layer. Transports translate it at the
//! edge — HTTP via `impl From<ServiceError> for AppError`, MCP via the
//! JSON-RPC error envelope.

use thiserror::Error;

// Variants and constructors below are deliberately ahead of the migration —
// `NotFound`, `MissingIndex`, and the helper constructors are used by
// upcoming feature modules (symbols, search). Silencing dead-code warnings
// here keeps the public API stable as those modules land.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    /// A capability that requires an external index (SCIP symbols, search
    /// embeddings, …) was asked for but the index is missing. `hint` is the
    /// CLI command to produce it.
    #[error("{which} index missing: {hint}")]
    MissingIndex { which: &'static str, hint: String },

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[allow(dead_code)]
impl ServiceError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn missing_index(which: &'static str, hint: impl Into<String>) -> Self {
        Self::MissingIndex {
            which,
            hint: hint.into(),
        }
    }
}

impl From<serde_json::Error> for ServiceError {
    fn from(e: serde_json::Error) -> Self {
        Self::BadRequest(format!("invalid arguments: {e}"))
    }
}

#[allow(dead_code)]
impl ServiceError {
    /// Stable numeric code for MCP clients. User errors are 1xxx, system errors 2xxx.
    pub fn code(&self) -> i32 {
        match self {
            Self::BadRequest(_) => 1001,
            Self::NotFound(_) => 1002,
            Self::MissingIndex { .. } => 1003,
            Self::Internal(_) => 2001,
        }
    }

    /// Short discriminant string. Pairs with `code` for clients that prefer strings.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::MissingIndex { .. } => "missing_index",
            Self::Internal(_) => "internal",
        }
    }

    /// CLI-command remediation hint when one is known. Agents can use this to
    /// self-heal (e.g. run `belisarius index .` after a `missing_index` error).
    pub fn remediation(&self) -> Option<String> {
        match self {
            Self::MissingIndex { hint, .. } => Some(hint.clone()),
            _ => None,
        }
    }

    /// Structured payload for MCP `isError: true` responses and JSON-RPC
    /// `error.data` envelopes. Includes code, kind, and any variant-specific
    /// context (e.g. which index is missing).
    pub fn data(&self) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "code": self.code(),
            "kind": self.kind(),
        });
        if let Some(rem) = self.remediation() {
            obj["remediation"] = serde_json::Value::String(rem);
        }
        if let Self::MissingIndex { which, .. } = self {
            obj["which"] = serde_json::Value::String((*which).to_string());
        }
        obj
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_distinct() {
        assert_eq!(ServiceError::bad_request("x").code(), 1001);
        assert_eq!(ServiceError::not_found("x").code(), 1002);
        assert_eq!(ServiceError::missing_index("scip", "run x").code(), 1003);
    }

    #[test]
    fn missing_index_data_carries_remediation_and_which() {
        let err = ServiceError::missing_index("scip", "run `belisarius index .`");
        let data = err.data();
        assert_eq!(data["code"], 1003);
        assert_eq!(data["kind"], "missing_index");
        assert_eq!(data["which"], "scip");
        assert_eq!(data["remediation"], "run `belisarius index .`");
    }

    #[test]
    fn bad_request_data_omits_remediation() {
        let err = ServiceError::bad_request("invalid arg");
        let data = err.data();
        assert_eq!(data["code"], 1001);
        assert_eq!(data["kind"], "bad_request");
        assert!(data.get("remediation").is_none());
        assert!(data.get("which").is_none());
    }
}
