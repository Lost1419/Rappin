//! # Example
//!
//! ```rust
//! let metrics = rappin::RequestMetrics::default();
//! assert_eq!(metrics.started, metrics.completed + metrics.cancelled + metrics.in_flight);
//! ```
//! Typed event-transport status. REST latency and gateway heartbeat RTT are separate measurements.
use std::time::Duration;
use tokio::sync::watch;

/// Exact client-wide totals. Shared by Client clones and independent of how
/// quickly observers consume notifications. Durations include cancelled calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// # Example
///
/// ```rust
/// let metrics = rappin::RequestMetrics::default();
/// assert_eq!(metrics.started, metrics.completed + metrics.cancelled + metrics.in_flight);
/// ```
pub struct RequestMetrics {
    /// Client-wide logical requests started.
    pub started: u64,
    /// Requests that returned a terminal success or error.
    pub completed: u64,
    /// Requests dropped before returning a terminal result.
    pub cancelled: u64,
    /// Logical requests currently executing or waiting.
    pub in_flight: u64,
    /// Completed logical requests classified as successful.
    pub succeeded: u64,
    /// Completed logical requests classified as failed.
    pub failed: u64,
    /// Transport attempts started, including retries.
    pub attempts: u64,
    /// Attempts for which a response/error observation was recorded.
    pub observed_attempts: u64,
    /// Attempts started after an initial attempt in the same logical request.
    pub retries: u64,
    /// Observed attempts without an HTTP-equivalent status.
    pub transport_errors: u64,
    /// Observed attempts returning status 429.
    pub rate_limited: u64,
    /// Observed attempts returning a 500-through-599 status.
    pub server_errors: u64,
    /// Cumulative request execution duration, including cancelled requests and internal waiting.
    pub total_duration: Duration,
    /// Cumulative shared/server-advised rate-limit waiting, including cancellation during a wait.
    pub total_rate_limit_wait: Duration,
    /// Latest attempt-event sequence issued by this client.
    pub last_sequence: u64,
}

/// An attempt notification. Sequence gaps or broadcast::RecvError::Lagged
/// indicate lost notifications; RequestMetrics totals are unaffected.
#[derive(Debug, Clone)]
/// # Example
///
/// ```rust
/// use rappin::{RequestEvent, RequestObservation};
/// let event = RequestEvent { sequence: 1, request_id: 1, attempt_duration: std::time::Duration::ZERO, observation: RequestObservation::default() };
/// assert_eq!(event.sequence, 1);
/// ```
pub struct RequestEvent {
    /// Monotonic attempt-notification sequence; gaps indicate lost observations.
    pub sequence: u64,
    /// Client-local logical request ID shared by its attempts.
    pub request_id: u64,
    /// Execution duration of this individual attempt.
    pub attempt_duration: Duration,
    /// Sanitized attempt observation without request bodies or credentials.
    pub observation: RequestObservation,
}

pub(crate) struct RequestTelemetry {
    metrics: std::sync::Mutex<RequestMetrics>,
    events: tokio::sync::broadcast::Sender<RequestEvent>,
}
impl Default for RequestTelemetry {
    fn default() -> Self {
        Self {
            metrics: Default::default(),
            events: tokio::sync::broadcast::channel(256).0,
        }
    }
}
impl RequestTelemetry {
    fn lock(&self) -> std::sync::MutexGuard<'_, RequestMetrics> {
        self.metrics.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub(crate) fn snapshot(&self) -> RequestMetrics {
        self.lock().clone()
    }
    pub(crate) fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RequestEvent> {
        self.events.subscribe()
    }
    pub(crate) fn start(&self) -> RequestGuard<'_> {
        let mut totals = self.lock();
        totals.started += 1;
        totals.in_flight += 1;
        RequestGuard {
            telemetry: self,
            id: totals.started,
            started: std::time::Instant::now(),
            done: false,
        }
    }
}
pub(crate) struct RequestGuard<'a> {
    telemetry: &'a RequestTelemetry,
    id: u64,
    started: std::time::Instant,
    done: bool,
}
impl RequestGuard<'_> {
    pub(crate) fn waiting(&self) -> RequestWait<'_> {
        self.waiting_for(Duration::MAX)
    }
    pub(crate) fn waiting_for(&self, limit: Duration) -> RequestWait<'_> {
        RequestWait {
            limit,
            telemetry: self.telemetry,
            started: std::time::Instant::now(),
            done: false,
        }
    }
    pub(crate) fn attempt(&self, retry: bool) {
        let mut totals = self.telemetry.lock();
        totals.attempts += 1;
        if retry {
            totals.retries += 1;
        }
    }
    pub(crate) fn observe(&self, observation: &RequestObservation, attempt_duration: Duration) {
        let mut totals = self.telemetry.lock();
        totals.observed_attempts += 1;
        totals.last_sequence += 1;
        match observation.status {
            None => totals.transport_errors += 1,
            Some(429) => totals.rate_limited += 1,
            Some(500..=599) => totals.server_errors += 1,
            _ => (),
        }
        // Send under the same lock so sequence order matches delivery order.
        if self.telemetry.events.receiver_count() > 0 {
            let _ = self.telemetry.events.send(RequestEvent {
                sequence: totals.last_sequence,
                request_id: self.id,
                attempt_duration,
                observation: observation.clone(),
            });
        }
    }
    pub(crate) fn finish(mut self, success: bool) {
        let mut totals = self.telemetry.lock();
        totals.completed += 1;
        totals.in_flight -= 1;
        totals.total_duration += self.started.elapsed();
        if success {
            totals.succeeded += 1;
        } else {
            totals.failed += 1;
        }
        self.done = true;
    }
}
pub(crate) struct RequestWait<'a> {
    limit: Duration,
    telemetry: &'a RequestTelemetry,
    started: std::time::Instant,
    done: bool,
}
impl RequestWait<'_> {
    pub(crate) fn finish(mut self, duration: Duration) {
        self.telemetry.lock().total_rate_limit_wait += duration.min(self.limit);
        self.done = true;
    }
}
impl Drop for RequestWait<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.telemetry.lock().total_rate_limit_wait += self.started.elapsed().min(self.limit);
        }
    }
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let mut totals = self.telemetry.lock();
            totals.cancelled += 1;
            totals.in_flight -= 1;
            totals.total_duration += self.started.elapsed();
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Requested event-intake transport policy. It does not select the optional gateway JSON request transport.
///
/// # Example
///
/// ```rust
/// assert_eq!(rappin::TransportMode::default(), rappin::TransportMode::Polling);
/// ```
pub enum TransportMode {
    #[default]
    /// Use long polling for event intake; the default.
    Polling,
    /// Prefer WebSocket events and allow documented polling fallback.
    GatewayPreferred,
    /// Require WebSocket intake; polling fallback is disabled.
    GatewayOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Transport currently used for event delivery.
///
/// # Example
///
/// ```rust
/// let active = rappin::EventTransport::Gateway;
/// assert_ne!(active, rappin::EventTransport::Polling);
/// ```
pub enum EventTransport {
    /// Events are currently being read through the HTTP polling endpoint.
    Polling,
    /// Events are currently being read over WebSocket.
    Gateway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Reason the runner is using fallback event delivery.
///
/// # Example
///
/// ```rust
/// let reason = rappin::FallbackReason::Backpressure;
/// assert!(matches!(reason, rappin::FallbackReason::Backpressure));
/// ```
pub enum FallbackReason {
    /// Gateway connection or transport is unavailable.
    Unavailable,
    /// Gateway access was refused by the platform.
    AccessDenied,
    /// Gateway access is currently rate-limited.
    RateLimited,
    /// Raw dependency wire tracing made credential submission unsafe.
    WireLogging,
    /// Gateway delivery was paused because bounded intake capacity filled.
    Backpressure,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Latest event-intake state and cursor positions. A received cursor is not proof of handler completion.
///
/// # Example
///
/// ```rust
/// let status = rappin::TransportStatus::default();
/// assert_eq!(status.mode, rappin::TransportMode::Polling);
/// assert!(status.active.is_none());
/// ```
pub struct TransportStatus {
    /// Configured event transport mode.
    pub mode: TransportMode,
    /// Currently active intake transport, if connected.
    pub active: Option<EventTransport>,
    /// Last measured WebSocket heartbeat round-trip duration; absent outside a connected gateway.
    pub heartbeat_rtt: Option<Duration>,
    /// Current reason for polling fallback, if any.
    pub fallback: Option<FallbackReason>,
    /// Highest received intake cursor; it may be ahead of completed work.
    pub received_cursor: u64,
    /// Last successfully committed handled prefix; unfinished work keeps it behind the received cursor.
    pub committed_cursor: u64,
    /// Active batches awaiting ordered completion; excludes the separate intake queue.
    pub pending_batches: usize,
}

/// REST execution through response headers, including retries and advised waits; not UI latency.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let observation = rappin::RequestObservation::default();
/// assert!(observation.status.is_none());
/// assert_eq!(observation.attempts, 0);
/// ```
pub struct RequestObservation {
    /// HTTP method of the observed attempt.
    pub method: String,
    /// Sanitized route shape with resource IDs, query strings and callback credentials removed.
    pub route: String,
    /// HTTP-equivalent response status; absent for transport errors.
    pub status: Option<u16>,
    /// Elapsed logical execution through response headers, including retries and advised waits; excludes final body decoding.
    pub duration: Duration,
    /// Shared/server-advised rate-limit waiting accumulated for this request.
    pub rate_limit_wait: Duration,
    /// Number of attempts observed for this logical request so far.
    pub attempts: u32,
    /// Version response header, when supplied by the API.
    pub api_version: Option<String>,
    /// Build response header, when supplied by the API.
    pub api_build: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Telemetry(watch::Sender<TransportStatus>);
impl Telemetry {
    pub(crate) fn new(mode: TransportMode) -> Self {
        Self(
            watch::channel(TransportStatus {
                mode,
                ..Default::default()
            })
            .0,
        )
    }
    pub(crate) fn update(&self, f: impl FnOnce(&mut TransportStatus)) {
        self.0.send_modify(f);
    }
    pub(crate) fn status(&self) -> TransportStatus {
        self.0.borrow().clone()
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<TransportStatus> {
        self.0.subscribe()
    }
}
