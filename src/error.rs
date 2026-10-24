use thiserror::Error;

/// The error type for operations in the `yappin` crate.
#[derive(Debug, Error)]
pub enum YappinError {
    /// An API error returned by Yapin' servers.
    #[error("API error ({status}): {message}")]
    Api {
        status: u16,
        message: String,
        retry_after: Option<f64>,
    },

    /// An HTTP client or transport error.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

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

    /// A generic I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl YappinError {
    /// Returns the HTTP status code associated with this error, if available.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            Self::Http(e) => e.status().map(|s| s.as_u16()),
            Self::Unauthorized => Some(401),
            Self::RateLimited { .. } => Some(429),
            _ => None,
        }
    }

    /// Checks if this error represents a rate limit (HTTP 429).
    pub fn is_rate_limit(&self) -> bool {
        matches!(self, Self::RateLimited { .. }) || matches!(self, Self::Api { status: 429, .. })
    }

    /// Returns the suggested retry wait duration if this error was caused by a rate limit.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimited { retry_after } => {
                Some(std::time::Duration::from_secs_f64(*retry_after))
            }
            Self::Api {
                retry_after: Some(secs),
                ..
            } => Some(std::time::Duration::from_secs_f64(*secs)),
            _ => None,
        }
    }

    /// Checks if this error is an authorization failure (HTTP 401).
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized) || self.status_code() == Some(401)
    }

    /// Checks if this error is a permissions failure (HTTP 403 Forbidden).
    pub fn is_forbidden(&self) -> bool {
        self.status_code() == Some(403)
    }

    /// Checks if this error represents a resource not found (HTTP 404).
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
