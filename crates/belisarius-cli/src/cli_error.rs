//! Structured CLI error envelope.
//!
//! `CliError` carries a stable numeric `code` and short discriminant `kind`
//! alongside the human `message` so transports (CLI text, `--json`,
//! JSON-RPC `error.data`) all emit the same shape.
//!
//! Codes mirror the service-layer envelope in `service::error::ServiceError`:
//!   1001  bad_request
//!   1002  not_found
//!   1003  missing_index
//!   2001  internal
//!
//! Use the helper constructors instead of building the struct directly so the
//! `(code, kind)` pair stays consistent.

use serde_json::{json, Value};
use std::fmt;

#[derive(Debug, Clone)]
#[allow(dead_code)] // remediation/constructors land alongside the migration off bail!().
pub struct CliError {
    pub code: u16,
    pub kind: &'static str,
    pub message: String,
    pub remediation: Option<String>,
}

#[allow(dead_code)]
impl CliError {
    pub fn new(code: u16, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            kind,
            message: message.into(),
            remediation: None,
        }
    }

    pub fn with_remediation(mut self, hint: impl Into<String>) -> Self {
        self.remediation = Some(hint.into());
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(1001, "bad_request", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(1002, "not_found", message)
    }

    pub fn missing_index(which: &str, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        Self::new(1003, "missing_index", format!("{which} index missing")).with_remediation(hint)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(2001, "internal", message)
    }

    /// JSON shape suitable for `--json` output and JSON-RPC `error.data`.
    pub fn to_json(&self) -> Value {
        let mut obj = json!({
            "code": self.code,
            "kind": self.kind,
            "message": self.message,
        });
        if let Some(rem) = &self.remediation {
            obj["remediation"] = Value::String(rem.clone());
        }
        obj
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}/{}]: {}", self.code, self.kind, self.message)?;
        if let Some(rem) = &self.remediation {
            write!(f, "\n  try: {rem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CliError {}

/// Bridge: convert a `ServiceError` (the canonical envelope used by all
/// service handlers) into a `CliError` so a binary's `main()` can render it
/// uniformly regardless of which layer produced it.
impl From<crate::service::error::ServiceError> for CliError {
    fn from(e: crate::service::error::ServiceError) -> Self {
        let message = e.to_string();
        let code = e.code() as u16;
        let kind: &'static str = match code {
            1001 => "bad_request",
            1002 => "not_found",
            1003 => "missing_index",
            _ => "internal",
        };
        let remediation = e.remediation();
        Self {
            code,
            kind,
            message,
            remediation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_request_code_and_kind() {
        let e = CliError::bad_request("nope");
        assert_eq!(e.code, 1001);
        assert_eq!(e.kind, "bad_request");
    }

    #[test]
    fn json_includes_remediation_when_set() {
        let e = CliError::missing_index("scip", "belisarius index .");
        let v = e.to_json();
        assert_eq!(v["code"], 1003);
        assert_eq!(v["kind"], "missing_index");
        assert_eq!(v["remediation"], "belisarius index .");
    }

    #[test]
    fn json_omits_remediation_when_absent() {
        let e = CliError::bad_request("x");
        let v = e.to_json();
        assert!(v.get("remediation").is_none());
    }

    #[test]
    fn display_formats_code_kind_and_message() {
        let s = format!("{}", CliError::not_found("nothing here"));
        assert!(s.contains("1002"));
        assert!(s.contains("not_found"));
        assert!(s.contains("nothing here"));
    }
}
