use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

/// Stable, redacted public errors. Internal errors never echo SQL, paths, tokens or payloads.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Error {
    pub code: String,
    pub message: String,
}
impl Error {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn invalid(message: &str) -> Self {
        Self::new("INVALID_INPUT", message)
    }
    pub fn denied() -> Self {
        Self::new("PERMISSION_DENIED", "Operation is not authorized")
    }
    pub fn not_found() -> Self {
        Self::new("NOT_FOUND", "No authorized object matches")
    }
    pub fn conflict() -> Self {
        Self::new(
            "VERSION_CONFLICT",
            "Refresh state and retry with its current version",
        )
    }
    pub fn status(&self) -> u16 {
        match self.code.as_str() {
            "BAD_REQUEST" => 400,
            "METHOD_NOT_ALLOWED" => 405,
            "AUTH_REQUIRED" => 401,
            "PERMISSION_DENIED" | "POLICY_DENIED" => 403,
            "NOT_FOUND" => 404,
            "VERSION_CONFLICT"
            | "POLICY_CHANGED"
            | "DB_INCOMPATIBLE"
            | "CANCELLED"
            | "GOAL_ALREADY_MET"
            | "TYPE_REGISTRY_CONFLICT" => 409,
            "LIMIT_EXCEEDED" => 413,
            "INVALID_INPUT"
            | "TYPE_UNKNOWN"
            | "MODEL_OUTPUT_INVALID"
            | "INFERENCE_DISABLED"
            | "UNSUPPORTED_SCHEMA" => 422,
            "BACKPRESSURE" | "RATE_LIMITED" => 429,
            "ADAPTER_UNAVAILABLE"
            | "UNSUPPORTED_PROTOCOL"
            | "DEADLINE_EXCEEDED"
            | "CORE_UNAVAILABLE"
            | "MODEL_UNAVAILABLE"
            | "OFFLINE"
            | "READ_ONLY_RECOVERY"
            | "UNSUPPORTED_PLATFORM" => 503,
            _ => 500,
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::new("IO_ERROR", "Local I/O operation failed")
    }
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::new(
            "STORAGE_ERROR",
            "Database operation failed; no automatic repair was attempted",
        )
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::invalid("Malformed or unsupported JSON structure")
    }
}
impl From<reqwest::Error> for Error {
    fn from(_: reqwest::Error) -> Self {
        Self::new(
            "NETWORK_ERROR",
            "Approved request failed or exceeded its deadline",
        )
    }
}

pub fn require(ok: bool, code: &str, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::new(code, message))
    }
}
