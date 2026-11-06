//! Prometheus metrics export for request telemetry.
//!
//! This module provides optional Prometheus metrics export for the
//! request telemetry collected by the Client. Enable with the `metrics` feature.
//!
//! # Example
//!
//! ```no_run
//! use rappin::{Client, metrics};
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
//!     metrics::record_request_metrics(&client.request_metrics());
//!
//!     // Metrics are now available at /metrics endpoint
//!     // (requires running a Prometheus exporter server)
//!     Ok(())
//! }
//! ```

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::time::Duration;

/// Install a Prometheus metrics exporter.
///
/// This sets up a global recorder that exports metrics in Prometheus format.
/// The metrics are available via the `/metrics` endpoint when using the
/// `metrics_exporter_prometheus` crate's built-in server, or you can use
/// `PrometheusBuilder` directly for custom configuration.
pub fn install_prometheus() -> Result<(), Box<dyn std::error::Error>> {
    let builder = PrometheusBuilder::new();
    builder.install()?;
    Ok(())
}

/// Install with an optional `namespace` global label and histogram bucket bounds.
/// An empty namespace adds no label. Buckets are seconds, finite, nonnegative,
/// strictly increasing and nonempty when supplied. Existing metric names remain.
pub fn install_prometheus_with_config(
    namespace: impl Into<String>,
    buckets: Option<Vec<f64>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let builder = configured_builder(namespace.into(), buckets)?;
    builder.install()?;
    Ok(())
}

fn configured_builder(
    namespace: String,
    buckets: Option<Vec<f64>>,
) -> Result<PrometheusBuilder, Box<dyn std::error::Error>> {
    let mut builder = PrometheusBuilder::new();
    if !namespace.is_empty() {
        builder = builder.add_global_label("namespace", namespace);
    }
    if let Some(buckets) = buckets {
        if buckets.is_empty()
            || buckets.iter().any(|v| !v.is_finite() || *v < 0.0)
            || buckets.windows(2).any(|v| v[0] >= v[1])
        {
            return Err(
                "histogram buckets must be finite, nonnegative and strictly increasing".into(),
            );
        }
        builder = builder.set_buckets(&buckets)?;
    }
    Ok(builder)
}

/// Record request metrics from a `RequestMetrics` snapshot.
///
/// Publish one cumulative client/aggregate series periodically. Counters use
/// absolute values, so repeated snapshots do not count traffic twice. Independent
/// clients must be aggregated before publishing; values are not per-call deltas.
/// Cumulative durations are gauges ending in `_seconds_total`, not distributions.
/// Use final request observations separately for latency histograms.
pub fn record_request_metrics(m: &crate::diagnostics::RequestMetrics) {
    counter!("rappin_requests_started_total").absolute(m.started);
    counter!("rappin_requests_completed_total").absolute(m.completed);
    counter!("rappin_requests_cancelled_total").absolute(m.cancelled);
    counter!("rappin_requests_succeeded_total").absolute(m.succeeded);
    counter!("rappin_requests_failed_total").absolute(m.failed);
    counter!("rappin_requests_attempts_total").absolute(m.attempts);
    counter!("rappin_requests_observed_attempts_total").absolute(m.observed_attempts);
    counter!("rappin_requests_retries_total").absolute(m.retries);
    counter!("rappin_requests_transport_errors_total").absolute(m.transport_errors);
    counter!("rappin_requests_rate_limited_total").absolute(m.rate_limited);
    counter!("rappin_requests_server_errors_total").absolute(m.server_errors);

    gauge!("rappin_requests_in_flight").set(m.in_flight as f64);

    gauge!("rappin_request_duration_seconds_total").set(m.total_duration.as_secs_f64());
    gauge!("rappin_rate_limit_wait_seconds_total").set(m.total_rate_limit_wait.as_secs_f64());
}

/// Record the final observation of one request once. `attempts`, elapsed time
/// and rate-limit wait are cumulative for that call. Do not repeatedly export
/// `last_request()` or pass every intermediate retry observation to this helper;
/// use RequestEvent::attempt_duration for individual-attempt latency analysis.
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

/// Publish current transport state. Repeated snapshots do not create transition counts.
pub fn record_transport_status(status: &crate::diagnostics::TransportStatus) {
    for (mode, label) in [
        (crate::diagnostics::TransportMode::Polling, "polling"),
        (
            crate::diagnostics::TransportMode::GatewayPreferred,
            "gateway_preferred",
        ),
        (
            crate::diagnostics::TransportMode::GatewayOnly,
            "gateway_only",
        ),
    ] {
        gauge!("rappin_transport_mode","mode"=>label).set(if status.mode == mode {
            1.0
        } else {
            0.0
        });
    }
    for (active, label) in [
        (crate::diagnostics::EventTransport::Polling, "polling"),
        (crate::diagnostics::EventTransport::Gateway, "gateway"),
    ] {
        gauge!("rappin_event_transport","transport"=>label).set(if status.active == Some(active) {
            1.0
        } else {
            0.0
        });
    }
    gauge!("rappin_gateway_heartbeat_rtt_seconds")
        .set(status.heartbeat_rtt.unwrap_or_default().as_secs_f64());
    for (reason, label) in [
        (
            crate::diagnostics::FallbackReason::Unavailable,
            "unavailable",
        ),
        (
            crate::diagnostics::FallbackReason::AccessDenied,
            "access_denied",
        ),
        (
            crate::diagnostics::FallbackReason::RateLimited,
            "rate_limited",
        ),
        (
            crate::diagnostics::FallbackReason::WireLogging,
            "wire_logging",
        ),
        (
            crate::diagnostics::FallbackReason::Backpressure,
            "backpressure",
        ),
    ] {
        gauge!("rappin_transport_fallback","reason"=>label).set(
            if status.fallback == Some(reason) {
                1.0
            } else {
                0.0
            },
        );
    }

    gauge!("rappin_pending_batches").set(status.pending_batches as f64);
}

/// Publish cumulative availability totals and current state for one aggregate series.
pub fn record_availability_status(status: &crate::availability::AvailabilityStatus) {
    for (state, label) in [
        (crate::availability::AvailabilityState::Idle, "idle"),
        (crate::availability::AvailabilityState::Reading, "reading"),
        (crate::availability::AvailabilityState::Waiting, "waiting"),
        (
            crate::availability::AvailabilityState::Recovered,
            "recovered",
        ),
        (
            crate::availability::AvailabilityState::Exhausted,
            "exhausted",
        ),
        (
            crate::availability::AvailabilityState::Cancelled,
            "cancelled",
        ),
        (crate::availability::AvailabilityState::Terminal, "terminal"),
    ] {
        gauge!("rappin_availability_state","state"=>label).set(if status.state == state {
            1.0
        } else {
            0.0
        });
    }
    for (phase, label) in [
        (
            crate::availability::AvailabilityPhase::StartupIdentity,
            "startup_identity",
        ),
        (
            crate::availability::AvailabilityPhase::StartupCursor,
            "startup_cursor",
        ),
        (crate::availability::AvailabilityPhase::Intake, "intake"),
    ] {
        gauge!("rappin_availability_phase","phase"=>label).set(if status.phase == Some(phase) {
            1.0
        } else {
            0.0
        });
    }

    gauge!("rappin_availability_attempts").set(status.attempts as f64);
    gauge!("rappin_availability_elapsed_seconds").set(status.elapsed.as_secs_f64());
    gauge!("rappin_availability_backoff_wait_seconds").set(status.backoff_wait.as_secs_f64());

    gauge!("rappin_availability_next_delay_seconds")
        .set(status.next_delay.unwrap_or_default().as_secs_f64());

    counter!("rappin_availability_episodes_total").absolute(status.episodes);
    counter!("rappin_availability_total_attempts_total").absolute(status.total_attempts);
    counter!("rappin_availability_total_retries_total").absolute(status.total_retries);
    gauge!("rappin_availability_total_backoff_wait_seconds")
        .set(status.total_backoff_wait.as_secs_f64());
    counter!("rappin_availability_recoveries_total").absolute(status.recoveries);
    counter!("rappin_availability_exhaustions_total").absolute(status.exhaustions);
    counter!("rappin_availability_cancellations_total").absolute(status.cancellations);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(rendered: &str, name: &str) -> f64 {
        rendered
            .lines()
            .find_map(|line| {
                line.strip_prefix(name)
                    .and_then(|value| value.strip_prefix(' '))
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or_else(|| panic!("missing {name} in {rendered}"))
    }
    #[test]
    fn repeated_snapshots_preserve_totals_and_clear_old_state() {
        let recorder = PrometheusBuilder::new().build_recorder();
        metrics::with_local_recorder(&recorder, || {
            let mut requests = crate::diagnostics::RequestMetrics {
                started: 3,
                completed: 2,
                attempts: 4,
                total_duration: Duration::from_secs(8),
                ..Default::default()
            };
            record_request_metrics(&requests);
            record_request_metrics(&requests);
            requests.started = 4;
            record_request_metrics(&requests);
            let mut transport = crate::diagnostics::TransportStatus {
                mode: crate::TransportMode::GatewayPreferred,
                active: Some(crate::EventTransport::Gateway),
                heartbeat_rtt: Some(Duration::from_secs(1)),
                fallback: Some(crate::FallbackReason::Unavailable),
                ..Default::default()
            };
            record_transport_status(&transport);
            record_transport_status(&transport);
            transport.mode = crate::TransportMode::Polling;
            transport.active = None;
            transport.heartbeat_rtt = None;
            transport.fallback = None;
            record_transport_status(&transport);
            let mut availability = crate::AvailabilityStatus {
                state: crate::AvailabilityState::Waiting,
                phase: Some(crate::AvailabilityPhase::Intake),
                episodes: 2,
                total_attempts: 5,
                next_delay: Some(Duration::from_secs(2)),
                ..Default::default()
            };
            record_availability_status(&availability);
            record_availability_status(&availability);
            availability.state = crate::AvailabilityState::Idle;
            availability.phase = None;
            availability.next_delay = None;
            record_availability_status(&availability);
        });
        let rendered = recorder.handle().render();
        assert_eq!(sample(&rendered, "rappin_requests_started_total"), 4.0);
        assert_eq!(sample(&rendered, "rappin_requests_attempts_total"), 4.0);
        assert_eq!(
            sample(&rendered, "rappin_request_duration_seconds_total"),
            8.0
        );
        assert_eq!(sample(&rendered, "rappin_availability_episodes_total"), 2.0);
        assert_eq!(
            sample(&rendered, "rappin_availability_total_attempts_total"),
            5.0
        );
        for name in [
            "rappin_transport_mode{mode=\"gateway_preferred\"}",
            "rappin_event_transport{transport=\"gateway\"}",
            "rappin_event_transport{transport=\"polling\"}",
            "rappin_gateway_heartbeat_rtt_seconds",
            "rappin_transport_fallback{reason=\"unavailable\"}",
            "rappin_availability_state{state=\"waiting\"}",
            "rappin_availability_phase{phase=\"intake\"}",
            "rappin_availability_next_delay_seconds",
        ] {
            assert_eq!(sample(&rendered, name), 0.0);
        }
        assert_eq!(
            sample(&rendered, "rappin_transport_mode{mode=\"polling\"}"),
            1.0
        );
        assert_eq!(
            sample(&rendered, "rappin_availability_state{state=\"idle\"}"),
            1.0
        );
        assert!(!rendered.contains("rappin_transport_fallback_total"));
        assert!(!rendered.contains("rappin_request_duration_seconds_count"));
    }
    #[test]
    fn configuration_applies_namespace_and_buckets_without_starting_a_server() {
        let recorder = configured_builder("fixture".into(), Some(vec![0.1, 0.5, 1.0]))
            .unwrap()
            .build_recorder();
        metrics::with_local_recorder(&recorder, || {
            histogram!("fixture_latency_seconds").record(0.25);
        });
        let rendered = recorder.handle().render();
        assert!(rendered.contains("namespace=\"fixture\""));
        assert!(rendered.contains("le=\"0.5\""));
        for invalid in [
            vec![],
            vec![f64::NAN],
            vec![f64::INFINITY],
            vec![-1.0],
            vec![1.0, 0.5],
            vec![1.0, 1.0],
        ] {
            assert!(configured_builder(String::new(), Some(invalid)).is_err());
        }
    }
}
