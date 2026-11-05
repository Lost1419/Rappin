//! Opt-in shared admission and durable receipt contract.
use super::{BotHandle, EventProcessingFailure};
use crate::{Event, RemoteError, Result, YappinError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch, Notify};
use uuid::Uuid;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct IngressConfig {
    pub max_claims: usize,
    pub max_event_bytes: usize,
    pub persistence_timeout: Duration,
}
impl Default for IngressConfig {
    fn default() -> Self {
        Self {
            max_claims: 4096,
            max_event_bytes: 1024 * 1024,
            persistence_timeout: Duration::from_secs(2),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReceiptStage {
    Admitting,
    Queued,
    Started,
    Completed,
    Failed,
    Unprocessed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AdmissionDurability {
    Volatile,
    Durable,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReceiptStatus {
    pub event_id: u64,
    pub stage: ReceiptStage,
    pub durability: AdmissionDurability,
    pub committed: bool,
    pub payload_retained: bool,
    /// The event has been published to the bounded executor queue.
    pub queued_for_execution: bool,
    pub error: Option<RemoteError>,
}
/// A per-event receipt, independent of the HTTP admission response.
#[derive(Clone)]
pub struct EventReceipt(watch::Receiver<ReceiptStatus>);
impl EventReceipt {
    pub fn status(&self) -> ReceiptStatus {
        self.0.borrow().clone()
    }
    pub async fn wait(&mut self) -> ReceiptStatus {
        loop {
            let status = self.status();
            if matches!(
                status.stage,
                ReceiptStage::Completed | ReceiptStage::Failed | ReceiptStage::Unprocessed
            ) {
                return status;
            }
            if self.0.changed().await.is_err() {
                return self.status();
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdmissionStatus {
    Accepted,
    Duplicate,
    Full,
    Stopped,
    Error,
}
#[must_use]
pub struct Admission {
    pub status: AdmissionStatus,
    pub receipt: Option<EventReceipt>,
    pub error: Option<YappinError>,
}
impl Admission {
    fn simple(status: AdmissionStatus) -> Self {
        Self {
            status,
            receipt: None,
            error: None,
        }
    }
}
/// Persist the event and each state transition before acknowledging it. `Started`
/// must be durable before any collector/ack/handler effects. One active runner per
/// bot/store is required; this trait is not a distributed execution lease.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IngressRecord {
    pub bot_id: Uuid,
    pub event: Event,
    pub status: ReceiptStatus,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IngressRecovery {
    pub polling_checkpoint: Option<u64>,
    pub records: Vec<IngressRecord>,
}
#[async_trait::async_trait]
pub trait IngressStore: Send + Sync + 'static {
    async fn load(&self, bot_id: Uuid) -> Result<IngressRecovery>;
    async fn record(&self, record: IngressRecord) -> Result<()>;
    /// Atomically persist an authoritative polling checkpoint. Pruning records
    /// at/below it is permitted only once this checkpoint is durable.
    async fn checkpoint(&self, bot_id: Uuid, cursor: u64) -> Result<()>;
}
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct IngressStatus {
    pub running: bool,
    pub retained: usize,
    pub accepted: u64,
    pub duplicates: u64,
    pub conflicts: u64,
    pub full: u64,
    pub polling_watermark: u64,
    pub committed_cursor: u64,
    pub completed_uncommitted: usize,
    pub unresolved: usize,
}
struct Claim {
    fingerprint: [u8; 32],
    status: watch::Sender<ReceiptStatus>,
}
struct State {
    bot_id: Option<Uuid>,
    sender: Option<mpsc::Sender<Result<Batch>>>,
    stopped: bool,
    claims: BTreeMap<u64, Claim>,
    recovery: Vec<ClaimedEvent>,
    runtime: Option<(Arc<crate::Client>, Option<Duration>)>,
    status: IngressStatus,
    fatal: Option<RemoteError>,
}
struct Inner {
    config: IngressConfig,
    store: Option<Arc<dyn IngressStore>>,
    state: Mutex<State>,
    changed: Notify,
    checkpoint_fence: tokio::sync::Mutex<()>,
}
/// A cloneable ingress for the running pipeline. Admission is bounded and never
/// waits for execution. Without an IngressStore every receipt is volatile.
#[derive(Clone)]
pub struct EventIngress(Arc<Inner>);
pub(super) struct ClaimedEvent {
    pub event: Event,
    pub owner: bool,
    pub receipt: EventReceipt,
    pub prepared: Option<Box<crate::InteractionEvent>>,
}
pub(super) struct Batch {
    pub watermark: Option<u64>,
    pub events: Vec<ClaimedEvent>,
}
impl EventIngress {
    pub(super) fn new(config: IngressConfig, store: Option<Arc<dyn IngressStore>>) -> Self {
        Self(Arc::new(Inner {
            config,
            store,
            state: Mutex::new(State {
                bot_id: None,
                sender: None,
                stopped: false,
                claims: BTreeMap::new(),
                recovery: vec![],
                runtime: None,
                status: Default::default(),
                fatal: None,
            }),
            changed: Notify::new(),
            checkpoint_fence: tokio::sync::Mutex::new(()),
        }))
    }
    pub(super) fn validate(&self) -> Result<()> {
        let c = &self.0.config;
        if !(2..=1_000_000).contains(&c.max_claims)
            || c.max_event_bytes == 0
            || c.max_event_bytes > 16 * 1024 * 1024
            || c.persistence_timeout.is_zero()
            || c.persistence_timeout > Duration::from_secs(120)
        {
            return Err(YappinError::validation(
                "ingress.config",
                "invalid claim, event-size or persistence-timeout bound",
            ));
        }
        Ok(())
    }
    pub fn status(&self) -> IngressStatus {
        let s = self.0.state.lock().unwrap();
        let mut result = s.status.clone();
        result.running = s.sender.is_some() && !s.stopped;
        result.retained = s.claims.len();
        result.completed_uncommitted = s
            .claims
            .values()
            .filter(|c| {
                let r = c.status.borrow();
                r.stage == ReceiptStage::Completed && !r.committed
            })
            .count();
        result.unresolved = s
            .claims
            .values()
            .filter(|c| !matches!(c.status.borrow().stage, ReceiptStage::Completed))
            .count();
        result
    }
    pub async fn wait_ready(&self) -> Result<()> {
        loop {
            let changed = self.0.changed.notified();
            {
                let s = self.0.state.lock().unwrap();
                if s.stopped {
                    return Err(YappinError::Config("event ingress stopped".into()));
                }
                if s.sender.is_some() {
                    return Ok(());
                }
            }
            changed.await;
        }
    }
    pub async fn admit(&self, verified: crate::webhook::VerifiedEvent) -> Admission {
        let sender = {
            let s = self.0.state.lock().unwrap();
            if s.stopped || s.sender.is_none() {
                return Admission::simple(AdmissionStatus::Stopped);
            }
            if s.bot_id != Some(verified.bot_id) {
                return Admission {
                    status: AdmissionStatus::Error,
                    receipt: None,
                    error: Some(YappinError::validation(
                        "ingress.bot_id",
                        "event belongs to another authenticated bot",
                    )),
                };
            }
            s.sender.as_ref().unwrap().clone()
        };
        // Duplicates remain observable even when admission capacity is full.
        match self.existing(&verified.event) {
            Ok(Some(receipt)) => {
                return Admission {
                    status: AdmissionStatus::Duplicate,
                    receipt: Some(receipt),
                    error: None,
                }
            }
            Err(error) => {
                return Admission {
                    status: AdmissionStatus::Error,
                    receipt: None,
                    error: Some(error),
                }
            }
            _ => (),
        }
        let permit = match sender.try_reserve_owned() {
            Ok(permit) => permit,
            Err(error) => {
                return Admission::simple(
                    if matches!(error, mpsc::error::TrySendError::Closed(_)) {
                        AdmissionStatus::Stopped
                    } else {
                        self.full();
                        AdmissionStatus::Full
                    },
                )
            }
        };
        let event_id = verified.event.id;
        let claim = tokio::time::timeout(
            self.0
                .config
                .persistence_timeout
                .min(Duration::from_secs(2)),
            super::failure::catch(self.claim(verified.event)),
        )
        .await
        .unwrap_or_else(|_| {
            Err(YappinError::HandlerInterrupted {
                reason: "event admission timed out; inspect its receipt before resubmitting".into(),
                source: None,
            })
        });
        match claim {
            Ok(Some(claimed)) => {
                let receipt = claimed.receipt.clone();
                if claimed.owner {
                    if self.0.state.lock().unwrap().stopped {
                        self.interrupted(claimed.event.id, None);
                        return Admission::simple(AdmissionStatus::Stopped);
                    }
                    let id = claimed.event.id;
                    permit.send(Ok(Batch {
                        watermark: None,
                        events: vec![claimed],
                    }));
                    self.publish(&[id]);
                    Admission {
                        status: AdmissionStatus::Accepted,
                        receipt: Some(receipt),
                        error: None,
                    }
                } else {
                    Admission {
                        status: AdmissionStatus::Duplicate,
                        receipt: Some(receipt),
                        error: None,
                    }
                }
            }
            Ok(None) => Admission::simple(AdmissionStatus::Full),
            Err(error) => {
                permit.send(Err(error.to_remote().into()));
                Admission {
                    status: AdmissionStatus::Error,
                    receipt: self.receipt(event_id),
                    error: Some(error),
                }
            }
        }
    }
    pub fn receipt(&self, event_id: u64) -> Option<EventReceipt> {
        self.0
            .state
            .lock()
            .unwrap()
            .claims
            .get(&event_id)
            .map(|claim| EventReceipt(claim.status.subscribe()))
    }
    fn fail_unpublished(&self, id: u64) {
        let mut state = self.0.state.lock().unwrap();
        let error = state.claims.get(&id).and_then(|claim| {
            let status = claim.status.borrow();
            (status.stage == ReceiptStage::Failed)
                .then(|| status.error.clone())
                .flatten()
        });
        if !state.stopped && state.fatal.is_none() {
            state.fatal = error;
        }
        drop(state);
        self.0.changed.notify_waiters();
    }
    pub(super) async fn failure(&self) -> YappinError {
        loop {
            let changed = self.0.changed.notified();
            if let Some(error) = self.0.state.lock().unwrap().fatal.take() {
                return error.into();
            }
            changed.await;
        }
    }
    fn full(&self) {
        let mut s = self.0.state.lock().unwrap();
        s.status.full = s.status.full.saturating_add(1);
    }
    fn fingerprint(&self, event: &Event) -> Result<[u8; 32]> {
        if event.id == 0 {
            return Err(YappinError::validation(
                "ingress.event_id",
                "must be nonzero",
            ));
        }
        let bytes = serde_json::to_vec(event)?;
        if bytes.len() > self.0.config.max_event_bytes {
            return Err(YappinError::validation(
                "ingress.event",
                "event exceeds configured size bound",
            ));
        }
        Ok(Sha256::digest(bytes).into())
    }
    fn existing(&self, event: &Event) -> Result<Option<EventReceipt>> {
        let hash = self.fingerprint(event)?;
        let mut s = self.0.state.lock().unwrap();
        if let Some(claim) = s.claims.get(&event.id) {
            if claim.fingerprint != hash {
                s.status.conflicts = s.status.conflicts.saturating_add(1);
                return Err(YappinError::validation(
                    "ingress.payload",
                    "conflicting payload for the same bot/event ID",
                ));
            }
            let receipt = EventReceipt(claim.status.subscribe());
            s.status.duplicates = s.status.duplicates.saturating_add(1);
            return Ok(Some(receipt));
        }
        if event.id <= s.status.committed_cursor {
            s.status.duplicates = s.status.duplicates.saturating_add(1);
            return Ok(Some(EventReceipt(
                watch::channel(ReceiptStatus {
                    event_id: event.id,
                    stage: ReceiptStage::Completed,
                    durability: if self.0.store.is_some() {
                        AdmissionDurability::Durable
                    } else {
                        AdmissionDurability::Volatile
                    },
                    committed: true,
                    payload_retained: false,
                    queued_for_execution: true,
                    error: None,
                })
                .1,
            )));
        }
        Ok(None)
    }
    pub(super) async fn claim(&self, event: Event) -> Result<Option<ClaimedEvent>> {
        let hash = self.fingerprint(&event)?;
        let fence = self.0.checkpoint_fence.lock().await;
        let (owner, mut receipt) = {
            let mut s = self.0.state.lock().unwrap();
            if let Some(claim) = s.claims.get(&event.id) {
                if claim.fingerprint != hash {
                    s.status.conflicts = s.status.conflicts.saturating_add(1);
                    return Err(YappinError::validation(
                        "ingress.payload",
                        "conflicting concurrent event payload",
                    ));
                }
                let receipt = EventReceipt(claim.status.subscribe());
                s.status.duplicates = s.status.duplicates.saturating_add(1);
                (false, receipt)
            } else if event.id <= s.status.committed_cursor {
                s.status.duplicates = s.status.duplicates.saturating_add(1);
                (
                    false,
                    EventReceipt(
                        watch::channel(ReceiptStatus {
                            event_id: event.id,
                            stage: ReceiptStage::Completed,
                            durability: if self.0.store.is_some() {
                                AdmissionDurability::Durable
                            } else {
                                AdmissionDurability::Volatile
                            },
                            committed: true,
                            payload_retained: false,
                            queued_for_execution: true,
                            error: None,
                        })
                        .1,
                    ),
                )
            } else {
                if s.claims.len() >= self.0.config.max_claims {
                    s.status.full = s.status.full.saturating_add(1);
                    return Ok(None);
                }
                let status = ReceiptStatus {
                    event_id: event.id,
                    stage: ReceiptStage::Admitting,
                    durability: AdmissionDurability::Volatile,
                    committed: false,
                    payload_retained: true,
                    queued_for_execution: false,
                    error: None,
                };
                let (tx, rx) = watch::channel(status);
                s.claims.insert(
                    event.id,
                    Claim {
                        fingerprint: hash,
                        status: tx,
                    },
                );
                (true, EventReceipt(rx))
            }
        };
        drop(fence);
        if !owner {
            while !receipt.status().queued_for_execution
                && !matches!(
                    receipt.status().stage,
                    ReceiptStage::Completed | ReceiptStage::Failed | ReceiptStage::Unprocessed
                )
            {
                if receipt.0.changed().await.is_err() {
                    break;
                }
            }
            return Ok(Some(ClaimedEvent {
                event,
                owner: false,
                receipt,
                prepared: None,
            }));
        }
        let mut guard = ClaimGuard {
            ingress: self.clone(),
            id: event.id,
            armed: true,
        };
        self.transition(&event, ReceiptStage::Queued, None).await?;
        let preparation = async {
            self.transition(&event, ReceiptStage::Started, None).await?;
            self.prepare_owner(&event).await
        }
        .await;
        let prepared = match preparation {
            Ok(prepared) => prepared,
            Err(error) => {
                self.interrupted(event.id, Some(error.to_remote()));
                guard.armed = false;
                return Err(error);
            }
        };
        guard.armed = false;
        {
            let mut s = self.0.state.lock().unwrap();
            s.status.accepted = s.status.accepted.saturating_add(1);
        }
        Ok(Some(ClaimedEvent {
            event,
            owner: true,
            receipt,
            prepared,
        }))
    }
    async fn persist<T>(
        &self,
        phase: &str,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let result = tokio::time::timeout(self.0.config.persistence_timeout, future)
            .await
            .unwrap_or_else(|_| Err(YappinError::Config("ingress persistence timed out".into())));
        result.map_err(|cause| {
            YappinError::Remote(RemoteError {
                kind: "ingress_storage".into(),
                message: "durable ingress receipt operation failed".into(),
                status: cause.status_code(),
                retry_after: cause.retry_after().map(|d| d.as_secs_f64()),
                details: BTreeMap::from([
                    ("phase".into(), phase.into()),
                    ("source".into(), cause.to_json()),
                    ("retry_safety".into(), "do_not_replay_workflow".into()),
                    ("outcome".into(), "incomplete".into()),
                ]),
            })
        })
    }
    pub(super) async fn transition(
        &self,
        event: &Event,
        stage: ReceiptStage,
        error: Option<RemoteError>,
    ) -> Result<()> {
        let (bot_id, mut status) = {
            let s = self.0.state.lock().unwrap();
            let status = s
                .claims
                .get(&event.id)
                .ok_or_else(|| YappinError::Config("ingress claim missing".into()))?
                .status
                .borrow()
                .clone();
            (
                s.bot_id
                    .ok_or_else(|| YappinError::Config("ingress identity unavailable".into()))?,
                status,
            )
        };
        status.stage = stage;
        status.error = error;
        if let Some(store) = &self.0.store {
            status.durability = AdmissionDurability::Durable;
            self.persist(
                &format!("{stage:?}").to_lowercase(),
                store.record(IngressRecord {
                    bot_id,
                    event: event.clone(),
                    status: status.clone(),
                }),
            )
            .await?;
        }
        // Publish queue ownership only after the owner has entered the shared
        // channel. Internal duplicates must not overtake an unpublished owner.
        if stage == ReceiptStage::Queued {
            status.stage = ReceiptStage::Admitting;
        }
        self.set_status(event.id, status);
        Ok(())
    }
    fn set_status(&self, id: u64, status: ReceiptStatus) {
        if let Some(claim) = self.0.state.lock().unwrap().claims.get(&id) {
            claim.status.send_replace(status);
        }
    }
    pub(super) fn interrupted(&self, id: u64, error: Option<RemoteError>) {
        let s = self.0.state.lock().unwrap();
        if let Some(claim) = s.claims.get(&id) {
            let mut status = claim.status.borrow().clone();
            // Cancellation/drain paths can observe the same claim more than
            // once. Never erase a completion or downgrade an uncertain failure.
            if matches!(status.stage, ReceiptStage::Completed | ReceiptStage::Failed) {
                return;
            }
            status.stage = if status.stage == ReceiptStage::Started {
                ReceiptStage::Failed
            } else {
                ReceiptStage::Unprocessed
            };
            status.error = error.or_else(|| {
                (status.stage == ReceiptStage::Failed).then(|| {
                    YappinError::HandlerInterrupted {
                        reason:
                            "event effects may have started before queue publication or completion"
                                .into(),
                        source: None,
                    }
                    .to_remote()
                })
            });
            claim.status.send_replace(status);
        }
    }
    pub(super) async fn restore(
        &self,
        bot: Uuid,
        initial_checkpoint: Option<u64>,
    ) -> Result<Option<u64>> {
        if bot.is_nil() {
            return Err(YappinError::validation(
                "ingress.bot_id",
                "invalid authenticated bot identity",
            ));
        }
        self.0.state.lock().unwrap().bot_id = Some(bot);
        let Some(store) = &self.0.store else {
            return Ok(None);
        };
        let mut recovery = self.persist("load", store.load(bot)).await?;
        if recovery.polling_checkpoint.is_none() {
            if !recovery.records.is_empty() {
                return Err(YappinError::validation(
                    "ingress.recovery",
                    "stored receipts require their authoritative polling checkpoint",
                ));
            }
            let baseline = initial_checkpoint.unwrap_or(0);
            self.persist("initialize_checkpoint", store.checkpoint(bot, baseline))
                .await?;
            recovery.polling_checkpoint = Some(baseline);
        }
        if recovery.records.len() > self.0.config.max_claims {
            return Err(YappinError::validation(
                "ingress.recovery",
                "stored claims exceed configured bound",
            ));
        }
        let mut pending = vec![];
        let mut uncertain = vec![];
        for record in recovery.records {
            if record.bot_id != bot || record.status.event_id != record.event.id {
                return Err(YappinError::validation(
                    "ingress.recovery",
                    "stored identity mismatch",
                ));
            }
            let hash = self.fingerprint(&record.event)?;
            let id = record.event.id;
            if recovery
                .polling_checkpoint
                .is_some_and(|cursor| id <= cursor)
            {
                if record.status.stage != ReceiptStage::Completed {
                    return Err(YappinError::validation(
                        "ingress.recovery",
                        "unresolved claim below durable checkpoint",
                    ));
                }
                continue;
            }
            let mut status = record.status;
            status.committed = false;
            status.durability = AdmissionDurability::Durable;
            let owner = matches!(
                status.stage,
                ReceiptStage::Queued | ReceiptStage::Admitting | ReceiptStage::Unprocessed
            );
            if !owner && status.stage != ReceiptStage::Completed {
                uncertain.push(id);
            }
            if owner {
                status.stage = ReceiptStage::Queued;
            }
            let (tx, rx) = watch::channel(status);
            let mut state = self.0.state.lock().unwrap();
            if state
                .claims
                .insert(
                    id,
                    Claim {
                        fingerprint: hash,
                        status: tx,
                    },
                )
                .is_some()
            {
                return Err(YappinError::validation(
                    "ingress.recovery",
                    "duplicate stored claim",
                ));
            }
            if owner {
                pending.push(ClaimedEvent {
                    event: record.event,
                    owner: true,
                    receipt: EventReceipt(rx),
                    prepared: None,
                });
            }
        }
        let mut state = self.0.state.lock().unwrap();
        state.recovery = pending;
        if let Some(cursor) = recovery.polling_checkpoint {
            state.status.committed_cursor = cursor;
        }
        if !uncertain.is_empty() {
            return Err(YappinError::Remote(RemoteError{kind:"ingress_recovery".into(),message:"stored events have uncertain or failed effects; reconcile receipts before restarting intake".into(),status:None,retry_after:None,details:BTreeMap::from([("event_ids".into(),serde_json::json!(uncertain)),("retry_safety".into(),"do_not_replay_workflow".into()),("outcome".into(),"unknown".into())])}));
        }
        Ok(recovery.polling_checkpoint)
    }
    pub(super) fn open(
        &self,
        sender: mpsc::Sender<Result<Batch>>,
        last: u64,
        client: Arc<crate::Client>,
        auto_ack: Option<Duration>,
    ) -> Result<Vec<ClaimedEvent>> {
        let mut s = self.0.state.lock().unwrap();
        if s.stopped {
            return Err(YappinError::Config(
                "event ingress stopped during startup".into(),
            ));
        }
        s.sender = Some(sender);
        s.runtime = Some((client, auto_ack));
        s.status.committed_cursor = last;
        s.status.polling_watermark = last;
        let recovery = std::mem::take(&mut s.recovery);
        drop(s);
        self.0.changed.notify_waiters();
        Ok(recovery)
    }
    pub(super) fn publish(&self, ids: &[u64]) {
        let s = self.0.state.lock().unwrap();
        for id in ids {
            if let Some(claim) = s.claims.get(id) {
                let mut status = claim.status.borrow().clone();
                status.queued_for_execution = true;
                claim.status.send_replace(status);
            }
        }
    }
    pub(super) fn cancel_unpublished(&self) {
        let ids: Vec<_> = {
            let s = self.0.state.lock().unwrap();
            s.claims
                .iter()
                .filter(|(_, c)| !c.status.borrow().queued_for_execution)
                .map(|(id, _)| *id)
                .collect()
        };
        for id in ids {
            self.interrupted(id, None);
        }
    }
    pub(super) async fn prepare_owner(
        &self,
        event: &Event,
    ) -> Result<Option<Box<crate::InteractionEvent>>> {
        let (client, auto_ack) = self
            .0
            .state
            .lock()
            .unwrap()
            .runtime
            .clone()
            .ok_or_else(|| YappinError::Config("ingress runtime unavailable".into()))?;
        let mut prepared = None;
        if event.kind == "interaction" {
            if let Ok(crate::EventData::Interaction(mut interaction)) = event.parse_data() {
                interaction.created_at = interaction.created_at.or(event.created_at);
                client.track_interaction(&interaction).await?;
                if let Some(after) = auto_ack {
                    client.auto_ack(interaction.clone(), after);
                }
                prepared = Some(Box::new(interaction));
            }
        }
        client.receive_event(event.clone()).await;
        Ok(prepared)
    }
    pub(super) fn close(&self) {
        let mut s = self.0.state.lock().unwrap();
        s.stopped = true;
        s.sender = None;
        drop(s);
        self.0.changed.notify_waiters();
    }
    pub(super) fn watermark(&self, last: u64) {
        let mut s = self.0.state.lock().unwrap();
        s.status.polling_watermark = s.status.polling_watermark.max(last);
    }
    pub(super) async fn checkpoint(
        &self,
        candidate: u64,
        cursor: Option<&crate::events::EventCursor>,
    ) -> Result<u64> {
        // A newly arriving claim cannot cross a persisted checkpoint while it
        // is being published. This lock never covers a handler or callback.
        let _fence = self.0.checkpoint_fence.lock().await;
        let (last, current, bot) = {
            let s = self.0.state.lock().unwrap();
            let first = s
                .claims
                .range(..=candidate)
                .find(|(_, c)| c.status.borrow().stage != ReceiptStage::Completed)
                .map(|(id, _)| *id);
            (
                first
                    .map_or(candidate, |id| candidate.min(id.saturating_sub(1)))
                    .max(s.status.committed_cursor),
                s.status.committed_cursor,
                s.bot_id.expect("authenticated ingress"),
            )
        };
        if last <= current {
            return Ok(current);
        }
        if let Some(cursor) = cursor {
            cursor.save(last).await?;
        }
        if let Some(store) = &self.0.store {
            self.persist("checkpoint", store.checkpoint(bot, last))
                .await?;
        }
        let mut s = self.0.state.lock().unwrap();
        s.status.committed_cursor = last;
        for (&id, claim) in &s.claims {
            if id <= last {
                let mut status = claim.status.borrow().clone();
                if status.stage == ReceiptStage::Completed {
                    status.committed = true;
                    status.payload_retained = false;
                    claim.status.send_replace(status);
                }
            }
        }
        s.claims
            .retain(|id, claim| *id > last || !claim.status.borrow().committed);
        Ok(last)
    }
    pub(super) fn abort_unfinished(&self) {
        let s = self.0.state.lock().unwrap();
        for claim in s.claims.values() {
            let mut status = claim.status.borrow().clone();
            match status.stage {
                ReceiptStage::Admitting | ReceiptStage::Queued => {
                    status.stage = ReceiptStage::Unprocessed
                }
                ReceiptStage::Started => {
                    status.stage = ReceiptStage::Failed;
                    status.error = Some(
                        YappinError::HandlerInterrupted {
                            reason: "event runner cancelled after effects may have started".into(),
                            source: None,
                        }
                        .to_remote(),
                    );
                }
                _ => continue,
            }
            claim.status.send_replace(status);
        }
    }
    pub(super) fn add_evidence(&self, report: &mut EventProcessingFailure) {
        let s = self.0.state.lock().unwrap();
        for (&id, claim) in &s.claims {
            let status = claim.status.borrow();
            match status.stage {
                ReceiptStage::Completed if !status.committed => {
                    report.completed_uncommitted_events.push(id)
                }
                ReceiptStage::Failed => {
                    if !report.failed_events.iter().any(|f| f.event_id == id) {
                        if let Some(error) = &status.error {
                            report.failed_events.push(super::EventFailure {
                                event_id: id,
                                error: error.clone(),
                            });
                        }
                    }
                }
                _ if !matches!(status.stage, ReceiptStage::Completed) => {
                    report.unprocessed_events.push(id)
                }
                _ => (),
            }
        }
    }
}
struct ClaimGuard {
    ingress: EventIngress,
    id: u64,
    armed: bool,
}
impl Drop for ClaimGuard {
    fn drop(&mut self) {
        if self.armed {
            self.ingress.interrupted(self.id, None);
            self.ingress.fail_unpublished(self.id);
        }
    }
}
impl BotHandle {
    pub fn ingress(&self) -> Option<EventIngress> {
        self.inner.ingress.clone()
    }
}
