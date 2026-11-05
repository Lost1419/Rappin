//! Explicit, fingerprint-bound reconciliation of a completed legacy envelope.
use super::*;
use crate::{Client, OperationId};
use chrono::{DateTime, Utc};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPORTS: usize = 1024;

/// Only a missing outer timestamp becoming a supplied timestamp is supported.
/// This is a consumer-approved exception, not a claim of platform equivalence.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TimestampReconciliationRequest {
    pub bot_id: Uuid,
    pub event_id: u64,
    pub retained_fingerprint: String,
    pub polling_fingerprint: String,
}
impl TimestampReconciliationRequest {
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
            polling_fingerprint: polling.into(),
        }
    }
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TimestampReconciliationOptions {
    /// Bounds each network/storage operation; cancellation never retries a write.
    pub timeout: Duration,
    /// Preparation expires before approval (maximum five minutes).
    pub approval_ttl: Duration,
    /// Read-only polling retention filter, not a checkpoint adjustment.
    pub max_age: Option<u32>,
}
impl Default for TimestampReconciliationOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            approval_ttl: Duration::from_secs(300),
            max_age: Some(86400),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TimestampReconciliationState {
    Prepared,
    Approved,
    Confirmed,
}
/// Separate durable audit record. Never replaces the original event or receipt.
/// Consumers must retain this record while its receipt/conflicts remain loadable.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TimestampReconciliation {
    pub decision_id: Uuid,
    pub approval_id: OperationId,
    pub bot_id: Uuid,
    pub event_id: u64,
    pub retained_fingerprint: String,
    pub polling_fingerprint: String,
    pub polling_created_at: DateTime<Utc>,
    pub polling_watermark: u64,
    pub prepared_checkpoint: u64,
    pub prepared_at: DateTime<Utc>,
    pub approved_at: DateTime<Utc>,
    pub approval_deadline: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub state: TimestampReconciliationState,
    /// Digest of the full retained receipt (including source and error state).
    pub receipt_fingerprint: String,
    /// Immutable original reports; live reports may acquire a resolved disposition.
    pub conflicts: Vec<IngressConflict>,
}
/// Opaque, single-use preparation: GET /me + GET /events and store reads only.
/// Approval policy belongs to the consumer. Dropping this value has no effects.
pub struct PreparedTimestampReconciliation {
    decision: TimestampReconciliation,
    store: Arc<dyn IngressStore>,
    expires: tokio::time::Instant,
    timeout: Duration,
}
impl PreparedTimestampReconciliation {
    /// Content-free evidence for the consumer's explicit approval decision.
    /// State is a proposal until `approve` successfully commits it.
    pub fn evidence(&self) -> &TimestampReconciliation {
        &self.decision
    }
    /// Commit once through the store's atomic compare-and-swap hook. An error,
    /// timeout or cancellation may follow a durable commit: load before deciding
    /// what to do next. Never retry this write or a completed handler blindly.
    pub async fn approve(mut self, approval_id: OperationId) -> Result<TimestampReconciliation> {
        if tokio::time::Instant::now() >= self.expires
            || Utc::now() > self.decision.approval_deadline
        {
            return Err(rejected("expired_preparation"));
        }
        self.decision.state = TimestampReconciliationState::Approved;
        self.decision.approval_id = approval_id;
        self.decision.approved_at = Utc::now();
        let snapshot = bounded(self.timeout, "load", self.store.load(self.decision.bot_id)).await?;
        self.decision.validate_write(&snapshot)?;
        bounded(
            self.timeout,
            "approval",
            self.store
                .commit_timestamp_reconciliation(self.decision.clone()),
        )
        .await?;
        Ok(self.decision)
    }
}
/// Read fresh authenticated polling evidence without starting intake, dispatching
/// handlers, acknowledging interactions, or writing any event/checkpoint. Requires
/// the runner to be stopped and the consumer's exclusive store lease to be held.
/// Empty polling results, higher watermarks and nested timestamps are insufficient.
pub async fn prepare_timestamp_reconciliation(
    client: &Client,
    store: Arc<dyn IngressStore>,
    request: TimestampReconciliationRequest,
    options: TimestampReconciliationOptions,
) -> Result<PreparedTimestampReconciliation> {
    if request.bot_id.is_nil()
        || request.event_id <= 1
        || !valid_hash(&request.retained_fingerprint)
        || !valid_hash(&request.polling_fingerprint)
        || request.retained_fingerprint == request.polling_fingerprint
        || options.timeout.is_zero()
        || options.timeout > Duration::from_secs(120)
        || options.approval_ttl.is_zero()
        || options.approval_ttl > Duration::from_secs(300)
        || options.max_age.is_some_and(|age| age > 86400)
    {
        return Err(rejected("invalid_request_or_bounds"));
    }
    let me = bounded(
        options.timeout,
        "identity",
        crate::client::single_attempt(client.get_me()),
    )
    .await?;
    if me.id != request.bot_id {
        return Err(rejected("authenticated_bot_mismatch"));
    }
    let snapshot = bounded(options.timeout, "load", store.load(request.bot_id)).await?;
    let record = retained(&snapshot, request.bot_id, request.event_id)?;
    if fingerprint(&record.event, MAX_BYTES)? != request.retained_fingerprint
        || record.event.created_at.is_some()
    {
        return Err(rejected("retained_fingerprint_or_timestamp_mismatch"));
    }
    // after=0 is an API bootstrap operation, so event 1 cannot use this probe.
    let response = bounded(
        options.timeout,
        "polling",
        crate::client::single_attempt(client.get_events(
            Some(request.event_id - 1),
            Some(0),
            options.max_age,
        )),
    )
    .await?;
    let mut incoming = None;
    for event in response.events.iter().filter(|e| e.id == request.event_id) {
        if fingerprint(event, MAX_BYTES)? != request.polling_fingerprint {
            return Err(rejected("polling_fingerprint_mismatch"));
        }
        incoming = Some(event);
    }
    let incoming = incoming.ok_or_else(|| rejected("authoritative_event_unavailable"))?;
    if response.last < request.event_id {
        return Err(rejected("invalid_polling_watermark"));
    }
    let timestamp = incoming
        .created_at
        .ok_or_else(|| rejected("polling_timestamp_missing"))?;
    let mut without_timestamp = incoming.clone();
    without_timestamp.created_at = None;
    if fingerprint(&without_timestamp, MAX_BYTES)? != request.retained_fingerprint {
        return Err(rejected("non_metadata_difference"));
    }
    let now = Utc::now();
    let mut decision = TimestampReconciliation {
        decision_id: Uuid::new_v4(),
        approval_id: OperationId::new("unapproved")?,
        bot_id: request.bot_id,
        event_id: request.event_id,
        retained_fingerprint: request.retained_fingerprint,
        polling_fingerprint: request.polling_fingerprint,
        polling_created_at: timestamp,
        polling_watermark: response.last,
        prepared_checkpoint: snapshot
            .polling_checkpoint
            .ok_or_else(|| rejected("checkpoint_missing"))?,
        prepared_at: now,
        approved_at: now,
        approval_deadline: now
            + chrono::Duration::from_std(options.approval_ttl)
                .map_err(|_| rejected("invalid_ttl"))?,
        confirmed_at: None,
        state: TimestampReconciliationState::Approved,
        receipt_fingerprint: digest(record)?,
        conflicts: snapshot
            .conflicts
            .iter()
            .filter(|r| r.event_id == request.event_id)
            .cloned()
            .collect(),
    };
    decision.validate_write(&snapshot)?;
    decision.state = TimestampReconciliationState::Prepared;
    Ok(PreparedTimestampReconciliation {
        decision,
        store,
        expires: tokio::time::Instant::now() + options.approval_ttl,
        timeout: options.timeout,
    })
}
impl TimestampReconciliation {
    /// Run INSIDE the same exclusive database transaction that inserts/transitions
    /// this decision. A prior load plus a later unconditional write is insufficient.
    /// The transaction preserves original event/completion, reports and checkpoint.
    /// Repeated identical writes are accepted for storage-level idempotence only;
    /// the SDK never automatically resubmits an ambiguous commit.
    pub fn validate_write(&self, current: &IngressRecovery) -> Result<()> {
        if self.state == TimestampReconciliationState::Prepared {
            return Err(rejected("approval_required"));
        }
        self.validate_retained(current)?;
        let prior = current
            .timestamp_reconciliations
            .iter()
            .filter(|r| r.event_id == self.event_id)
            .collect::<Vec<_>>();
        if prior.len() > 1 {
            return Err(rejected("duplicate_decision"));
        }
        match (self.state, prior.first()) {
            (TimestampReconciliationState::Approved, None) => {
                if current.polling_checkpoint != Some(self.prepared_checkpoint)
                    || self.confirmed_at.is_some()
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
                    return Err(rejected("stale_preparation"));
                }
            }
            (_, Some(prior)) => {
                let mut expected = (*prior).clone();
                if self.state == TimestampReconciliationState::Confirmed
                    && prior.state == TimestampReconciliationState::Approved
                {
                    expected.state = self.state;
                    expected.confirmed_at = self.confirmed_at;
                }
                if digest(&expected)? != digest(self)? {
                    return Err(rejected("stale_or_conflicting_decision"));
                }
            }
            _ => return Err(rejected("approval_missing")),
        }
        Ok(())
    }
    pub(super) fn validate_retained(&self, current: &IngressRecovery) -> Result<()> {
        if self.state == TimestampReconciliationState::Prepared {
            return Err(rejected("approval_required"));
        }
        let record = retained(current, self.bot_id, self.event_id)?;
        let checkpoint = current
            .polling_checkpoint
            .ok_or_else(|| rejected("checkpoint_missing"))?;
        if self.decision_id.is_nil()
            || self.event_id <= 1
            || !valid_hash(&self.retained_fingerprint)
            || !valid_hash(&self.polling_fingerprint)
            || self.retained_fingerprint == self.polling_fingerprint
            || self.polling_watermark < self.event_id
            || checkpoint < self.prepared_checkpoint
            || self.approved_at < self.prepared_at
            || self.approved_at > self.approval_deadline
            || self.approval_deadline <= self.prepared_at
            || self.approval_deadline - self.prepared_at > chrono::Duration::minutes(5)
            || (self.state == TimestampReconciliationState::Confirmed)
                != self.confirmed_at.is_some()
            || self
                .confirmed_at
                .is_some_and(|time| time < self.approved_at)
            || digest(record)? != self.receipt_fingerprint
            || record.event.created_at.is_some()
            || fingerprint(&record.event, MAX_BYTES)? != self.retained_fingerprint
        {
            return Err(rejected("retained_decision_mismatch"));
        }
        let mut candidate = record.event.clone();
        candidate.created_at = Some(self.polling_created_at);
        if fingerprint(&candidate, MAX_BYTES)? != self.polling_fingerprint {
            return Err(rejected("non_metadata_difference"));
        }
        // Full canonical hashes above verify all Event fields, including data;
        // bounded path/type diagnostics alone are never used as proof.
        if self.conflicts.is_empty()
            || self.conflicts.len() > MAX_REPORTS
            || !self
                .conflicts
                .iter()
                .any(|r| r.incoming_source == IngressSource::Polling)
            || !self.conflicts.iter().all(|r| self.covers(r))
            || !current
                .conflicts
                .iter()
                .any(|r| r.event_id == self.event_id)
            || !current
                .conflicts
                .iter()
                .filter(|r| r.event_id == self.event_id)
                .all(|r| self.covers(r))
        {
            return Err(rejected("conflict_evidence_mismatch"));
        }
        Ok(())
    }
    pub(super) fn covers(&self, report: &IngressConflict) -> bool {
        report.validate()
            && report.bot_id == self.bot_id
            && report.event_id == self.event_id
            && report.existing_fingerprint == self.retained_fingerprint
            && report.incoming_fingerprint == self.polling_fingerprint
            && report.retained_stage == Some(ReceiptStage::Completed)
            && !report.differences_truncated
            && report.differences.len() == 1
            && report.differences[0].path == "/created_at"
            && report.differences[0].existing_type == PayloadType::Null
            && report.differences[0].incoming_type == PayloadType::String
    }
}
fn retained(snapshot: &IngressRecovery, bot: Uuid, id: u64) -> Result<&IngressRecord> {
    let mut records = snapshot.records.iter().filter(|r| r.event.id == id);
    let record = records
        .next()
        .ok_or_else(|| rejected("retained_event_missing"))?;
    if records.next().is_some()
        || bot.is_nil()
        || record.bot_id != bot
        || record.status.event_id != id
        || record.status.stage != ReceiptStage::Completed
        || record.status.durability != AdmissionDurability::Durable
        || record.status.committed
        || !record.status.payload_retained
        || record.status.error.is_some()
        || snapshot
            .polling_checkpoint
            .is_none_or(|cursor| cursor >= id)
    {
        return Err(rejected("ineligible_receipt"));
    }
    // Deliberately limited to server messages. No interaction deadlines or other
    // timestamp-sensitive event kinds are eligible for this legacy exception.
    let data = &record.event.data;
    if record.event.kind != "message"
        || ["id", "server_id", "channel_id", "author_id"]
            .iter()
            .any(|key| {
                data.get(*key)
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .is_none_or(|id| id.is_nil())
            })
    {
        return Err(rejected("unsupported_event_scope"));
    }
    Ok(record)
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn fingerprint(event: &Event, limit: usize) -> Result<String> {
    Ok(hex(&event_fingerprint(event, limit)?))
}

fn digest(value: &impl Serialize) -> Result<String> {
    let mut value = serde_json::to_value(value)?;
    conflicts::canonicalize(&mut value);
    Ok(hex(&Sha256::digest(serde_json::to_vec(&value)?).into()))
}
fn rejected(reason: &str) -> YappinError {
    YappinError::Remote(RemoteError {
        kind: "timestamp_reconciliation".into(),
        message: "completed event timestamp reconciliation rejected".into(),
        status: None,
        retry_after: None,
        details: BTreeMap::from([
            ("reason".into(), reason.into()),
            (
                "retry_safety".into(),
                "inspect_store_do_not_replay_workflow".into(),
            ),
        ]),
    })
}
pub(super) async fn bounded<T>(
    timeout: Duration,
    phase: &str,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(timeout, future)
        .await
        .unwrap_or_else(|_| {
            Err(YappinError::Config(
                "reconciliation operation timed out; inspect store before retrying".into(),
            ))
        })
        .map_err(|cause| {
            YappinError::Remote(RemoteError {
                kind: "timestamp_reconciliation".into(),
                message: "reconciliation read or durable commit failed".into(),
                status: cause.status_code(),
                retry_after: cause.retry_after().map(|d| d.as_secs_f64()),
                details: BTreeMap::from([
                    ("phase".into(), phase.into()),
                    ("source".into(), cause.to_json()),
                    (
                        "outcome".into(),
                        if matches!(phase, "approval" | "confirmation") {
                            "unknown"
                        } else {
                            "not_applied"
                        }
                        .into(),
                    ),
                    (
                        "retry_safety".into(),
                        "inspect_store_do_not_replay_workflow".into(),
                    ),
                ]),
            })
        })
}

impl EventIngress {
    // The caller holds checkpoint_fence throughout persistence and publication.
    pub(super) async fn confirm_reconciliation(
        &self,
        id: u64,
        source: IngressSource,
        mut decision: TimestampReconciliation,
    ) -> Result<bool> {
        if decision.state == TimestampReconciliationState::Confirmed {
            return Ok(true);
        }
        let report = {
            let state = self.0.state.lock().unwrap();
            if let Some(report) = state.conflicts.iter().find(|r| {
                r.event_id == id
                    && (!decision.covers(r)
                        || matches!(
                            r.disposition,
                            ConflictDisposition::Stop | ConflictDisposition::TimedOut
                        ))
            }) {
                return Err(report.to_error());
            }
            if let Some(error) = &state.fatal {
                return Err(error.clone().into());
            }
            state
                .conflicts
                .iter()
                .find(|r| r.event_id == id)
                .expect("approved retained conflict")
                .clone()
        };
        if source != IngressSource::Polling {
            return Err(report.to_error());
        }
        decision.state = TimestampReconciliationState::Confirmed;
        decision.confirmed_at = Some(Utc::now());
        let mut guard = ConflictGuard {
            ingress: self.clone(),
            armed: true,
            conflict: Some(report.clone()),
        };
        let store = self
            .0
            .store
            .as_ref()
            .expect("reconciliation requires durable store");
        if let Err(error) = bounded(
            self.0.config.persistence_timeout,
            "confirmation",
            store.commit_timestamp_reconciliation(decision.clone()),
        )
        .await
        {
            let error = YappinError::FailureHandling {
                source: Box::new(report.to_error()),
                handling_error: Box::new(error),
            };
            self.fatal(error.to_remote());
            guard.armed = false;
            return Err(error);
        }
        let mut state = self.0.state.lock().unwrap();
        let claim = state.claims.get_mut(&id).unwrap();
        claim.reconciliation = Some(decision.clone());
        claim.conflict = false;
        claim.pending_since = None;
        claim.polling_verified = true;
        for report in state.conflicts.iter_mut().filter(|r| decision.covers(r)) {
            report.disposition = ConflictDisposition::Reconciled;
        }
        guard.armed = false;
        drop(state);
        self.0.changed.notify_one();
        Ok(true)
    }
}
