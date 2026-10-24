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

/// A specialized Result type for Yapin' operations.
pub type Result<T> = std::result::Result<T, YappinError>;

