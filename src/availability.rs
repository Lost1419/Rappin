//! # Example
//!
//! ```rust
//! use rappin::AvailabilityPolicy;
//! use std::time::Duration;
//! let policy = AvailabilityPolicy::new(5, Duration::from_secs(120))?
//!     .backoff(Duration::from_millis(500), Duration::from_secs(10))?
//!     .jitter_percent(20)?;
//! # Ok::<(), rappin::YappinError>(())
//! ```
//! Opt-in availability budgets for startup, event intake and client reads.
use crate::{RemoteError, Result, YappinError};
use serde::{Deserialize, Serialize};
use std::{future::Future, time::Duration};
use tokio::{sync::watch, time::Instant};

#[derive(Clone, Debug)]
#[non_exhaustive]
/// Opt-in retry budget for read-only startup, event intake and client requests. It does not retry handlers or mutations and is separate from the client's server-advised wait budget.
///
/// # Example
///
/// ```rust
/// use rappin::AvailabilityPolicy;
/// use std::time::Duration;
/// let policy = AvailabilityPolicy::new(5, Duration::from_secs(120))?
///     .backoff(Duration::from_millis(500), Duration::from_secs(10))?
///     .jitter_percent(20)?;
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub struct AvailabilityPolicy {
    max_attempts: u32,
    max_elapsed: Duration,
    initial_backoff: Duration,
    max_backoff: Duration,
    jitter_percent: u8,
}
impl Default for AvailabilityPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            max_elapsed: Duration::from_secs(120),
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(10),
            jitter_percent: 20,
        }
    }
}
impl AvailabilityPolicy {
    /// Includes the first attempt. Elapsed time includes requests, shared waits
    /// and backoff; each successful read starts a new availability episode.
    /// # Example
    ///
    /// ```rust
    /// use rappin::AvailabilityPolicy;
    /// use std::time::Duration;
    /// let policy = AvailabilityPolicy::new(5, Duration::from_secs(120))?
    ///     .backoff(Duration::from_millis(500), Duration::from_secs(10))?
    ///     .jitter_percent(20)?;
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn new(max_attempts: u32, max_elapsed: Duration) -> Result<Self> {
        if !(1..=1000).contains(&max_attempts)
            || max_elapsed.is_zero()
            || Instant::now().checked_add(max_elapsed).is_none()
        {
            return Err(YappinError::validation(
                "availability",
                "requires 1..=1000 attempts and a positive representable elapsed budget",
            ));
        }
        Ok(Self {
            max_attempts,
            max_elapsed,
            ..Self::default()
        })
    }
    /// Set positive exponential-backoff bounds. The maximum must be at least the initial delay; invalid or unrepresentable durations return a validation error.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::AvailabilityPolicy;
    /// use std::time::Duration;
    /// let policy = AvailabilityPolicy::new(5, Duration::from_secs(120))?
    ///     .backoff(Duration::from_millis(500), Duration::from_secs(10))?
    ///     .jitter_percent(20)?;
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn backoff(mut self, initial: Duration, maximum: Duration) -> Result<Self> {
        if initial.is_zero() || maximum < initial || Instant::now().checked_add(maximum).is_none() {
            return Err(YappinError::validation(
                "availability.backoff",
                "requires positive initial <= maximum with representable duration",
            ));
        }
        self.initial_backoff = initial;
        self.max_backoff = maximum;
        Ok(self)
    }
    /// Downward jitter in [base * (1 - percent/100), base], bounded by max_backoff.
    /// # Example
    ///
    /// ```rust
    /// let policy = rappin::AvailabilityPolicy::default().jitter_percent(0).unwrap();
    /// assert!(rappin::AvailabilityPolicy::default().jitter_percent(101).is_err());
    /// ```
    pub fn jitter_percent(mut self, percent: u8) -> Result<Self> {
        if percent > 100 {
            return Err(YappinError::validation(
                "availability.jitter",
                "maximum 100 percent",
            ));
        }
        self.jitter_percent = percent;
        Ok(self)
    }
    fn delay(&self, failure: u32, error: &YappinError) -> Duration {
        let base = self
            .initial_backoff
            .saturating_mul(1u32 << failure.saturating_sub(1).min(31))
            .min(self.max_backoff);
        let span = base.mul_f64(f64::from(self.jitter_percent) / 100.0);
        let jitter = if span.is_zero() {
            Duration::ZERO
        } else {
            let nanos = uuid::Uuid::new_v4().as_u128() % (span.as_nanos() + 1);
            Duration::new(
                (nanos / 1_000_000_000) as u64,
                (nanos % 1_000_000_000) as u32,
            )
        };
        base.saturating_sub(jitter)
            .max(error.retry_after().unwrap_or_default())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// Identifies the startup, intake or client read being recovered.
///
/// # Example
///
/// ```rust
/// assert_eq!(serde_json::to_value(rappin::AvailabilityPhase::StartupIdentity).unwrap(), "startup_identity");
/// ```
pub enum AvailabilityPhase {
    /// Initial bot identity read from GET /me.
    StartupIdentity,
    /// Initial event probe used when establishing a starting cursor.
    StartupCursor,
    /// Subsequent polling or gateway intake reads.
    Intake,
    /// A client or handler GET/HEAD request, including worker proxy reads.
    Request,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// Lifecycle state of the current availability episode.
///
/// # Example
///
/// ```rust
/// assert_eq!(rappin::AvailabilityState::default(), rappin::AvailabilityState::Idle);
/// ```
pub enum AvailabilityState {
    #[default]
    /// No episode is active or has been reported.
    Idle,
    /// A read attempt is in progress.
    Reading,
    /// Recovery is waiting before the next permitted read attempt.
    Waiting,
    /// The episode completed with a successful read.
    Recovered,
    /// A configured recovery budget prevented further attempts.
    Exhausted,
    /// The episode was interrupted by cancellation or shutdown.
    Cancelled,
    /// The read failed with a cause that this availability policy does not retry.
    Terminal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// Reason an availability episode ended without a successful read.
///
/// # Example
///
/// ```rust
/// assert_eq!(serde_json::to_value(rappin::AvailabilityEnd::AttemptLimit).unwrap(), "attempt_limit");
/// ```
pub enum AvailabilityEnd {
    /// The maximum attempt count was reached.
    AttemptLimit,
    /// The overall episode deadline was reached.
    ElapsedLimit,
    /// The next required delay would exceed the episode deadline.
    BackoffExceedsDeadline,
    /// Recovery was cancelled before another successful read.
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
/// Serializable recovery failure retaining its budget, read phase and last structured cause. Missing last_error is not an invented HTTP success or status.
///
/// # Example
///
/// ```rust
/// let zero = serde_json::json!({"secs":0,"nanos":0});
/// let failure: rappin::AvailabilityFailure = serde_json::from_value(serde_json::json!({"phase":"intake","reason":"attempt_limit","attempts":5,"elapsed":zero,"backoff_wait":zero,"max_attempts":5,"max_elapsed":{"secs":120,"nanos":0},"last_error":null})).unwrap();
/// assert_eq!(failure.attempts, failure.max_attempts);
/// ```
pub struct AvailabilityFailure {
    /// Read phase that exhausted or cancelled its availability episode.
    pub phase: AvailabilityPhase,
    /// Budget or cancellation condition that ended recovery.
    pub reason: AvailabilityEnd,
    /// Attempts started in this episode, including the first attempt.
    pub attempts: u32,
    /// Wall-clock elapsed time, including request execution and waits.
    pub elapsed: Duration,
    /// Time spent in availability backoff, separate from server-advised rate-limit waiting.
    pub backoff_wait: Duration,
    /// Configured attempt limit for this episode.
    pub max_attempts: u32,
    /// Configured overall elapsed budget for this episode.
    pub max_elapsed: Duration,
    /// Last observed cause. An aborted in-flight read has no invented HTTP status.
    pub last_error: Option<RemoteError>,
}

/// Snapshot counters remain exact even when latest-only watch notifications coalesce.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let status = rappin::AvailabilityStatus::default();
/// assert_eq!(status.total_attempts, 0);
/// assert_eq!(status.state, rappin::AvailabilityState::Idle);
/// ```
pub struct AvailabilityStatus {
    /// Current or most recent availability read phase.
    pub phase: Option<AvailabilityPhase>,
    /// Current availability episode state.
    pub state: AvailabilityState,
    /// Attempts started in the current or most recent episode.
    pub attempts: u32,
    /// Attempt budget for the current or most recent episode.
    pub max_attempts: u32,
    /// Elapsed budget for the current or most recent episode.
    pub max_elapsed: Duration,
    /// Elapsed episode time measured at the last update.
    pub elapsed: Duration,
    /// Availability-backoff waiting measured in the current episode.
    pub backoff_wait: Duration,
    /// Next planned availability delay, if recovery is waiting.
    pub next_delay: Option<Duration>,
    /// Last structured read error, if one has been observed.
    pub last_error: Option<RemoteError>,
    /// Cumulative availability episodes started.
    pub episodes: u64,
    /// Cumulative attempts started across episodes.
    pub total_attempts: u64,
    /// Cumulative retries after initial attempts.
    pub total_retries: u64,
    /// Cumulative availability-backoff time across episodes.
    pub total_backoff_wait: Duration,
    /// Cumulative episodes that recovered after a transient failure.
    pub recoveries: u64,
    /// Cumulative episodes that exhausted their configured budget.
    pub exhaustions: u64,
    /// Cumulative episodes cancelled by shutdown or cancellation.
    pub cancellations: u64,
}
#[derive(Clone)]
pub(crate) struct Telemetry(watch::Sender<AvailabilityStatus>);
impl Default for Telemetry {
    fn default() -> Self {
        Self(watch::channel(AvailabilityStatus::default()).0)
    }
}
impl Telemetry {
    pub(crate) fn status(&self) -> AvailabilityStatus {
        self.0.borrow().clone()
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<AvailabilityStatus> {
        self.0.subscribe()
    }
    fn update(&self, change: impl FnOnce(&mut AvailabilityStatus)) {
        self.0.send_modify(change);
    }
}

pub(crate) fn retryable_read(error: &YappinError) -> bool {
    match error {
        YappinError::Http(e) => {
            !e.is_builder() && (e.is_connect() || e.is_timeout() || e.is_request() || e.is_body())
        }
        // Exhausting the client's shared/advised wait budget is terminal here.
        YappinError::RetryWaitExceeded { .. } => false,
        YappinError::Remote(e) if e.kind == "gateway_transport" => true,
        YappinError::Remote(e) if e.kind == "gateway_request" && e.status.is_none() => {
            matches!(
                e.details.get("reason").and_then(serde_json::Value::as_str),
                Some(
                    "capacity"
                        | "admission_closed"
                        | "queue_timeout"
                        | "response_timeout"
                        | "request_timeout"
                        | "peer_closed"
                        | "peer_disconnected"
                        | "connection_reset"
                        | "io_error"
                        | "heartbeat_timeout"
                        | "send_timeout"
                        | "response_channel_closed"
                        | "request_channel_closed"
                )
            )
        }
        _ => {
            matches!(
                error.status_code(),
                Some(
                    408 | 500
                        | 502
                        | 503
                        | 504
                        | 520
                        | 521
                        | 522
                        | 1000
                        | 1001
                        | 1006
                        | 1011
                        | 1012
                        | 1013
                )
            ) || (error.status_code() == Some(429) && error.retry_after().is_some())
        }
    }
}

pub(crate) struct Episode {
    policy: AvailabilityPolicy,
    phase: AvailabilityPhase,
    handle: Option<crate::BotHandle>,
    client: crate::Client,
    telemetry: Telemetry,
    started: Instant,
    deadline: Instant,
    attempts: u32,
    id: u64,
    waited: Duration,
    waiting: Option<Instant>,
    advised_wait: Duration,
    wait_budget: crate::client::ReadWaitBudget,
    last_error: Option<RemoteError>,
    finished: bool,
}
impl Episode {
    pub(crate) fn new(
        handle: crate::BotHandle,
        policy: AvailabilityPolicy,
        phase: AvailabilityPhase,
    ) -> Self {
        let telemetry = handle.inner.availability_telemetry.clone();
        Self::start(
            handle.client().clone(),
            Some(handle),
            telemetry,
            policy,
            phase,
        )
    }
    pub(crate) fn for_client(client: crate::Client, policy: AvailabilityPolicy) -> Self {
        let telemetry = client.read_availability_telemetry();
        Self::start(client, None, telemetry, policy, AvailabilityPhase::Request)
    }
    fn start(
        client: crate::Client,
        handle: Option<crate::BotHandle>,
        telemetry: Telemetry,
        policy: AvailabilityPolicy,
        phase: AvailabilityPhase,
    ) -> Self {
        let wait_budget = client.read_wait_budget();
        let started = Instant::now();
        let deadline = started.checked_add(policy.max_elapsed).unwrap_or(started);
        let mut id = 0;
        telemetry.update(|s| {
            s.phase = Some(phase);
            s.state = AvailabilityState::Reading;
            s.attempts = 0;
            s.max_attempts = policy.max_attempts;
            s.max_elapsed = policy.max_elapsed;
            s.elapsed = Duration::ZERO;
            s.backoff_wait = Duration::ZERO;
            s.next_delay = None;
            s.last_error = None;
            s.episodes += 1;
            id = s.episodes;
        });
        Self {
            policy,
            phase,
            handle,
            client,
            telemetry,
            started,
            deadline,
            attempts: 0,
            id,
            waited: Duration::ZERO,
            waiting: None,
            advised_wait: Duration::ZERO,
            wait_budget,
            last_error: None,
            finished: false,
        }
    }
    fn update(&self, change: impl FnOnce(&mut AvailabilityStatus)) {
        self.telemetry.update(|status| {
            if status.episodes == self.id {
                change(status);
            }
        });
    }
    async fn cancelled(handle: &Option<crate::BotHandle>) {
        let Some(handle) = handle else {
            return std::future::pending().await;
        };
        loop {
            let notified = handle.inner.stopped.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !handle
                .inner
                .running
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return;
            }
            notified.await;
        }
    }
    pub(crate) async fn attempt<T>(
        &mut self,
        operation: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        if self.handle.as_ref().is_some_and(|handle| {
            !handle
                .inner
                .running
                .load(std::sync::atomic::Ordering::SeqCst)
        }) {
            return Err(self.end(AvailabilityEnd::Cancelled));
        }
        if Instant::now() >= self.deadline {
            return Err(self.end(AvailabilityEnd::ElapsedLimit));
        }
        self.attempts += 1;
        self.update(|s| {
            s.state = AvailabilityState::Reading;
            s.attempts = self.attempts;
            s.elapsed = self.started.elapsed();
            s.next_delay = None;
        });
        self.telemetry.update(|s| {
            s.total_attempts += 1;
            if self.attempts > 1 {
                s.total_retries += 1;
            }
        });
        tokio::select! {
            biased;
            _ = Self::cancelled(&self.handle) => Err(self.end(AvailabilityEnd::Cancelled)),
            result = tokio::time::timeout_at(self.deadline, self.wait_budget.read(operation)) => match result {
                Ok(value) => value,
                Err(_) => Err(self.end(AvailabilityEnd::ElapsedLimit)),
            },
        }
    }
    pub(crate) async fn retry(&mut self, mut error: YappinError) -> Result<()> {
        if self.finished {
            return Err(error);
        }
        if retryable_read(&error)
            && self.attempts < self.policy.max_attempts
            && error
                .retry_after()
                .is_some_and(|hint| hint > self.wait_budget.remaining())
        {
            error = self.wait_budget.exceeded(error);
        }
        self.last_error = Some(
            serde_json::from_value(self.client.redact_credentials(error.to_json()))
                .expect("structured cause"),
        );
        self.update(|s| {
            s.last_error = self.last_error.clone();
            s.elapsed = self.started.elapsed();
        });
        if !retryable_read(&error) {
            self.finished = true;
            self.update(|s| s.state = AvailabilityState::Terminal);
            return Err(error);
        }
        if self.attempts >= self.policy.max_attempts {
            return Err(self.end(AvailabilityEnd::AttemptLimit));
        }
        let delay = self.policy.delay(self.attempts, &error);
        if delay >= self.deadline.saturating_duration_since(Instant::now()) {
            return Err(self.end(AvailabilityEnd::BackoffExceedsDeadline));
        }
        self.update(|s| {
            s.state = AvailabilityState::Waiting;
            s.next_delay = Some(delay);
        });
        self.waiting = Some(Instant::now());
        self.advised_wait = error.retry_after().unwrap_or_default();
        let cancelled = tokio::select! {
            biased;
            _ = Self::cancelled(&self.handle) => true,
            _ = tokio::time::sleep(delay) => false,
        };
        self.end_wait();
        if cancelled {
            return Err(self.end(AvailabilityEnd::Cancelled));
        }
        Ok(())
    }
    pub(crate) fn success(&mut self) {
        self.finished = true;
        self.update(|s| {
            s.state = AvailabilityState::Recovered;
            s.elapsed = self.started.elapsed();
            s.next_delay = None;
        });
        if self.attempts > 1 {
            self.telemetry.update(|s| s.recoveries += 1);
        }
    }
    fn end_wait(&mut self) {
        if let Some(started) = self.waiting.take() {
            let elapsed = started.elapsed();
            self.wait_budget.charge(elapsed.min(self.advised_wait));
            self.advised_wait = Duration::ZERO;
            self.waited += elapsed;
            self.update(|s| {
                s.backoff_wait = self.waited;
                s.next_delay = None;
            });
            self.telemetry.update(|s| s.total_backoff_wait += elapsed);
        }
    }
    fn end(&mut self, reason: AvailabilityEnd) -> YappinError {
        self.end_wait();
        self.finished = true;
        self.update(|s| {
            s.state = if reason == AvailabilityEnd::Cancelled {
                AvailabilityState::Cancelled
            } else {
                AvailabilityState::Exhausted
            };
            s.elapsed = self.started.elapsed();
            s.next_delay = None;
        });
        self.telemetry.update(|s| {
            if reason == AvailabilityEnd::Cancelled {
                s.cancellations += 1;
            } else {
                s.exhaustions += 1;
            }
        });
        let failure = AvailabilityFailure {
            phase: self.phase,
            reason,
            attempts: self.attempts,
            elapsed: self.started.elapsed(),
            backoff_wait: self.waited,
            max_attempts: self.policy.max_attempts,
            max_elapsed: self.policy.max_elapsed,
            last_error: self.last_error.clone(),
        };
        YappinError::Remote(RemoteError {
            kind: "availability".into(),
            message: format!(
                "read availability ended during {:?}: {:?}",
                self.phase, reason
            ),
            status: self.last_error.as_ref().and_then(|e| e.status),
            retry_after: self.last_error.as_ref().and_then(|e| e.retry_after),
            details: [
                (
                    "availability".into(),
                    serde_json::to_value(failure).expect("availability report"),
                ),
                ("outcome".into(), "incomplete".into()),
                ("retry_safety".into(), "do_not_replay_workflow".into()),
            ]
            .into(),
        })
    }
}
impl Drop for Episode {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.end(AvailabilityEnd::Cancelled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_read_recovery_distinguishes_transport_from_local_shutdown_and_bad_frames() {
        for (reason, retryable) in [
            ("peer_closed", true),
            ("response_timeout", true),
            ("capacity", true),
            ("invalid_response", false),
            ("pipeline_shutdown", false),
            ("run_cancelled", false),
            ("drain_timeout", false),
        ] {
            let error = YappinError::Remote(RemoteError {
                kind: "gateway_request".into(),
                message: "request failed".into(),
                status: None,
                retry_after: None,
                details: [("reason".into(), reason.into())].into(),
            });
            assert_eq!(retryable_read(&error), retryable, "{reason}");
        }
    }

    #[test]
    fn bounds_and_jitter_respect_backoff_and_retry_hints() {
        assert!(AvailabilityPolicy::new(0, Duration::from_secs(1)).is_err());
        assert!(AvailabilityPolicy::new(1001, Duration::from_secs(1)).is_err());
        assert!(AvailabilityPolicy::new(1, Duration::ZERO).is_err());
        assert!(AvailabilityPolicy::default().jitter_percent(101).is_err());
        assert!(AvailabilityPolicy::default()
            .backoff(Duration::ZERO, Duration::from_secs(1))
            .is_err());
        assert!(AvailabilityPolicy::default()
            .backoff(Duration::from_secs(2), Duration::from_secs(1))
            .is_err());
        let policy = AvailabilityPolicy::default();
        let error = YappinError::Api {
            status: 503,
            message: "busy".into(),
            retry_after: None,
        };
        for attempt in [1, 2, 20, u32::MAX] {
            let base = policy
                .initial_backoff
                .saturating_mul(1u32 << attempt.saturating_sub(1).min(31))
                .min(policy.max_backoff);
            for _ in 0..32 {
                let delay = policy.delay(attempt, &error);
                assert!(delay <= base && delay >= base.mul_f64(0.8));
            }
        }
        let advised = YappinError::Api {
            status: 503,
            message: "wait".into(),
            retry_after: Some(30.0),
        };
        assert_eq!(policy.delay(1, &advised), Duration::from_secs(30));
    }
}
