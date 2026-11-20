use super::IngressRecord;
use crate::{Client, OperationId, Result};

/// Stable application identifiers for the final unfinished step of a retained event.
///
/// ```rust
/// use rappin::{IngressRecord, OperationId, RecoveryStep, Result};
/// fn follow_up(original: &IngressRecord) -> Result<RecoveryStep> {
///     RecoveryStep::new(
///         original,
///         OperationId::new("publish/v1")?,
///         OperationId::new("follow-up")?,
///         OperationId::new("journal/entry-42")?,
///     )
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RecoveryStep {
    bot_id: uuid::Uuid,
    event_id: u64,
    fingerprint: String,
    /// Versioned command binding saved by the application at original execution.
    pub binding: OperationId,
    /// Stable step identifier within that binding.
    pub step: OperationId,
    /// Reference to the durable evidence that this step can execute safely.
    pub evidence: OperationId,
    /// The application asserts this step is idempotent, so a claimed attempt that failed
    /// or was cancelled may be claimed and run again. Defaults to `false`: an unfinished
    /// attempt stays blocked for explicit reconciliation. Set with [`RecoveryStep::allow_retry`].
    #[serde(default)]
    pub retry_safe: bool,
}

impl RecoveryStep {
    /// Bind a step and its evidence to the exact retained event.
    ///
    /// ```rust
    /// use rappin::{IngressRecord, OperationId, RecoveryStep, Result};
    /// fn follow_up(original: &IngressRecord) -> Result<RecoveryStep> {
    ///     RecoveryStep::new(
    ///         original,
    ///         OperationId::new("publish/v1")?,
    ///         OperationId::new("follow-up")?,
    ///         OperationId::new("journal/entry-42")?,
    ///     )
    /// }
    /// ```
    pub fn new(
        original: &IngressRecord,
        binding: OperationId,
        step: OperationId,
        evidence: OperationId,
    ) -> Result<Self> {
        Ok(Self {
            bot_id: original.bot_id,
            event_id: original.event.id,
            fingerprint: super::canonical_event_fingerprint(&original.event)?,
            binding,
            step,
            evidence,
            retry_safe: false,
        })
    }

    /// Mark the step idempotent: running it again after a failed or cancelled attempt
    /// has the same effect as running it once (for example confirming a commit or
    /// discarding a response). Only use this when the step's own effects make repeats safe.
    ///
    /// ```rust
    /// use rappin::{IngressRecord, OperationId, RecoveryStep, Result};
    /// fn confirm(original: &IngressRecord) -> Result<RecoveryStep> {
    ///     Ok(RecoveryStep::new(
    ///         original,
    ///         OperationId::new("publish/v1")?,
    ///         OperationId::new("confirm-commit")?,
    ///         OperationId::new("journal/entry-42")?,
    ///     )?
    ///     .allow_retry())
    /// }
    /// ```
    pub fn allow_retry(mut self) -> Self {
        self.retry_safe = true;
        self
    }

    /// Whether a store may replace the durable unfinished attempt `existing` with this
    /// claim. True only when both are the same retry-safe step (same bot, event,
    /// fingerprint, binding, step and evidence). Stores call this inside
    /// [`begin_step_recovery`](crate::IngressStore::begin_step_recovery) instead of
    /// rejecting every existing attempt.
    ///
    /// ```rust
    /// use rappin::RecoveryStep;
    /// fn reclaim(existing: &RecoveryStep, next: &RecoveryStep) -> bool {
    ///     next.may_reclaim(existing)
    /// }
    /// ```
    pub fn may_reclaim(&self, existing: &RecoveryStep) -> bool {
        self.retry_safe && existing.retry_safe && self == existing
    }

    /// Verify the step belongs to this bot and the exact retained event.
    /// Stores also compare the receipt and journal state in their transaction.
    ///
    /// ```rust
    /// use rappin::{IngressRecord, RecoveryStep, Result};
    /// fn validate(step: &RecoveryStep, retained: &IngressRecord) -> Result<()> {
    ///     step.validate_record(retained)
    /// }
    /// ```
    pub fn validate_record(&self, retained: &IngressRecord) -> Result<()> {
        if self.bot_id != retained.bot_id
            || self.event_id != retained.event.id
            || self.event_id != retained.status.event_id
            || self.fingerprint != super::canonical_event_fingerprint(&retained.event)?
        {
            return Err(crate::Error::validation(
                "ingress.step_recovery",
                "recovery step does not match the retained event",
            ));
        }
        Ok(())
    }
}

/// Application-owned startup continuation for events handled by native or worker commands.
///
/// The hook runs on the parent. It uses the original durable event and application
/// journal without entering normal command dispatch. Enable it alongside an
/// [ingress store](crate::IngressStore) implementing the recovery transactions.
///
/// ```rust
/// use std::sync::Arc;
/// use rappin::{Bot, IngressStepRecovery, IngressStore, Result};
/// fn configure(store: Arc<dyn IngressStore>, recovery: Arc<dyn IngressStepRecovery>) -> Result<Bot> {
///     Bot::builder("unused-documentation-token")
///         .configure(|options| {
///             options.persistence.store = Some(store);
///             options.persistence.step_recovery = Some(recovery);
///         })
///         .build()
/// }
/// ```
#[async_trait::async_trait]
pub trait IngressStepRecovery: Send + Sync + 'static {
    /// Read the journal for the same bot, event fingerprint and original binding.
    /// Return a plan only when preceding work is accounted for and durable evidence
    /// proves that the last unfinished step did not take effect. This method may
    /// be called again after a crash and must perform no workflow effects.
    ///
    /// ```rust
    /// # use rappin::{IngressRecord, IngressStepRecovery, RecoveryStep, Result};
    /// async fn assess(hook: &dyn IngressStepRecovery, original: &IngressRecord) -> Result<Option<RecoveryStep>> {
    ///     hook.plan(original).await
    /// }
    /// ```
    async fn plan(&self, original: &IngressRecord) -> Result<Option<RecoveryStep>>;

    /// Execute only the planned step after its attempt has been durably claimed.
    /// Return success once all required work is complete. The application owns
    /// authorization, external reconciliation and step-specific receipts. Failures
    /// or cancellation leave the attempt unresolved for explicit reconciliation,
    /// unless the step was marked [`allow_retry`](RecoveryStep::allow_retry): then the
    /// next startup may plan, claim and run it again.
    ///
    /// ```rust
    /// # use rappin::{Client, IngressRecord, IngressStepRecovery, RecoveryStep, Result};
    /// async fn after_durable_claim(
    ///     hook: &dyn IngressStepRecovery, client: &Client, original: &IngressRecord, step: &RecoveryStep,
    /// ) -> Result<()> {
    ///     hook.resume(client, original, step).await
    /// }
    /// ```
    async fn resume(
        &self,
        client: &Client,
        original: &IngressRecord,
        step: &RecoveryStep,
    ) -> Result<()>;
}
