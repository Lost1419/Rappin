//! Grouped advanced bot settings. Applications own persistence and cache backends.
use std::{sync::Arc, time::Duration};

/// Event transport, outage recovery and HTTP rate-limit configuration.
///
/// ```rust
/// let transport = rappin::bot::TransportConfig::polling();
/// assert_eq!(transport.mode, rappin::TransportMode::Polling);
/// ```
#[derive(Clone)]
#[non_exhaustive]
pub struct TransportConfig {
    /// Polling, preferred WebSocket delivery or gateway-only delivery.
    pub mode: crate::TransportMode,
    /// Custom WebSocket endpoint; omission selects the SDK default.
    pub gateway_url: Option<String>,
    /// HTTPS API base, or an explicit local test endpoint.
    pub api_base: String,
    /// Optional bounded policy for read-only startup and intake outages.
    pub availability: Option<crate::AvailabilityPolicy>,
    /// Shared/server-advised rate-limit waiting budget, separate from outages.
    pub max_retry_wait: Duration,
    /// Known-host HTTP fallback. Custom origins stay confined to their origin.
    pub api_fallback: bool,
}
impl Default for TransportConfig {
    fn default() -> Self {
        Self::polling()
    }
}
impl TransportConfig {
    /// Use long polling with the default HTTP endpoint.
    ///
    /// ```rust
    /// let transport = rappin::bot::TransportConfig::polling();
    /// assert!(transport.gateway_url.is_none());
    /// ```
    pub fn polling() -> Self {
        Self {
            mode: crate::TransportMode::Polling,
            gateway_url: None,
            api_base: crate::DEFAULT_API_BASE.into(),
            availability: None,
            max_retry_wait: Duration::from_secs(30),
            api_fallback: true,
        }
    }
    /// Prefer the SDK's default WebSocket endpoint with polling fallback.
    /// Building requires the `gateway` Cargo feature.
    ///
    /// ```rust
    /// let transport = rappin::bot::TransportConfig::gateway();
    /// assert_eq!(transport.mode, rappin::TransportMode::GatewayPreferred);
    /// ```
    pub fn gateway() -> Self {
        Self {
            mode: crate::TransportMode::GatewayPreferred,
            ..Self::polling()
        }
    }
}

/// Scheduling, response acknowledgement and draining limits.
///
/// ```rust
/// let mut config = rappin::bot::ExecutionConfig::default();
/// config.concurrency = 1;
/// ```
#[derive(Clone)]
#[non_exhaustive]
pub struct ExecutionConfig {
    /// Concurrent events, 1 through 100. One preserves handler order.
    pub concurrency: usize,
    /// Prepared/active batch bound, 1 through 32.
    pub pending_batches: usize,
    /// Graceful drain deadline. None waits for accepted work to finish.
    pub drain_timeout: Option<Duration>,
    /// Optional server event age filter in seconds.
    pub max_event_age: Option<u32>,
    /// Optional early interaction acknowledgement delay.
    pub auto_ack_after: Option<Duration>,
}
impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            concurrency: 8,
            pending_batches: 4,
            drain_timeout: None,
            max_event_age: None,
            auto_ack_after: None,
        }
    }
}

/// Restart checkpoints and optional shared-ingress storage.
///
/// ```rust
/// let persistence = rappin::bot::PersistenceConfig::file("./state/events.json");
/// assert!(persistence.cursor.is_some());
/// ```
#[derive(Default)]
#[non_exhaustive]
pub struct PersistenceConfig {
    /// Application-selected checkpoint. One runner owns each cursor.
    pub cursor: Option<crate::EventCursor>,
    /// Shared polling/webhook/gateway admission configuration.
    pub ingress: Option<crate::IngressConfig>,
    /// Application-owned durable receipts. Supplying this enables shared ingress.
    pub store: Option<Arc<dyn crate::IngressStore>>,
}
impl PersistenceConfig {
    /// Use an explicit local checkpoint path.
    ///
    /// ```rust
    /// let persistence = rappin::bot::PersistenceConfig::file("./state/events.json");
    /// assert!(persistence.cursor.is_some());
    /// ```
    pub fn file(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            cursor: Some(crate::EventCursor::new(path)),
            ..Self::default()
        }
    }
}

/// Opt-in lookup caching and an optional application-owned backend.
///
/// ```rust
/// let mut config = rappin::bot::CacheConfig::default();
/// config.policy = rappin::CachePolicy::new(128, std::time::Duration::from_secs(30));
/// ```
#[derive(Default)]
#[non_exhaustive]
pub struct CacheConfig {
    /// Bounded in-memory lookup policy. Authorization freshness stays explicit.
    pub policy: crate::CachePolicy,
    /// Custom asynchronous cache or database backend.
    pub backend: Option<Arc<dyn crate::LookupCache>>,
}

/// Advanced settings configured before building a bot.
///
/// ```rust
/// let bot = rappin::Bot::builder("unused-token")
///     .configure(|settings| settings.execution.concurrency = 1)
///     .build()?;
/// # Ok::<(), rappin::Error>(())
/// ```
#[derive(Default)]
#[non_exhaustive]
pub struct BotConfig {
    /// Event and HTTP transport settings.
    pub transport: TransportConfig,
    /// Event scheduling and draining limits.
    pub execution: ExecutionConfig,
    /// Application-selected checkpoints and durable receipt storage.
    pub persistence: PersistenceConfig,
    /// Lookup caching and application-owned backend.
    pub cache: CacheConfig,
}
impl BotConfig {
    pub(crate) fn validate(&self) -> crate::Result<()> {
        if !(1..=100).contains(&self.execution.concurrency) {
            return Err(crate::Error::validation(
                "execution.concurrency",
                "must be 1..=100",
            ));
        }
        if !(1..=32).contains(&self.execution.pending_batches) {
            return Err(crate::Error::validation(
                "execution.pending_batches",
                "must be 1..=32",
            ));
        }
        if self
            .execution
            .drain_timeout
            .is_some_and(|d| d.is_zero() || d > Duration::from_secs(3600))
        {
            return Err(crate::Error::validation(
                "execution.drain_timeout",
                "must be greater than zero and at most one hour",
            ));
        }
        let api = url::Url::parse(&self.transport.api_base).map_err(|_| {
            crate::Error::validation("transport.api_base", "requires an absolute HTTP API URL")
        })?;
        if !matches!(api.scheme(), "http" | "https")
            || api.host_str().is_none()
            || !api.username().is_empty()
            || api.password().is_some()
            || api.query().is_some()
            || api.fragment().is_some()
        {
            return Err(crate::Error::validation(
                "transport.api_base",
                "requires an HTTP origin/path without credentials, query or fragment",
            ));
        }
        if let Some(gateway) = &self.transport.gateway_url {
            let url = url::Url::parse(gateway).map_err(|_| {
                crate::Error::validation("transport.gateway_url", "requires a WebSocket URL")
            })?;
            if !matches!(url.scheme(), "ws" | "wss")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(crate::Error::validation(
                    "transport.gateway_url",
                    "requires a WebSocket URL without credentials or fragment",
                ));
            }
        }
        if (self.persistence.ingress.is_some() || self.persistence.store.is_some())
            && self.transport.mode == crate::TransportMode::GatewayOnly
        {
            return Err(crate::Error::Config(
                "shared ingress requires polling recovery; select Polling or GatewayPreferred"
                    .into(),
            ));
        }
        Ok(())
    }
}
