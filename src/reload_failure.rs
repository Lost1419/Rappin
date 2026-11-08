//! Bounded, serializable candidate failure reports (also usable without hot-reload).
use crate::RemoteError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Candidate compilation, startup and publication stages.
///
/// # Example
///
/// ```rust
/// let phase = rappin::ReloadPhase::Compilation;
/// assert_eq!(serde_json::to_value(phase).unwrap(), "compilation");
/// ```
pub enum ReloadPhase {
    /// Cargo compilation or build command failed.
    Compilation,
    /// The candidate process could not start or exited before readiness.
    Startup,
    /// The candidate readiness/exchange protocol was invalid or unsupported.
    Protocol,
    /// The candidate route manifest conflicted with local registration requirements.
    Registration,
    /// Publishing the prepared candidate or its remote commands failed.
    Publication,
}

/// Exit observed before cleanup. `None` code/signal does not imply success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let exit = rappin::ProcessExit { code: Some(101), signal: None };
/// assert_ne!(exit.code, Some(0));
/// ```
pub struct ProcessExit {
    /// Exit code when the operating system provides one.
    pub code: Option<i32>,
    /// Terminating Unix signal, when available; absent on other platforms.
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
/// # Example
///
/// ```rust
/// let span = rappin::CompilerSpan { file: Some("src/lib.rs".into()), line_start: 10, line_end: 10, column_start: 5, column_end: 9, is_primary: true };
/// assert_eq!(span.file.as_deref(), Some("src/lib.rs"));
/// ```
pub struct CompilerSpan {
    /// Project-relative source file, or None for omitted external paths.
    pub file: Option<String>,
    /// One-based starting source line.
    pub line_start: u64,
    /// One-based ending source line.
    pub line_end: u64,
    /// One-based starting source column reported by Cargo/rustc.
    pub column_start: u64,
    /// Ending source column reported by Cargo/rustc.
    pub column_end: u64,
    /// Whether this is a primary diagnostic location.
    pub is_primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Bounded compiler message with relative source spans and child diagnostics.
///
/// # Example
///
/// ```rust
/// let diagnostic = rappin::CompilerDiagnostic { code: Some("E0425".into()), level: "error".into(), message: "cannot find value in this scope".into(), spans: vec![], children: vec![] };
/// assert_eq!(diagnostic.code.as_deref(), Some("E0425"));
/// ```
pub struct CompilerDiagnostic {
    /// Compiler diagnostic code, such as E0425, when supplied.
    pub code: Option<String>,
    /// Compiler level such as error, warning, note or help.
    pub level: String,
    /// Bounded sanitized compiler message; source text and environment values are not intentionally retained.
    pub message: String,
    /// Bounded project-relative source locations.
    pub spans: Vec<CompilerSpan>,
    /// Bounded child diagnostics such as notes or help.
    pub children: Vec<CompilerDiagnostic>,
}

/// Candidate preparation/publication failed. Local routes were not replaced.
/// Compilation may execute build scripts/macros; this is not a sandbox receipt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let failure = rappin::HotReloadFailure { phase: rappin::ReloadPhase::Compilation, message: "candidate compilation failed".into(), diagnostics: vec![], truncated: false, exit: Some(rappin::ProcessExit { code: Some(101), signal: None }), timed_out: false, candidate_started: false, publication_attempted: false, routes_published: false, source: None };
/// assert!(!failure.routes_published);
/// ```
pub struct HotReloadFailure {
    /// Candidate preparation/publication phase that failed.
    pub phase: ReloadPhase,
    /// Bounded sanitized summary of the candidate failure.
    pub message: String,
    /// Bounded compiler diagnostic tree, if compilation produced diagnostics.
    pub diagnostics: Vec<CompilerDiagnostic>,
    /// At least one field/message was omitted or shortened to enforce bounds.
    pub truncated: bool,
    /// Observed candidate/build process exit status, if available.
    pub exit: Option<ProcessExit>,
    /// Whether the candidate operation exceeded its configured timeout.
    pub timed_out: bool,
    /// Whether the candidate executable was started before failure.
    pub candidate_started: bool,
    /// Whether remote slash-command synchronization was attempted.
    pub publication_attempted: bool,
    /// Local route publication; remote API outcomes belong to `source`.
    pub routes_published: bool,
    /// Original bounded structured cause, retaining available HTTP/retry/uncertain-write evidence.
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
