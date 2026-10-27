//! Typed event-transport status. REST latency and gateway heartbeat RTT are separate measurements.
use std::time::Duration;
use tokio::sync::watch;

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
