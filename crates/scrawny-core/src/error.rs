use std::fmt;
use std::path::PathBuf;

/// Typed errors for Scrawny. Every variant carries enough context to act on
/// without re-running the failing operation to find out what happened.
#[derive(Debug, thiserror::Error)]
pub enum ScrawnyError {
    #[error("not a git repository: {path}")]
    NotRepository { path: PathBuf },

    #[error("git command failed: {command}\n{stderr}")]
    GitFailure { command: String, stderr: String },

    #[error("invalid diff input: {reason}")]
    InvalidDiff { reason: String },

    #[error("unsupported language for {path}: {language}")]
    UnsupportedLanguage { path: PathBuf, language: String },

    #[error("failed to parse {path}: {reason}")]
    ParseFailure { path: PathBuf, reason: String },

    #[error("configuration error in {config_path}: {reason}")]
    Configuration { config_path: String, reason: String },

    #[error("analysis error: {reason}")]
    Analysis { reason: String },

    #[error("validation failed: {reason}")]
    Validation { reason: String },

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// A finding that could not be resolved with full confidence.
///
/// Analysis failures must never be silently represented as safe (see the
/// project directive, "Critical Invariants"): a degraded classification
/// carries its own confidence rather than defaulting to a clean bill.
#[derive(Debug, Clone)]
pub struct Degraded {
    pub reason: String,
}

impl fmt::Display for Degraded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "analysis continued in degraded mode: {}", self.reason)
    }
}

pub type Result<T> = std::result::Result<T, ScrawnyError>;
