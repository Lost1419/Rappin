use thiserror::Error;

/// Serializable error crossing a worker boundary. Details retain retry/outcome
/// hints and partial-write receipts; parent transports redact credentials first.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct RemoteError {
    pub kind: String,
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<f64>,
    #[serde(flatten)]
    pub details: std::collections::BTreeMap<String, serde_json::Value>,
}
impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RemoteError {}
impl RemoteError {
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
pub enum YappinError {
    #[error("{resource} search is incomplete after {pages} pages ({scanned} scanned, {matches} candidates); use an ID or a larger search budget")]
    IncompleteSearch {
        resource: String,
        scanned: usize,
        pages: usize,
        next_cursor: Option<uuid::Uuid>,
        matches: usize,
    },
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// A multi-step write stopped after zero or more confirmed successes.
    #[error("write failed at step {failed_step}: {source}")]
    PartialWrite {
        completed: Vec<serde_json::Value>,
        failed_step: usize,
        #[source]
        source: Box<YappinError>,
    },
    #[error(transparent)]
    Command(#[from] crate::commands::CommandError),
    #[error(transparent)]
    Wait(#[from] crate::collectors::WaitError),
    /// An API error returned by Yapin' servers.
    #[error("API error ({status}): {message}")]
    Api {
        status: u16,
        message: String,
        retry_after: Option<f64>,
    },

    /// An HTTP client or transport error.
    #[error("HTTP error: {0}")]
    Http(#[source] reqwest::Error),

    /// A JSON serialization or deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Rate limit reached with retry-after header.
    #[error("Rate limited, retry after {retry_after} seconds")]
    RateLimited { retry_after: f64 },

    /// The provided bot token is invalid or unauthorized.
    #[error("Invalid bot token: unauthorized (401)")]
    Unauthorized,

    /// Missing or invalid configuration.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Invalid request payload, detected before making an HTTP request.
    #[error("Invalid {field}: {message}")]
    Validation { field: String, message: String },

    /// A generic I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl YappinError {
    /// Returns the HTTP status code associated with this error, if available.
    pub fn status_code(&self) -> Option<u16> {
        match self {
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
    /// use rappin::error::YappinError;
    ///
    /// let err = YappinError::RateLimited { retry_after: 2.5 };
    /// assert!(err.is_rate_limit());
    /// ```
    pub fn is_rate_limit(&self) -> bool {
        if let Self::PartialWrite { source, .. } = self {
            return source.is_rate_limit();
        }
        self.status_code() == Some(429)
    }

    /// Returns the suggested retry wait duration if this error was caused by a rate limit.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
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

    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation {
            field: field.into(),
            message: message.into(),
        }
    }

    /// Stable JSON representation for agents. Transport URLs are intentionally omitted.
    pub fn to_json(&self) -> serde_json::Value {
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

    pub fn to_remote(&self) -> RemoteError {
        serde_json::from_value(self.to_json()).expect("error JSON contains required remote fields")
    }

    /// Checks if this error is an authorization failure (HTTP 401).
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized) || self.status_code() == Some(401)
    }
    pub fn is_command_error(&self) -> bool {
        matches!(self, Self::Command(_))
            || matches!(self, Self::Remote(error) if error.kind=="command")
    }

    /// Checks if this error is a permissions failure (HTTP 403 Forbidden).
    pub fn is_forbidden(&self) -> bool {
        self.status_code() == Some(403)
    }

    /// Checks if this error represents a resource not found (HTTP 404).
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::error::YappinError;
    ///
    /// let err = YappinError::Api {
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
    pub fn is_server_error(&self) -> bool {
        self.status_code()
            .is_some_and(|code| (500..=599).contains(&code))
    }
}

/// A specialized Result type for Yapin' operations.
pub type Result<T> = std::result::Result<T, YappinError>;

impl From<reqwest::Error> for YappinError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}
