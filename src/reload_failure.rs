//! Bounded, serializable candidate failure reports (also usable without hot-reload).
use crate::RemoteError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadPhase {
    Compilation,
    Startup,
    Protocol,
    Registration,
    Publication,
}

/// Exit observed before cleanup. `None` code/signal does not imply success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}
impl From<std::process::ExitStatus> for ProcessExit {
    fn from(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        Self {
            code: status.code(),
            #[cfg(unix)]
            signal: status.signal(),
            #[cfg(not(unix))]
            signal: None,
        }
    }
}

/// Project-relative location. External/dependency paths are omitted; source text
/// and suggestions are deliberately not retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerSpan {
    pub file: Option<String>,
    pub line_start: u64,
    pub line_end: u64,
    pub column_start: u64,
    pub column_end: u64,
    pub is_primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerDiagnostic {
    pub code: Option<String>,
    pub level: String,
    pub message: String,
    pub spans: Vec<CompilerSpan>,
    pub children: Vec<CompilerDiagnostic>,
}

/// Candidate preparation/publication failed. Local routes were not replaced.
/// Compilation may execute build scripts/macros; this is not a sandbox receipt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotReloadFailure {
    pub phase: ReloadPhase,
    pub message: String,
    pub diagnostics: Vec<CompilerDiagnostic>,
    /// At least one field/message was omitted or shortened to enforce bounds.
    pub truncated: bool,
    pub exit: Option<ProcessExit>,
    pub timed_out: bool,
    pub candidate_started: bool,
    /// Whether remote slash-command synchronization was attempted.
    pub publication_attempted: bool,
    /// Local route publication; remote API outcomes belong to `source`.
    pub routes_published: bool,
    pub source: Option<RemoteError>,
}
impl std::fmt::Display for HotReloadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hot reload {:?}: {}", self.phase, self.message)
    }
}
impl std::error::Error for HotReloadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|error| error as _)
    }
}
impl HotReloadFailure {
    #[cfg(feature = "hot-reload")]
    pub(crate) fn new(phase: ReloadPhase, message: &str) -> Self {
        Self {
            phase,
            message: message.into(),
            diagnostics: vec![],
            truncated: false,
            exit: None,
            timed_out: false,
            candidate_started: false,
            publication_attempted: false,
            routes_published: false,
            source: None,
        }
    }
}
