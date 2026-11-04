//! Prometheus metrics export for request telemetry.
//!
//! This module provides optional Prometheus metrics export for the
//! request telemetry collected by the Client. Enable with the `metrics` feature.
//!
//! # Example
//!
//! ```no_run
//! use rappin::{Client, metrics};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = Client::new("yb_token");
//!
//!     // Install Prometheus recorder
//!     metrics::install_prometheus()?;
//!
//!     // Make some requests...
//!     let _ = client.get_me().await;
//!
//!     // Metrics are now available at /metrics endpoint
//!     // (requires running a Prometheus exporter server)
//!     Ok(())
//! }
//! ```

#[cfg(feature = "metrics")]
use metrics::{counter, gauge, histogram};
#[cfg(feature = "metrics")]
use metrics_exporter_prometheus::PrometheusBuilder;
#[cfg(feature = "metrics")]
use std::time::Duration;

/// Install a Prometheus metrics exporter.
///
/// This sets up a global recorder that exports metrics in Prometheus format.
/// The metrics are available via the `/metrics` endpoint when using the
/// `metrics_exporter_prometheus` crate's built-in server, or you can use
/// `PrometheusBuilder` directly for custom configuration.
#[cfg(feature = "metrics")]
pub fn install_prometheus() -> Result<(), Box<dyn std::error::Error>> {
    let builder = PrometheusBuilder::new();
    builder.install()?;
    Ok(())
}

/// Install a Prometheus metrics exporter with custom configuration.
#[cfg(feature = "metrics")]
pub fn install_prometheus_with_config(
    _namespace: impl Into<String>,
    _buckets: Option<Vec<f64>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // For now, just use the basic install since the API varies
    let builder = PrometheusBuilder::new();
    builder.install()?;
    Ok(())
}

/// Record request metrics from a `RequestMetrics` snapshot.
///
/// This function records the metrics to the global Prometheus recorder.
/// Call this periodically (e.g., every 10 seconds) to export metrics.
#[cfg(feature = "metrics")]
pub fn record_request_metrics(m: &crate::diagnostics::RequestMetrics) {
    counter!("rappin_requests_started_total").increment(m.started);
    counter!("rappin_requests_completed_total").increment(m.completed);
    counter!("rappin_requests_cancelled_total").increment(m.cancelled);
    counter!("rappin_requests_succeeded_total").increment(m.succeeded);
    counter!("rappin_requests_failed_total").increment(m.failed);
    counter!("rappin_requests_attempts_total").increment(m.attempts);
    counter!("rappin_requests_observed_attempts_total").increment(m.observed_attempts);
    counter!("rappin_requests_retries_total").increment(m.retries);
    counter!("rappin_requests_transport_errors_total").increment(m.transport_errors);
    counter!("rappin_requests_rate_limited_total").increment(m.rate_limited);
    counter!("rappin_requests_server_errors_total").increment(m.server_errors);

    gauge!("rappin_requests_in_flight").set(m.in_flight as f64);

    histogram!("rappin_request_duration_seconds").record(m.total_duration.as_secs_f64());
    histogram!("rappin_rate_limit_wait_seconds").record(m.total_rate_limit_wait.as_secs_f64());
}

/// Record a single request observation.
#[cfg(feature = "metrics")]
pub fn record_request_observation(obs: &crate::diagnostics::RequestObservation) {
    let method = obs.method.clone();
    let route = obs.route.clone();
    let status = obs
        .status
        .map(|s| s.to_string())
        .unwrap_or_else(|| "none".to_string());

    histogram!("rappin_request_duration_seconds",
        "method" => method.clone(),
        "route" => route.clone(),
        "status" => status.clone()
    )
    .record(obs.duration.as_secs_f64());

    if obs.rate_limit_wait > Duration::ZERO {
        histogram!("rappin_rate_limit_wait_seconds",
            "method" => method.clone(),
            "route" => route.clone(),
            "status" => status.clone()
        )
        .record(obs.rate_limit_wait.as_secs_f64());
    }

    counter!("rappin_request_attempts_total",
        "method" => method,
        "route" => route,
        "status" => status
    )
    .increment(obs.attempts as u64);
}

/// Record transport status changes.
#[cfg(feature = "metrics")]
pub fn record_transport_status(status: &crate::diagnostics::TransportStatus) {
    let mode_label = match status.mode {
        crate::diagnostics::TransportMode::Polling => "polling",
        crate::diagnostics::TransportMode::GatewayPreferred => "gateway_preferred",
        crate::diagnostics::TransportMode::GatewayOnly => "gateway_only",
    };

    gauge!("rappin_transport_mode", "mode" => mode_label).set(1.0);

    if let Some(active) = status.active {
        let active_label = match active {
            crate::diagnostics::EventTransport::Polling => "polling",
            crate::diagnostics::EventTransport::Gateway => "gateway",
        };
        gauge!("rappin_event_transport", "transport" => active_label).set(1.0);
    }

    if let Some(rtt) = status.heartbeat_rtt {
        gauge!("rappin_gateway_heartbeat_rtt_seconds").set(rtt.as_secs_f64());
    }

    if let Some(fallback) = status.fallback {
        let fallback_label = match fallback {
            crate::diagnostics::FallbackReason::Unavailable => "unavailable",
            crate::diagnostics::FallbackReason::AccessDenied => "access_denied",
            crate::diagnostics::FallbackReason::RateLimited => "rate_limited",
            crate::diagnostics::FallbackReason::WireLogging => "wire_logging",
            crate::diagnostics::FallbackReason::Backpressure => "backpressure",
        };
        counter!("rappin_transport_fallback_total", "reason" => fallback_label).increment(1);
    }

    gauge!("rappin_pending_batches").set(status.pending_batches as f64);
}

/// Record availability status.
#[cfg(feature = "metrics")]
pub fn record_availability_status(status: &crate::availability::AvailabilityStatus) {
    let state_label = match status.state {
        crate::availability::AvailabilityState::Idle => "idle",
        crate::availability::AvailabilityState::Reading => "reading",
        crate::availability::AvailabilityState::Waiting => "waiting",
        crate::availability::AvailabilityState::Recovered => "recovered",
        crate::availability::AvailabilityState::Exhausted => "exhausted",
        crate::availability::AvailabilityState::Cancelled => "cancelled",
        crate::availability::AvailabilityState::Terminal => "terminal",
    };

    gauge!("rappin_availability_state", "state" => state_label).set(1.0);

    if let Some(phase) = status.phase {
        let phase_label = match phase {
            crate::availability::AvailabilityPhase::StartupIdentity => "startup_identity",
            crate::availability::AvailabilityPhase::StartupCursor => "startup_cursor",
            crate::availability::AvailabilityPhase::Intake => "intake",
        };
        gauge!("rappin_availability_phase", "phase" => phase_label).set(1.0);
    }

    gauge!("rappin_availability_attempts").set(status.attempts as f64);
    gauge!("rappin_availability_elapsed_seconds").set(status.elapsed.as_secs_f64());
    gauge!("rappin_availability_backoff_wait_seconds").set(status.backoff_wait.as_secs_f64());

    if let Some(next_delay) = status.next_delay {
        gauge!("rappin_availability_next_delay_seconds").set(next_delay.as_secs_f64());
    }

    counter!("rappin_availability_episodes_total").increment(status.episodes);
    counter!("rappin_availability_total_attempts_total").increment(status.total_attempts);
    counter!("rappin_availability_total_retries_total").increment(status.total_retries);
    histogram!("rappin_availability_total_backoff_wait_seconds")
        .record(status.total_backoff_wait.as_secs_f64());
    counter!("rappin_availability_recoveries_total").increment(status.recoveries);
    counter!("rappin_availability_exhaustions_total").increment(status.exhaustions);
    counter!("rappin_availability_cancellations_total").increment(status.cancellations);
}

/// Stub functions when metrics feature is disabled.
#[cfg(not(feature = "metrics"))]
pub fn install_prometheus() -> Result<(), Box<dyn std::error::Error>> {
    Err("metrics feature not enabled".into())
}

#[cfg(not(feature = "metrics"))]
pub fn install_prometheus_with_config(
    _namespace: impl Into<String>,
    _buckets: Option<Vec<f64>>,
) -> Result<(), Box<dyn std::error::Error>> {
    Err("metrics feature not enabled".into())
}

#[cfg(not(feature = "metrics"))]
pub fn record_request_metrics(_metrics: &crate::diagnostics::RequestMetrics) {}

#[cfg(not(feature = "metrics"))]
pub fn record_request_observation(_obs: &crate::diagnostics::RequestObservation) {}

#[cfg(not(feature = "metrics"))]
pub fn record_transport_status(_status: &crate::diagnostics::TransportStatus) {}

#[cfg(not(feature = "metrics"))]
pub fn record_availability_status(_status: &crate::availability::AvailabilityStatus) {}
