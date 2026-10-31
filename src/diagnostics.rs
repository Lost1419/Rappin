//! Typed event-transport status. REST latency and gateway heartbeat RTT are separate measurements.
use std::time::Duration;
use tokio::sync::watch;

/// Exact client-wide totals. Shared by Client clones and independent of how
/// quickly observers consume notifications. Durations include cancelled calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestMetrics {
    pub started: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub in_flight: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub attempts: u64,
    pub observed_attempts: u64,
    pub retries: u64,
    pub transport_errors: u64,
    pub rate_limited: u64,
    pub server_errors: u64,
    pub total_duration: Duration,
    pub total_rate_limit_wait: Duration,
    pub last_sequence: u64,
}

/// An attempt notification. Sequence gaps or broadcast::RecvError::Lagged
/// indicate lost notifications; RequestMetrics totals are unaffected.
#[derive(Debug, Clone)]
pub struct RequestEvent {
    pub sequence: u64,
    pub request_id: u64,
    pub attempt_duration: Duration,
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
        RequestWait {
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
    pub(crate) fn observe(&self, observation: RequestObservation, attempt_duration: Duration) {
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
        let _ = self.telemetry.events.send(RequestEvent {
            sequence: totals.last_sequence,
            request_id: self.id,
            attempt_duration,
            observation,
        });
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
    telemetry: &'a RequestTelemetry,
    started: std::time::Instant,
    done: bool,
}
impl RequestWait<'_> {
    pub(crate) fn finish(mut self, duration: Duration) {
        self.telemetry.lock().total_rate_limit_wait += duration;
        self.done = true;
    }
}
impl Drop for RequestWait<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.telemetry.lock().total_rate_limit_wait += self.started.elapsed();
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
pub enum TransportMode {
    #[default]
    Polling,
    GatewayPreferred,
    GatewayOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTransport {
    Polling,
    Gateway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    Unavailable,
    AccessDenied,
    RateLimited,
    WireLogging,
    Backpressure,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportStatus {
    pub mode: TransportMode,
    pub active: Option<EventTransport>,
    pub heartbeat_rtt: Option<Duration>,
    pub fallback: Option<FallbackReason>,
    pub received_cursor: u64,
    pub committed_cursor: u64,
    /// Active batches awaiting ordered completion; excludes the separate intake queue.
    pub pending_batches: usize,
}

/// REST execution through response headers, including retries and advised waits; not UI latency.
#[derive(Debug, Clone)]
pub struct RequestObservation {
    pub method: String,
    pub route: String,
    pub status: Option<u16>,
    pub duration: Duration,
    pub rate_limit_wait: Duration,
    pub attempts: u32,
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
