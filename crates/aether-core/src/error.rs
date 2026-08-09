//! Shared error type for the aether workspace.

use thiserror::Error;

/// Workspace-wide error type. Library crates convert their own errors into
/// this enum; application crates may wrap it in `anyhow`.
#[derive(Debug, Error)]
pub enum Error {
    /// Invalid or inconsistent configuration.
    #[error("config error: {0}")]
    Config(String),
    /// Filesystem error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// A path component that would escape a sandboxed base directory.
    #[error("path traversal blocked: {0:?}")]
    PathTraversal(String),
    /// Provider-level failure (bad status, malformed stream, ...).
    #[error("provider error: {0}")]
    Provider(String),
    /// Network/transport failure.
    #[error("network error: {0}")]
    Network(String),
    /// Invalid user input.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

/// Convenience alias used across the workspace.
pub type Result<T> = std::result::Result<T, Error>;

/// Redact a secret for debug output. Never print full API keys.
///
/// Empty values become `"<unset>"`; short values are fully masked; longer
/// values keep only the first and last two characters.
pub fn redact_secret(value: &str) -> String {
    if value.is_empty() {
        return "<unset>".to_string();
    }
    if value.len() <= 4 {
        return "****".to_string();
    }
    let mut out = String::with_capacity(value.len());
    out.push_str(&value[..2]);
    out.push('…');
    out.push_str(&value[value.len() - 2..]);
    out
}

#[cfg(test)]
mod tests {
    use super::redact_secret;

    #[test]
    fn redacts_secrets() {
        assert_eq!(redact_secret(""), "<unset>");
        assert_eq!(redact_secret("ab"), "****");
        assert_eq!(redact_secret("abcdefgh"), "ab…gh");
    }
}
