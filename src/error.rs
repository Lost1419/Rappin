//! Structured API, command, intake and candidate failures with explicit mutation outcomes.
use thiserror::Error;

/// Serializable error crossing a worker boundary. Details retain retry/outcome
/// hints and partial-write receipts; parent transports redact credentials first.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::Error;
/// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
/// assert_eq!(error.status_code(), Some(429));
/// assert!(error.is_rate_limit());
/// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
/// let remote = error.to_remote();
/// assert_eq!(remote.to_json()["status"], 429);
/// // A retry hint does not authorize replaying a handler or uncertain mutation.
/// ```
pub struct RemoteError {
    /// Stable structured error classification.
    pub kind: String,
    /// Human-readable cause; sanitize application-supplied details before external disclosure.
    pub message: String,
    /// Original HTTP status when available.
    pub status: Option<u16>,
    /// Original nonnegative retry advice in seconds, when available.
    pub retry_after: Option<f64>,
    #[serde(flatten)]
    /// Additional structured cause, outcome, partial receipts and diagnostics preserved through IPC.
    pub details: std::collections::BTreeMap<String, serde_json::Value>,
}
impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RemoteError {}
impl RemoteError {
    /// Recover a bounded report after a worker boundary or failure callback.
    /// # Example
    ///
    /// ```rust
    /// use rappin::RemoteError;
    /// fn inspect(error: &RemoteError) {
    ///     if let Some(report) = error.hot_reload_failure() { println!("{report:?}"); }
    /// }
    /// ```
    pub fn hot_reload_failure(&self) -> Option<crate::HotReloadFailure> {
        if let Some(report) = self.details.get("hot_reload") {
            serde_json::from_value(report.clone()).ok()
        } else if matches!(
            self.kind.as_str(),
            "failure_handling" | "partial_write" | "handler_interrupted" | "command_invocation"
        ) {
            let source: Self = serde_json::from_value(self.details.get("source")?.clone()).ok()?;
            source.hot_reload_failure()
        } else {
            None
        }
    }
    /// Recover the structured availability report when preserved in supported remote wrappers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::RemoteError;
    /// fn inspect(error: &RemoteError) {
    ///     if let Some(report) = error.availability_failure() { println!("{report:?}"); }
    /// }
    /// ```
    pub fn availability_failure(&self) -> Option<crate::AvailabilityFailure> {
        if let Some(value) = self.details.get("availability") {
            serde_json::from_value(value.clone()).ok()
        } else if self.kind == "command_invocation" {
            let source: Self = serde_json::from_value(self.details.get("source")?.clone()).ok()?;
            source.availability_failure()
        } else {
            None
        }
    }
    /// Encode original classification/status/retry and extra structured fields.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::Value::Object(self.details.clone().into_iter().collect());
        value["kind"] = self.kind.clone().into();
        value["message"] = self.message.clone().into();
        value["status"] = serde_json::json!(self.status);
        value["retry_after"] = serde_json::json!(self.retry_after);
        value
    }
}
impl serde::Serialize for RemoteError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&self.to_json(), serializer)
    }
}

/// The error type for operations in the `yappin` crate.
#[derive(Debug, Error)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::Error;
/// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
/// assert_eq!(error.status_code(), Some(429));
/// assert!(error.is_rate_limit());
/// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
/// let remote = error.to_remote();
/// assert_eq!(remote.to_json()["status"], 429);
/// // A retry hint does not authorize replaying a handler or uncertain mutation.
/// ```
pub enum Error {
    #[error("conflicting event payload for event {event_id}", event_id = .conflict.event_id)]
    /// Conflicting payloads were delivered for the same event identity; original receipts remain retained.
    IngressConflict {
        /// Bounded conflicting-source/schema evidence, without rewriting the original receipt.
        conflict: Box<crate::IngressConflict>,
    },
    /// Retains consumer metadata around an incomplete checked invocation without
    /// changing the original structured cause or enabling automatic replies.
    #[error("{source}")]
    IdentifiedInvocation {
        /// Consumer operation key; it supplies no authority or remote idempotency.
        operation_id: crate::OperationId,
        #[source]
        /// Original incomplete invocation cause.
        source: Box<Error>,
    },
    /// An explicitly invoked command did not complete. The outer handler owns
    /// reconciliation; this wrapper never triggers typed-rejection auto-replies.
    #[error("command invocation {command} did not complete ({status:?})")]
    Invocation {
        /// Explicit installed command that did not complete.
        command: String,
        /// Not-found, denied or failed disposition.
        status: crate::bot::InvocationStatus,
        /// Whether arbitrary gate/check/handler code may have produced effects.
        effects_possible: bool,
        #[source]
        /// Original structured cause when available.
        source: Option<Box<Error>>,
    },
    #[error("retry wait budget exhausted after {waited:?} (budget {budget:?}): {source}")]
    /// The server-advised/shared rate-limit wait budget was exhausted.
    RetryWaitExceeded {
        /// Total server-advised/shared delay charged to this operation.
        waited: std::time::Duration,
        /// Configured server-advised/shared wait allowance.
        budget: std::time::Duration,
        #[source]
        /// Original cause/status/retry hint that exhausted the allowance.
        source: Box<Error>,
    },
    #[error("{failure}")]
    /// Candidate build/startup/protocol/registration/publication failed; inspect its bounded report.
    HotReload {
        #[source]
        /// Bounded candidate phase, diagnostics and publication state.
        failure: Box<crate::HotReloadFailure>,
    },
    #[error("event processing stopped after cursor {cursor}",cursor=.report.committed_cursor)]
    /// Intake stopped with incomplete events or checkpoint persistence failure.
    EventProcessing {
        /// Ordered checkpoint, failed, unprocessed and completed-but-uncommitted evidence.
        report: Box<crate::bot::EventProcessingFailure>,
    },
    #[error(
        "failure callback/reply did not complete: {handling_error}; original failure: {source}"
    )]
    /// The failure callback or error reply failed as well as the original operation.
    FailureHandling {
        /// Original invocation failure.
        source: Box<Error>,
        /// Additional failure while replying or running the failure callback.
        handling_error: Box<Error>,
    },
    #[error("handler completion is uncertain: {reason}{cause}",cause=.source.as_ref().map(|e|format!(": {e}")).unwrap_or_default())]
    /// Handler completion became uncertain, including cancellation or panic.
    HandlerInterrupted {
        /// Reason completion could not be established.
        reason: String,
        #[source]
        /// Original failure, if one was captured before interruption.
        source: Option<Box<Error>>,
    },
    #[error("operation authorization is denied or incomplete: {operation}",operation=.report.operation)]
    /// Required scoped access was Denied or Unknown.
    OperationAuthorization {
        /// Independent requester/bot and resource-scope checks.
        report: Box<crate::capabilities::AuthorizationReport>,
    },
    #[error("{resource} search is incomplete after {pages} pages ({scanned} scanned, {matches} candidates); use an ID or a larger search budget")]
    /// The lookup budget ended before uniqueness/completeness could be established.
    IncompleteSearch {
        /// Resource family whose search is incomplete.
        resource: String,
        /// Number of records examined.
        scanned: usize,
        /// Number of API pages examined.
        pages: usize,
        /// Continuation cursor when the API supplied one.
        next_cursor: Option<uuid::Uuid>,
        /// Number of matching candidates found before the search stopped.
        matches: usize,
    },
    #[error(transparent)]
    /// Original structured error returned by an isolated worker.
    Remote(
        #[from]
        #[doc = "Structured worker cause."]
        RemoteError,
    ),
    /// A multi-step write stopped after zero or more confirmed successes.
    #[error("write failed at step {failed_step}: {source}")]
    PartialWrite {
        /// Confirmed successful write receipts; do not replay these steps.
        completed: Vec<serde_json::Value>,
        /// One-based step at which the workflow stopped.
        failed_step: usize,
        #[source]
        /// Original failed-step cause, including any unknown mutation outcome.
        source: Box<Error>,
    },
    #[error(transparent)]
    /// Typed command input/policy failure; invalid definitions remain unexpected.
    Command(
        #[from]
        #[doc = "Typed command rejection or definition cause."]
        crate::commands::CommandError,
    ),
    #[error(transparent)]
    /// Collector timeout, overflow, closure or invalid subscription options.
    Wait(
        #[from]
        #[doc = "Collector failure classification."]
        crate::collectors::WaitError,
    ),
    /// An API error returned by Yapin' servers.
    #[error("API error ({status}): {message}")]
    Api {
        /// HTTP response status.
        status: u16,
        /// API error explanation.
        message: String,
        /// Server retry advice in seconds; not permission to replay uncertain work.
        retry_after: Option<f64>,
    },

    /// An HTTP client or transport error.
    #[error("HTTP error: {0}")]
    Http(
        #[source]
        #[doc = "Original HTTP transport error with URL removed by normal conversion."]
        reqwest::Error,
    ),

    /// A JSON serialization or deserialization error.
    #[error("JSON error: {0}")]
    Json(
        #[from]
        #[doc = "Original JSON encoding/decoding cause."]
        serde_json::Error,
    ),

    /// Rate limit reached with retry-after header.
    #[error("Rate limited, retry after {retry_after} seconds")]
    RateLimited {
        #[doc = "Server-advised delay in seconds."]
        retry_after: f64,
    },

    /// The provided bot token is invalid or unauthorized.
    #[error("Invalid bot token: unauthorized (401)")]
    Unauthorized,

    /// Missing or invalid configuration.
    #[error("Configuration error: {0}")]
    Config(#[doc = "Configuration failure explanation."] String),

    /// Invalid request payload, detected before making an HTTP request.
    #[error("Invalid {field}: {message}")]
    Validation {
        #[doc = "Field or definition path rejected before dispatch."]
        field: String,
        #[doc = "Local validation explanation."]
        message: String,
    },

    /// A generic I/O error.
    #[error("I/O error: {0}")]
    Io(
        #[from]
        #[doc = "Original filesystem or I/O cause."]
        std::io::Error,
    ),
}

impl Error {
    /// Recover bounded ingress diagnostics from native, remote, or pipeline errors.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// fn inspect(error: &Error) {
    ///     if let Some(report) = error.ingress_conflict() { println!("{report:?}"); }
    /// }
    /// ```
    pub fn ingress_conflict(&self) -> Option<crate::IngressConflict> {
        fn find(value: &serde_json::Value, remaining: usize) -> Option<crate::IngressConflict> {
            if remaining == 0 {
                return None;
            }
            if let Some(value) = value.get("conflict") {
                if let Ok(report) = serde_json::from_value::<crate::IngressConflict>(value.clone())
                {
                    return Some(report);
                }
            }
            for key in ["source", "handling_error", "report", "intake_error"] {
                if let Some(found) = value.get(key).and_then(|v| find(v, remaining - 1)) {
                    return Some(found);
                }
            }
            None
        }
        find(&self.to_json(), 16)
    }
    /// Recover bounded availability diagnostics from supported native/remote wrappers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// fn inspect(error: &Error) {
    ///     if let Some(report) = error.availability_failure() { println!("{report:?}"); }
    /// }
    /// ```
    pub fn availability_failure(&self) -> Option<crate::AvailabilityFailure> {
        match self {
            Self::IdentifiedInvocation { source, .. } => source.availability_failure(),
            Self::Invocation { source, .. } => source.as_ref()?.availability_failure(),
            Self::Remote(error) => error.availability_failure(),
            Self::EventProcessing { report } => report.primary()?.availability_failure(),
            Self::FailureHandling { source, .. } | Self::PartialWrite { source, .. } => {
                source.availability_failure()
            }
            _ => None,
        }
    }
    /// Returns the HTTP status code associated with this error, if available.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::IdentifiedInvocation { source, .. } => source.status_code(),
            Self::Invocation { source, .. } => source.as_ref().and_then(|e| e.status_code()),
            Self::RetryWaitExceeded { source, .. } => source.status_code(),
            Self::HotReload { failure } => failure.source.as_ref().and_then(|e| e.status),
            Self::EventProcessing { report } => report.primary().and_then(|e| e.status),
            Self::FailureHandling {
                source,
                handling_error,
            } => source
                .status_code()
                .or_else(|| handling_error.status_code()),
            Self::HandlerInterrupted { source, .. } => {
                source.as_ref().and_then(|e| e.status_code())
            }
            Self::OperationAuthorization { report }
                if report.decision() == crate::capabilities::AccessDecision::Denied =>
            {
                Some(403)
            }
            Self::Remote(error) => error.status,
            Self::PartialWrite { source, .. } => source.status_code(),
            Self::Api { status, .. } => Some(*status),
            Self::Http(e) => e.status().map(|s| s.as_u16()),
            Self::Unauthorized => Some(401),
            Self::RateLimited { .. } => Some(429),
            _ => None,
        }
    }

    /// Checks if this error represents a rate limit (HTTP 429).
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::error::Error;
    ///
    /// let err = Error::RateLimited { retry_after: 2.5 };
    /// assert!(err.is_rate_limit());
    /// ```
    pub fn is_rate_limit(&self) -> bool {
        if let Self::PartialWrite { source, .. } = self {
            return source.is_rate_limit();
        }
        self.status_code() == Some(429)
    }

    /// Returns the suggested retry wait duration if this error was caused by a rate limit.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::IdentifiedInvocation { source, .. } => return source.retry_after(),
            Self::Invocation { source, .. } => {
                return source.as_ref().and_then(|e| e.retry_after())
            }
            Self::RetryWaitExceeded { source, .. } => return source.retry_after(),
            Self::HotReload { failure } => {
                return failure
                    .source
                    .as_ref()
                    .and_then(|e| e.retry_after)
                    .and_then(|s| std::time::Duration::try_from_secs_f64(s).ok())
            }
            Self::EventProcessing { report } => {
                return report
                    .primary()
                    .and_then(|e| e.retry_after)
                    .and_then(|s| std::time::Duration::try_from_secs_f64(s).ok())
            }
            Self::FailureHandling {
                source,
                handling_error,
            } => {
                return source
                    .retry_after()
                    .or_else(|| handling_error.retry_after())
            }
            Self::HandlerInterrupted { source, .. } => {
                return source.as_ref().and_then(|e| e.retry_after())
            }
            _ => (),
        }
        if let Self::PartialWrite { source, .. } = self {
            return source.retry_after();
        }
        let seconds = match self {
            Self::Remote(error) => error.retry_after,
            Self::RateLimited { retry_after } => Some(*retry_after),
            Self::Api { retry_after, .. } => *retry_after,
            _ => None,
        }?;
        std::time::Duration::try_from_secs_f64(seconds).ok()
    }

    /// Construct a structured local validation failure before making an HTTP request.
    ///
    /// # Example
    ///
    /// ```rust
    /// let error = rappin::Error::validation("content", "must not be empty");
    /// assert_eq!(error.to_json()["field"], "content");
    /// ```
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation {
            field: field.into(),
            message: message.into(),
        }
    }

    /// Stable JSON representation for agents. Transport URLs are intentionally omitted.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::IngressConflict { conflict } => {
                return serde_json::json!({"kind":"ingress_conflict","field":"ingress.payload","message":self.to_string(),"conflict":conflict,"outcome":"incomplete","retry_safety":"do_not_replay_workflow"});
            }
            Self::IdentifiedInvocation {
                operation_id,
                source,
            } => {
                let mut value = source.to_json();
                value["operation_id"] = serde_json::json!(operation_id);
                return value;
            }
            Self::Invocation {
                command,
                status,
                effects_possible,
                source,
            } => {
                return serde_json::json!({"kind":"command_invocation", "message":self.to_string(), "status":self.status_code(), "retry_after":self.retry_after().map(|d|d.as_secs_f64()), "command":command, "invocation_status":status, "effects_possible":effects_possible, "source":source.as_ref().map(|e|e.to_json()), "outcome":if *effects_possible { "unknown" } else { "not_applied" }, "retry_safety":"do_not_replay_workflow"});
            }
            Self::RetryWaitExceeded {
                waited,
                budget,
                source,
            } => {
                let cause = source.to_json();
                return serde_json::json!({"kind":"retry_wait_exceeded","message":self.to_string(),"status":self.status_code(),"retry_after":self.retry_after().map(|d|d.as_secs_f64()),"waited_seconds":waited.as_secs_f64(),"budget_seconds":budget.as_secs_f64(),"source":cause,"outcome":cause["outcome"],"retry_safety":"do_not_replay_workflow"});
            }

            Self::HotReload { failure } => {
                return serde_json::json!({"kind":"hot_reload","message":self.to_string(),"status":self.status_code(),"retry_after":self.retry_after().map(|d|d.as_secs_f64()),"hot_reload":failure,"outcome":"incomplete","retry_safety":"do_not_replay_workflow"})
            }
            Self::EventProcessing { report } => {
                return serde_json::json!({"kind":"event_processing","message":self.to_string(),"status":self.status_code(),"retry_after":self.retry_after().map(|d|d.as_secs_f64()),"report":report,"retry_safety":"do_not_replay_workflow","outcome":"incomplete"})
            }
            Self::FailureHandling {
                source,
                handling_error,
            } => {
                return serde_json::json!({"kind":"failure_handling","message":self.to_string(),"status":self.status_code(),"retry_after":self.retry_after().map(|d|d.as_secs_f64()),"source":source.to_json(),"handling_error":handling_error.to_json(),"retry_safety":"do_not_replay_workflow","outcome":"incomplete"})
            }
            Self::HandlerInterrupted { source, .. } => {
                return serde_json::json!({"kind":"handler_interrupted","message":self.to_string(),"status":self.status_code(),"retry_after":self.retry_after().map(|d|d.as_secs_f64()),"source":source.as_ref().map(|e|e.to_json()),"retry_safety":"do_not_replay_workflow","outcome":"unknown"})
            }
            _ => (),
        }
        if let Self::OperationAuthorization { report } = self {
            return serde_json::json!({"kind":"operation_authorization","message":self.to_string(),"status":self.status_code(),"retry_after":null,"decision":report.decision(),"authorization":report,"retry_safety":"fix_request","outcome":"not_applied"});
        }
        if let Self::Remote(error) = self {
            return error.to_json();
        }
        if let Self::PartialWrite {
            completed,
            failed_step,
            source,
        } = self
        {
            let mut value = source.to_json();
            value["cause_kind"] = value["kind"].clone();
            value["kind"] = "partial_write".into();
            value["completed_steps"] = serde_json::json!(completed);
            value["failed_step"] = (*failed_step).into();
            value["retry_safety"] = "do_not_replay_workflow".into();
            return value;
        }
        let kind = match self {
            Self::IncompleteSearch { .. } => "incomplete_search",
            Self::Command(_) => "command",
            Self::Wait(_) => "event_wait",
            Self::Validation { .. } => "validation",
            Self::Config(_) => "configuration",
            Self::Unauthorized => "unauthorized",
            Self::Http(_) => "transport",
            Self::Json(_) => "json",
            Self::Io(_) => "io",
            _ => "api",
        };
        let message = if matches!(self, Self::Http(_)) {
            "HTTP transport failed".into()
        } else {
            self.to_string()
        };
        let mut value = serde_json::json!({"kind": kind, "message": message,
            "status": self.status_code(), "retry_after": self.retry_after().map(|d| d.as_secs_f64())});
        if let Self::Validation { field, .. } = self {
            value["field"] = field.clone().into();
        }
        if let Self::Command(error) = self {
            value["command_kind"] = error.kind().into();
        }
        if let Self::IncompleteSearch {
            resource,
            scanned,
            pages,
            next_cursor,
            matches,
        } = self
        {
            value["resource"] = resource.clone().into();
            value["scanned"] = (*scanned).into();
            value["pages"] = (*pages).into();
            value["next_cursor"] = serde_json::json!(next_cursor);
            value["matches"] = (*matches).into();
            value["complete"] = false.into();
        }
        let advised = self.retry_after().is_some() && matches!(self.status_code(), Some(429 | 503));
        value["retry_safety"] = (if advised {
            "server_advised"
        } else if matches!(self, Self::Http(_) | Self::Json(_))
            || self.status_code().is_some_and(|s| s >= 500)
        {
            "unknown_write_outcome"
        } else {
            "fix_request"
        })
        .into();
        value["outcome"] = (if advised {
            "not_applied"
        } else if matches!(self, Self::Http(_) | Self::Json(_))
            || self.status_code().is_some_and(|s| s >= 500)
        {
            "unknown"
        } else {
            "rejected"
        })
        .into();
        value
    }

    /// Convert to the serializable structured cause; caller-provided details still need application disclosure policy.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn to_remote(&self) -> RemoteError {
        serde_json::from_value(self.to_json()).expect("error JSON contains required remote fields")
    }

    /// Recover a candidate report from native or structured worker/callback errors.
    /// Does not classify a failed candidate as a completed command.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// fn inspect(error: &Error) {
    ///     if let Some(report) = error.hot_reload_failure() { println!("{report:?}"); }
    /// }
    /// ```
    pub fn hot_reload_failure(&self) -> Option<crate::HotReloadFailure> {
        match self {
            Self::IdentifiedInvocation { source, .. } => source.hot_reload_failure(),
            Self::Invocation { source, .. } => source.as_ref()?.hot_reload_failure(),
            Self::HotReload { failure } => Some((**failure).clone()),
            Self::Remote(error) => error.hot_reload_failure(),
            Self::FailureHandling { source, .. } | Self::PartialWrite { source, .. } => {
                source.hot_reload_failure()
            }
            Self::HandlerInterrupted { source, .. } => source.as_ref()?.hot_reload_failure(),
            _ => None,
        }
    }

    /// Checks if this error is an authorization failure (HTTP 401).
    /// # Example
    ///
    /// ```rust
    /// let error = rappin::Error::Api { status: 401, message: "Example".into(), retry_after: None };
    /// assert!(error.is_unauthorized());
    /// ```
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized) || self.status_code() == Some(401)
    }
    /// Whether this is a direct typed command error or a structured remote command error.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn is_command_error(&self) -> bool {
        matches!(self, Self::Command(_))
            || matches!(self, Self::Remote(error) if error.kind=="command")
    }
    /// Only recognized typed input/check failures, never definition errors or
    /// legacy remote strings whose classification was lost.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Error;
    /// let error = Error::Api { status: 429, message: "Rate limited".into(), retry_after: Some(2.0) };
    /// assert_eq!(error.status_code(), Some(429));
    /// assert!(error.is_rate_limit());
    /// assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    /// let remote = error.to_remote();
    /// assert_eq!(remote.to_json()["status"], 429);
    /// // A retry hint does not authorize replaying a handler or uncertain mutation.
    /// ```
    pub fn is_command_rejection(&self) -> bool {
        matches!(self, Self::Command(error) if error.is_rejection())
            || matches!(self, Self::Remote(error) if error.kind=="command" && matches!(error.details.get("command_kind").and_then(|v|v.as_str()),Some("quoting"|"missing"|"invalid"|"extra"|"denied")))
    }

    /// Checks if this error is a permissions failure (HTTP 403 Forbidden).
    /// # Example
    ///
    /// ```rust
    /// let error = rappin::Error::Api { status: 403, message: "Example".into(), retry_after: None };
    /// assert!(error.is_forbidden());
    /// ```
    pub fn is_forbidden(&self) -> bool {
        self.status_code() == Some(403)
    }

    /// Checks if this error represents a resource not found (HTTP 404).
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::error::Error;
    ///
    /// let err = Error::Api {
    ///     status: 404,
    ///     message: "Channel not found".to_string(),
    ///     retry_after: None,
    /// };
    /// assert!(err.is_not_found());
    /// ```
    pub fn is_not_found(&self) -> bool {
        self.status_code() == Some(404)
    }

    /// Checks if this error represents a server-side error (HTTP 5xx).
    /// # Example
    ///
    /// ```rust
    /// let error = rappin::Error::Api { status: 503, message: "Example".into(), retry_after: None };
    /// assert!(error.is_server_error());
    /// ```
    pub fn is_server_error(&self) -> bool {
        self.status_code()
            .is_some_and(|code| (500..=599).contains(&code))
    }
}

/// A specialized Result type for Yapin' operations.
pub type Result<T> = std::result::Result<T, Error>;

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}
