//! Explicit retirement of assessed failed/uncertain work, never a handler replay.
use super::reconciliation::{bounded_for, digest, fingerprint, valid_hash as hash};
use super::*;
use crate::{Client, OperationId};
use chrono::{DateTime, Utc};

/// What the consumer attests. The SDK does not verify remote mutation outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventRetirementAssessment {
    AttestedApplied,
    AttestedNotApplied,
    /// Deliberately resume without replay while preserving the unknown outcome.
    AcceptUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventRetirementState {
    Prepared,
    Approved,
    PollingConfirmed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventRetirementEvidence {
    ExactPollingEvent,
    /// Explicitly retire the retained attempt despite unavailable historical
    /// event data. Only AcceptUnknown is allowed. Coverage is not equivalence.
    RetainedExecutionCoverage,
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EventRetirementRequest {
    pub bot_id: Uuid,
    pub event_id: u64,
    pub retained_fingerprint: String,
    pub polling_fingerprint: Option<String>,
    pub evidence: EventRetirementEvidence,
}
impl EventRetirementRequest {
    pub fn new(
        bot_id: Uuid,
        event_id: u64,
        retained: impl Into<String>,
        polling: impl Into<String>,
    ) -> Self {
        Self {
            bot_id,
            event_id,
            retained_fingerprint: retained.into(),
            polling_fingerprint: Some(polling.into()),
            evidence: EventRetirementEvidence::ExactPollingEvent,
        }
    }
    /// Select deliberately retained-work retirement, not payload reconciliation.
    /// The consumer must explicitly accept uncertainty during approval.
    pub fn retained_execution(bot_id: Uuid, event_id: u64, retained: impl Into<String>) -> Self {
        Self {
            bot_id,
            event_id,
            retained_fingerprint: retained.into(),
            polling_fingerprint: None,
            evidence: EventRetirementEvidence::RetainedExecutionCoverage,
        }
    }
}
/// Read/expiry bounds shared with timestamp preparation. Eligibility, decisions,
/// durable hooks and outcomes remain separate; this does not widen that API.
pub type EventRetirementOptions = TimestampReconciliationOptions;
/// Immutable audit evidence, stored separately from the original event/error.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EventRetirement {
    pub decision_id: Uuid,
    pub bot_id: Uuid,
    pub event_id: u64,
    pub retained_fingerprint: String,
    pub polling_fingerprint: Option<String>,
    pub evidence: EventRetirementEvidence,
    pub target_observed: bool,
    pub receipt_fingerprint: String,
    pub original_stage: ReceiptStage,
    pub polling_created_at: Option<DateTime<Utc>>,
    pub polling_watermark: u64,
    pub prepared_checkpoint: u64,
    pub prepared_at: DateTime<Utc>,
    pub approval_deadline: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub polling_confirmed_at: Option<DateTime<Utc>>,
    pub confirmation_watermark: Option<u64>,
    pub target_observed_at_confirmation: Option<bool>,
    pub state: EventRetirementState,
    pub assessment: Option<EventRetirementAssessment>,
    pub approval_reference: Option<OperationId>,
    pub evidence_reference: Option<OperationId>,
    pub conflicts: Vec<IngressConflict>,
}
pub struct PreparedEventRetirement {
    decision: EventRetirement,
    store: Arc<dyn IngressStore>,
    expires: tokio::time::Instant,
    timeout: Duration,
}
impl PreparedEventRetirement {
    pub fn evidence(&self) -> &EventRetirement {
        &self.decision
    }
    /// Consumer approval only: no remote success is inferred. Hold the stopped
    /// runner's exclusive store lease. Never retry an ambiguous durable commit;
    /// reload its decision first. References point to consumer-owned audit data.
    pub async fn approve(
        mut self,
        reference: OperationId,
        assessment: EventRetirementAssessment,
        evidence: OperationId,
    ) -> Result<EventRetirement> {
        if tokio::time::Instant::now() >= self.expires
            || Utc::now() > self.decision.approval_deadline
        {
            return Err(reject("expired_preparation"));
        }
        if self.decision.evidence == EventRetirementEvidence::RetainedExecutionCoverage
            && assessment != EventRetirementAssessment::AcceptUnknown
        {
            return Err(reject("unknown_assessment_required"));
        }
        self.decision.state = EventRetirementState::Approved;
        self.decision.approved_at = Some(Utc::now());
        self.decision.approval_reference = Some(reference);
        self.decision.assessment = Some(assessment);
        self.decision.evidence_reference = Some(evidence);
        let current = bounded(
            self.timeout,
            "retirement_load",
            self.store.load(self.decision.bot_id),
        )
        .await?;
        self.decision.validate_write(&current)?;
        bounded(
            self.timeout,
            "approval",
            self.store.commit_event_retirement(self.decision.clone()),
        )
        .await?;
        Ok(self.decision)
    }
}
/// GET-only preparation for a Started/Failed durable server-message receipt.
/// Exact mode requires the original Event or only a missing outer timestamp
/// becoming supplied. Explicit retained-execution mode accepts polling coverage
/// without claiming payload equivalence or remote success. No handlers, callbacks, mutation
/// retries, approvals or checkpoint writes occur here. Keep intake stopped and
/// hold the consumer's exclusive lease for preparation and approval.
pub async fn prepare_event_retirement(
    client: &Client,
    store: Arc<dyn IngressStore>,
    request: EventRetirementRequest,
    options: EventRetirementOptions,
) -> Result<PreparedEventRetirement> {
    if request.bot_id.is_nil()
        || request.event_id <= 1
        || !hash(&request.retained_fingerprint)
        || match request.evidence {
            EventRetirementEvidence::ExactPollingEvent => request
                .polling_fingerprint
                .as_deref()
                .is_none_or(|fp| !hash(fp)),
            EventRetirementEvidence::RetainedExecutionCoverage => {
                request.polling_fingerprint.is_some()
            }
        }
        || !options.valid()
    {
        return Err(reject("invalid_request_or_bounds"));
    }
    let budget = options.preparation_timeout.unwrap_or(options.timeout * 3);
    let deadline = tokio::time::Instant::now() + budget;
    let result = tokio::time::timeout_at(deadline, prepare(client, store, request, options))
        .await
        .unwrap_or_else(|_| Err(reject("preparation_timeout")));
    if result.is_ok() && tokio::time::Instant::now() >= deadline {
        return Err(reject("preparation_timeout"));
    }
    result
}
async fn prepare(
    client: &Client,
    store: Arc<dyn IngressStore>,
    request: EventRetirementRequest,
    options: EventRetirementOptions,
) -> Result<PreparedEventRetirement> {
    if bounded(
        options.timeout,
        "retirement_identity",
        crate::client::single_attempt(client.get_me()),
    )
    .await?
    .id != request.bot_id
    {
        return Err(reject("authenticated_bot_mismatch"));
    }
    let current = bounded(
        options.timeout,
        "retirement_load",
        store.load(request.bot_id),
    )
    .await?;
    let record = retained(&current, request.bot_id, request.event_id)?;
    if fingerprint(&record.event, 16 * 1024 * 1024)? != request.retained_fingerprint {
        return Err(reject("retained_fingerprint_mismatch"));
    }
    if request.evidence == EventRetirementEvidence::RetainedExecutionCoverage
        && current.polling_checkpoint == Some(0)
    {
        return Err(reject("polling_coverage_requires_saved_cursor"));
    }
    if current
        .event_retirements
        .iter()
        .any(|r| r.event_id == request.event_id)
    {
        return Err(reject("decision_exists"));
    }
    let response = bounded(
        options.timeout,
        "retirement_polling",
        crate::client::single_attempt(client.get_events(
            Some(
                if request.evidence == EventRetirementEvidence::ExactPollingEvent {
                    request.event_id - 1
                } else {
                    current
                        .polling_checkpoint
                        .ok_or_else(|| reject("checkpoint_missing"))?
                },
            ),
            Some(options.polling_wait),
            options.max_age,
        )),
    )
    .await?;
    if response
        .events
        .iter()
        .any(|event| event.id == 0 || event.id > response.last)
    {
        return Err(reject("invalid_polling_watermark"));
    }
    let mut selected = None;
    for event in response.events.iter().filter(|e| e.id == request.event_id) {
        let expected = request
            .polling_fingerprint
            .as_deref()
            .unwrap_or(&request.retained_fingerprint);
        if fingerprint(event, 16 * 1024 * 1024)? != expected {
            return Err(reject("polling_fingerprint_mismatch"));
        }
        selected = Some(event);
    }
    if request.evidence == EventRetirementEvidence::ExactPollingEvent && selected.is_none() {
        return Err(reject("authoritative_event_unavailable"));
    }
    if response.last < request.event_id {
        return Err(reject("authoritative_coverage_unavailable"));
    }
    let now = Utc::now();
    let decision = EventRetirement {
        decision_id: Uuid::new_v4(),
        bot_id: request.bot_id,
        event_id: request.event_id,
        retained_fingerprint: request.retained_fingerprint,
        polling_fingerprint: request.polling_fingerprint,
        evidence: request.evidence,
        target_observed: selected.is_some(),
        receipt_fingerprint: digest(record)?,
        original_stage: record.status.stage,
        polling_created_at: if request.evidence == EventRetirementEvidence::ExactPollingEvent {
            selected.and_then(|event| event.created_at)
        } else {
            None
        },
        polling_watermark: response.last,
        prepared_checkpoint: current
            .polling_checkpoint
            .ok_or_else(|| reject("checkpoint_missing"))?,
        prepared_at: now,
        approval_deadline: now
            + chrono::Duration::from_std(options.approval_ttl)
                .map_err(|_| reject("invalid_ttl"))?,
        approved_at: None,
        polling_confirmed_at: None,
        confirmation_watermark: None,
        target_observed_at_confirmation: None,
        state: EventRetirementState::Prepared,
        assessment: None,
        approval_reference: None,
        evidence_reference: None,
        conflicts: current
            .conflicts
            .iter()
            .filter(|r| r.event_id == request.event_id)
            .cloned()
            .collect(),
    };
    decision.validate_evidence(&current)?;
    Ok(PreparedEventRetirement {
        decision,
        store,
        expires: tokio::time::Instant::now() + options.approval_ttl,
        timeout: options.timeout,
    })
}
impl EventRetirement {
    /// Call inside the same atomic transaction that writes/transitions the audit
    /// decision. Original event, Failed/Started state, error/partial receipts,
    /// conflicts and checkpoint are immutable. Archive them before pruning.
    pub fn validate_write(&self, current: &IngressRecovery) -> Result<()> {
        self.validate_loaded(current)?;
        let prior = current
            .event_retirements
            .iter()
            .filter(|r| r.event_id == self.event_id)
            .collect::<Vec<_>>();
        if prior.len() > 1 {
            return Err(reject("duplicate_decision"));
        }
        match (self.state, prior.first()) {
            (EventRetirementState::Approved, None) => {
                if current.polling_checkpoint != Some(self.prepared_checkpoint)
                    || Utc::now() > self.approval_deadline
                    || digest(&self.conflicts)?
                        != digest(
                            &current
                                .conflicts
                                .iter()
                                .filter(|r| r.event_id == self.event_id)
                                .cloned()
                                .collect::<Vec<_>>(),
                        )?
                {
                    return Err(reject("stale_preparation"));
                }
            }
            (_, Some(prior)) => {
                let mut expected = (*prior).clone();
                if self.state == EventRetirementState::PollingConfirmed
                    && prior.state == EventRetirementState::Approved
                {
                    expected.state = self.state;
                    expected.polling_confirmed_at = self.polling_confirmed_at;
                    expected.confirmation_watermark = self.confirmation_watermark;
                    expected.target_observed_at_confirmation = self.target_observed_at_confirmation;
                }
                if digest(&expected)? != digest(self)? {
                    return Err(reject("stale_or_conflicting_decision"));
                }
            }
            _ => return Err(reject("approval_missing")),
        }
        Ok(())
    }
    pub(super) fn validate_loaded(&self, current: &IngressRecovery) -> Result<()> {
        self.validate_evidence(current)?;
        if self.state == EventRetirementState::Prepared
            || self.assessment.is_none()
            || self.approval_reference.is_none()
            || self.evidence_reference.is_none()
            || self
                .approved_at
                .is_none_or(|v| v < self.prepared_at || v > self.approval_deadline)
            || (self.state == EventRetirementState::PollingConfirmed)
                != self.polling_confirmed_at.is_some()
            || (self.state == EventRetirementState::PollingConfirmed)
                != self.confirmation_watermark.is_some()
            || (self.state == EventRetirementState::PollingConfirmed)
                != self.target_observed_at_confirmation.is_some()
            || self
                .confirmation_watermark
                .is_some_and(|id| id < self.event_id)
            || self.evidence == EventRetirementEvidence::RetainedExecutionCoverage
                && self.assessment != Some(EventRetirementAssessment::AcceptUnknown)
            || self.state == EventRetirementState::PollingConfirmed
                && self.evidence == EventRetirementEvidence::ExactPollingEvent
                && self.target_observed_at_confirmation != Some(true)
            || self
                .polling_confirmed_at
                .is_some_and(|v| self.approved_at.is_none_or(|a| v < a))
        {
            return Err(reject("approval_required_or_invalid"));
        }
        Ok(())
    }
    fn validate_evidence(&self, current: &IngressRecovery) -> Result<()> {
        let record = retained(current, self.bot_id, self.event_id)?;
        if self.decision_id.is_nil()
            || self.event_id <= 1
            || !hash(&self.retained_fingerprint)
            || match self.evidence {
                EventRetirementEvidence::ExactPollingEvent => {
                    !self.target_observed
                        || self
                            .polling_fingerprint
                            .as_deref()
                            .is_none_or(|fp| !hash(fp))
                }
                EventRetirementEvidence::RetainedExecutionCoverage => {
                    self.polling_fingerprint.is_some() || self.polling_created_at.is_some()
                }
            }
            || self.original_stage != record.status.stage
            || digest(record)? != self.receipt_fingerprint
            || fingerprint(&record.event, 16 * 1024 * 1024)? != self.retained_fingerprint
            || self.polling_watermark < self.event_id
            || current
                .polling_checkpoint
                .is_none_or(|v| v < self.prepared_checkpoint)
            || self.approval_deadline <= self.prepared_at
            || self.approval_deadline - self.prepared_at > chrono::Duration::minutes(5)
            || self.conflicts.len() > 1024
            || !self
                .conflicts
                .iter()
                .all(|r| r.validate() && r.bot_id == self.bot_id && r.event_id == self.event_id)
            || conflict_set(&self.conflicts)?
                != conflict_set(
                    &current
                        .conflicts
                        .iter()
                        .filter(|r| r.event_id == self.event_id)
                        .cloned()
                        .collect::<Vec<_>>(),
                )?
        {
            return Err(reject("retained_evidence_mismatch"));
        }
        if self.evidence == EventRetirementEvidence::ExactPollingEvent {
            let mut candidate = record.event.clone();
            candidate.created_at = self.polling_created_at;
            if self.polling_fingerprint.as_deref()
                != Some(&fingerprint(&candidate, 16 * 1024 * 1024)?)
                || record.event.created_at.is_some()
                    && record.event.created_at != self.polling_created_at
            {
                return Err(reject("content_identity_or_timestamp_change"));
            }
        }
        Ok(())
    }
}
fn conflict_set(reports: &[IngressConflict]) -> Result<String> {
    let mut hashes = Vec::with_capacity(reports.len());
    for report in reports {
        let mut stable = report.clone();
        stable.disposition = ConflictDisposition::Stop;
        hashes.push(digest(&stable)?);
    }
    hashes.sort_unstable();
    digest(&hashes)
}
fn retained(current: &IngressRecovery, bot: Uuid, id: u64) -> Result<&IngressRecord> {
    let mut records = current.records.iter().filter(|r| r.event.id == id);
    let record = records
        .next()
        .ok_or_else(|| reject("retained_event_missing"))?;
    if records.next().is_some()
        || bot.is_nil()
        || record.bot_id != bot
        || record.status.event_id != id
        || record.status.committed
        || !record.status.payload_retained
        || record.status.durability != AdmissionDurability::Durable
        || !matches!(
            record.status.stage,
            ReceiptStage::Started | ReceiptStage::Failed
        )
        || current.polling_checkpoint.is_none_or(|v| v >= id)
        || current
            .timestamp_reconciliations
            .iter()
            .any(|r| r.event_id == id)
        || record.event.kind != "message"
        || ["id", "server_id", "channel_id", "author_id"]
            .iter()
            .any(|key| {
                record
                    .event
                    .data
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .and_then(|v| Uuid::parse_str(v).ok())
                    .is_none_or(|id| id.is_nil())
            })
    {
        return Err(reject("ineligible_receipt"));
    }
    Ok(record)
}
async fn bounded<T>(
    timeout: Duration,
    phase: &str,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    bounded_for("event_retirement", timeout, phase, future).await
}
fn reject(reason: &str) -> YappinError {
    YappinError::Remote(RemoteError {
        kind: "event_retirement".into(),
        message: "explicit event retirement rejected; original work must not be replayed".into(),
        status: None,
        retry_after: None,
        details: BTreeMap::from([
            ("reason".into(), reason.into()),
            ("outcome".into(), "not_applied".into()),
            ("retry_safety".into(), "do_not_replay_workflow".into()),
        ]),
    })
}
impl EventIngress {
    pub fn retirements(&self) -> Vec<EventRetirement> {
        self.0
            .state
            .lock()
            .unwrap()
            .claims
            .values()
            .filter_map(|c| c.retirement.clone())
            .collect()
    }
    pub(in crate::bot) fn retired(&self, id: u64) -> bool {
        self.0
            .state
            .lock()
            .unwrap()
            .claims
            .get(&id)
            .and_then(|c| c.retirement.as_ref())
            .is_some_and(|r| r.state == EventRetirementState::PollingConfirmed)
    }
    pub(super) async fn confirm_retirement(
        &self,
        id: u64,
        source: IngressSource,
        hash: [u8; 32],
    ) -> Result<bool> {
        let Some(decision) = self
            .0
            .state
            .lock()
            .unwrap()
            .claims
            .get(&id)
            .and_then(|c| c.retirement.clone())
        else {
            return Ok(false);
        };
        let hash = hex(&hash);
        let expected = decision
            .polling_fingerprint
            .as_deref()
            .unwrap_or(&decision.retained_fingerprint);
        if decision.state == EventRetirementState::PollingConfirmed {
            return Ok(hash == expected || hash == decision.retained_fingerprint);
        }
        if source != IngressSource::Polling {
            let covered = hash == expected
                || hash == decision.retained_fingerprint
                || decision
                    .conflicts
                    .iter()
                    .any(|r| r.incoming_fingerprint == hash || r.existing_fingerprint == hash);
            return if covered {
                Err(reject("awaiting_normal_polling_confirmation"))
            } else {
                Ok(false)
            };
        }
        if hash != expected {
            return Err(reject("polling_confirmation_mismatch"));
        }
        self.publish_retirement(decision, id, true).await?;
        Ok(true)
    }
    /// Called only on an actual polling batch, before claims/effects. Coverage
    /// retirement is explicit acceptance of unknown work, never payload equivalence.
    pub(in crate::bot) async fn confirm_retirement_coverage(
        &self,
        events: &[CanonicalEvent],
        watermark: u64,
    ) -> Result<()> {
        let _fence = self.0.checkpoint_fence.lock().await;
        let decisions = self
            .0
            .state
            .lock()
            .unwrap()
            .claims
            .values()
            .filter_map(|c| c.retirement.as_ref())
            .filter(|r| {
                r.state == EventRetirementState::Approved
                    && r.evidence == EventRetirementEvidence::RetainedExecutionCoverage
                    && r.event_id <= watermark
            })
            .cloned()
            .collect::<Vec<_>>();
        for decision in decisions {
            let target = events
                .iter()
                .find(|event| event.event.id == decision.event_id);
            if let Some(target) = target {
                if hex(&target.fingerprint) != decision.retained_fingerprint {
                    return Err(reject("polling_confirmation_mismatch"));
                }
            }
            self.publish_retirement(decision, watermark, target.is_some())
                .await?;
        }
        Ok(())
    }
    async fn publish_retirement(
        &self,
        mut decision: EventRetirement,
        watermark: u64,
        target_seen: bool,
    ) -> Result<()> {
        let id = decision.event_id;
        // checkpoint_fence is held by both callers; stores revalidate atomically.
        decision.state = EventRetirementState::PollingConfirmed;
        decision.polling_confirmed_at = Some(Utc::now());
        decision.confirmation_watermark = Some(watermark);
        decision.target_observed_at_confirmation = Some(target_seen);
        let mut guard = ConflictGuard {
            ingress: self.clone(),
            armed: true,
            conflict: self.conflicts().into_iter().find(|r| r.event_id == id),
        };
        if let Err(error) = bounded(
            self.0.config.persistence_timeout,
            "confirmation",
            self.0
                .store
                .as_ref()
                .expect("durable retirement")
                .commit_event_retirement(decision.clone()),
        )
        .await
        {
            self.fatal(error.to_remote());
            guard.armed = false;
            return Err(error);
        }
        let mut state = self.0.state.lock().unwrap();
        let claim = state.claims.get_mut(&id).unwrap();
        claim.retirement = Some(decision);
        claim.conflict = false;
        claim.pending_since = None;
        claim.polling_verified = true;
        for report in state.conflicts.iter_mut().filter(|r| r.event_id == id) {
            report.disposition = ConflictDisposition::Retired;
        }
        guard.armed = false;
        drop(state);
        self.0.changed.notify_one();
        Ok(())
    }
}
