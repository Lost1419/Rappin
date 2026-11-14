//! Shared atomic Unix journal adapter for the maintenance examples.

use rappin::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Default, Serialize, Deserialize)]
struct Data {
    ingress: IngressRecovery,
    /// Immutable append-only decisions survive checkpoint pruning.
    #[serde(default)]
    audit: Vec<AuditEntry>,
    #[serde(default)]
    retirement_audit: Vec<RetirementAuditEntry>,
}
#[derive(Serialize, Deserialize)]
struct AuditEntry {
    decision: TimestampReconciliation,
    original: IngressRecord,
}
#[derive(Serialize, Deserialize)]
struct RetirementAuditEntry {
    decision: EventRetirement,
    original: IngressRecord,
}
struct Inner {
    path: PathBuf,
    lease_path: PathBuf,
    _lease: File,
    gate: Mutex<()>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lease_path);
    }
}
#[derive(Clone)]
pub(crate) struct Journal(Arc<Inner>);
impl Journal {
    pub(crate) fn open(path: PathBuf) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::Config(
                "an existing consumer-owned journal is required".into(),
            ));
        }
        let lease_path = path.with_extension("reconciliation-lease");
        // The running consumer must acquire this SAME lease. A create-new
        // lease is deliberately fail-closed after a crash: an operator must
        // establish that no runner is alive before removing a stale lease.
        let lease = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&lease_path)?;
        Ok(Self(Arc::new(Inner {
            path,
            lease_path,
            _lease: lease,
            gate: Mutex::new(()),
        })))
    }
    async fn transaction<T: Send + 'static>(
        &self,
        write: bool,
        f: impl FnOnce(&mut Data) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let journal = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = journal
                .gate
                .lock()
                .map_err(|_| Error::Config("journal lock poisoned".into()))?;
            if fs::metadata(&journal.path)?.len() > MAX_JOURNAL_BYTES {
                return Err(Error::Config(
                    "journal size bound exceeded; archive audit with a transaction".into(),
                ));
            }
            // Always reload: a previous rename may have committed even when
            // its final sync/response failed. Never trust stale memory.
            let mut data: Data = serde_json::from_slice(&fs::read(&journal.path)?)?;
            let result = f(&mut data)?;
            if write {
                persist(&journal.path, &data)?;
            }
            Ok(result)
        })
        .await
        .map_err(|_| Error::Config("journal task interrupted; inspect durable state".into()))?
    }
}
fn persist(path: &Path, data: &Data) -> Result<()> {
    let bytes = serde_json::to_vec(data)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(Error::Config("journal size bound exceeded".into()));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".ingress-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}
#[async_trait::async_trait]
impl IngressStore for Journal {
    async fn load(&self, bot: Uuid) -> Result<IngressRecovery> {
        self.transaction(false, move |data| {
            if data.ingress.records.iter().any(|r| r.bot_id != bot)
                || data.ingress.conflicts.iter().any(|r| r.bot_id != bot)
            {
                return Err(Error::Config("journal bot scope mismatch".into()));
            }
            Ok(data.ingress.clone())
        })
        .await
    }
    async fn record(&self, record: IngressRecord) -> Result<()> {
        self.transaction(true, move |data| {
            data.ingress
                .records
                .retain(|r| r.event.id != record.event.id);
            data.ingress.records.push(record);
            Ok(())
        })
        .await
    }
    async fn record_conflict(&self, report: IngressConflict) -> Result<AdmissionDurability> {
        self.transaction(true, move |data| {
            data.ingress.conflicts.retain(|r| {
                !(r.event_id == report.event_id
                    && r.incoming_source == report.incoming_source
                    && r.incoming_fingerprint == report.incoming_fingerprint
                    && r.existing_fingerprint == report.existing_fingerprint)
            });
            data.ingress.conflicts.push(report);
            Ok(AdmissionDurability::Durable)
        })
        .await
    }
    async fn commit_timestamp_reconciliation(
        &self,
        decision: TimestampReconciliation,
    ) -> Result<()> {
        self.transaction(true, move |data| {
            // The validation and write share one serialized durable transaction.
            decision.validate_write(&data.ingress)?;
            data.ingress
                .timestamp_reconciliations
                .retain(|r| r.event_id != decision.event_id);
            data.ingress
                .timestamp_reconciliations
                .push(decision.clone());
            let original = data
                .ingress
                .records
                .iter()
                .find(|r| r.event.id == decision.event_id)
                .expect("validated retained receipt")
                .clone();
            data.audit.push(AuditEntry { decision, original });
            Ok(())
        })
        .await
    }
    async fn commit_event_retirement(&self, decision: EventRetirement) -> Result<()> {
        self.transaction(true, move |data| {
            decision.validate_write(&data.ingress)?;
            let original = data
                .ingress
                .records
                .iter()
                .find(|r| r.event.id == decision.event_id)
                .expect("validated original record")
                .clone();
            data.ingress
                .event_retirements
                .retain(|r| r.event_id != decision.event_id);
            data.ingress.event_retirements.push(decision.clone());
            data.retirement_audit
                .push(RetirementAuditEntry { decision, original });
            Ok(())
        })
        .await
    }
    async fn checkpoint(&self, _bot: Uuid, cursor: u64) -> Result<()> {
        self.transaction(true, move |data| {
            data.ingress.polling_checkpoint = Some(cursor);
            data.ingress.records.retain(|r| r.event.id > cursor);
            data.ingress.conflicts.retain(|r| r.event_id > cursor);
            data.ingress
                .timestamp_reconciliations
                .retain(|r| r.event_id > cursor);
            data.ingress
                .event_retirements
                .retain(|r| r.event_id > cursor);
            Ok(())
        })
        .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn timestamp_decision_roundtrip_archives_original_receipt() {
        use serde_json::json;
        let bot: Uuid = crate::support::profile()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let original: Event=serde_json::from_value(json!({"id":100,"type":"message","created_at":null,"data":{
                "id":Uuid::from_u128(1),"server_id":Uuid::from_u128(2),"channel_id":Uuid::from_u128(3),"author_id":Uuid::from_u128(4),"content":"fixture"
            }})).unwrap();
        let mut incoming = original.clone();
        incoming.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
        let retained_hash = canonical_event_fingerprint(&original).unwrap();
        let incoming_hash = canonical_event_fingerprint(&incoming).unwrap();
        let record: IngressRecord=serde_json::from_value(json!({"bot_id":bot,"event":original,"source":null,"status":{
                "event_id":100,"stage":"completed","durability":"durable","committed":false,"payload_retained":true,"queued_for_execution":true,"error":null
            }})).unwrap();
        let original_record = serde_json::to_value(&record).unwrap();
        let conflict=serde_json::from_value(json!({"bot_id":bot,"event_id":100,"existing_source":"recovery","recovered_source":null,
                "incoming_source":"polling","existing_fingerprint":retained_hash,"incoming_fingerprint":incoming_hash,"retained_stage":"completed",
                "disposition":"stop","differences":[{"path":"/created_at","existing_type":"null","incoming_type":"string"}],"differences_truncated":false
            })).unwrap();
        let mut data = Data::default();
        data.ingress.polling_checkpoint = Some(0);
        data.ingress.records.push(record);
        data.ingress.conflicts.push(conflict);
        let dir = std::env::temp_dir().join(format!("rappin-reconciliation-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("ingress.json");
        persist(&path, &data).unwrap();
        let store = Arc::new(Journal::open(path.clone()).unwrap());
        let (url, mut requests, server) = crate::support::mock(vec![
            crate::support::response(200, crate::support::profile()),
            crate::support::response(200, json!({"events":[incoming],"last":100})),
        ])
        .await;
        let client = Client::builder("fixture").base_url(url).build();
        let prepared = prepare_timestamp_reconciliation(
            &client,
            store.clone(),
            TimestampReconciliationRequest::new(bot, 100, retained_hash, incoming_hash),
            Default::default(),
        )
        .await
        .unwrap();
        let approved = prepared
            .approve(OperationId::new("fixture/approved").unwrap())
            .await
            .unwrap();
        server.await.unwrap();
        assert!(requests.recv().await.unwrap().starts_with("GET /me "));
        assert!(requests.recv().await.unwrap().starts_with("GET /events?"));
        assert!(requests.recv().await.is_none());
        drop(store);
        let store = Journal::open(path.clone()).unwrap();
        assert_eq!(
            store.load(bot).await.unwrap().timestamp_reconciliations[0].state,
            TimestampReconciliationState::Approved
        );
        let before = fs::read(&path).unwrap();
        let mut invalid = approved.clone();
        invalid.polling_fingerprint = "f".repeat(64);
        assert!(store
            .commit_timestamp_reconciliation(invalid)
            .await
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        // The core tests establish polling proof; here exercise the adapter's
        // durable transition and audit/pruning independently.
        let mut confirmed = approved;
        confirmed.state = TimestampReconciliationState::Confirmed;
        confirmed.confirmed_at = Some(chrono::Utc::now());
        store
            .commit_timestamp_reconciliation(confirmed)
            .await
            .unwrap();
        store.checkpoint(bot, 100).await.unwrap();
        drop(store);
        let archived: Data = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(archived.ingress.records.is_empty());
        assert!(archived.ingress.timestamp_reconciliations.is_empty());
        assert_eq!(archived.audit.len(), 2);
        for entry in archived.audit {
            assert_eq!(
                serde_json::to_value(entry.original).unwrap(),
                original_record
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn retirement_roundtrip_keeps_unknown_error_and_audit_after_checkpoint() {
        use serde_json::json;
        let bot: Uuid = crate::support::profile()["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let event: Event = serde_json::from_value(json!({"id":100,"type":"message","data":{
            "id":Uuid::from_u128(1),"server_id":Uuid::from_u128(2),"channel_id":Uuid::from_u128(3),"author_id":Uuid::from_u128(4),"content":"fixture"
        }})).unwrap();
        let error = Error::PartialWrite {
            completed: vec![json!({"receipt_id":"confirmed-step"})],
            failed_step: 2,
            source: Box::new(Error::Remote(RemoteError {
                kind: "gateway_request".into(),
                message: "response unknown".into(),
                status: Some(502),
                retry_after: Some(1.0),
                details: std::collections::BTreeMap::from([
                    ("submitted".into(), true.into()),
                    ("outcome".into(), "unknown".into()),
                ]),
            })),
        }
        .to_remote();
        let record: IngressRecord = serde_json::from_value(json!({"bot_id":bot,"event":event,"source":"gateway","status":{
            "event_id":100,"stage":"failed","durability":"durable","committed":false,"payload_retained":true,"queued_for_execution":true,"error":error
        }})).unwrap();
        let original = serde_json::to_value(&record).unwrap();
        let mut data = Data::default();
        data.ingress.polling_checkpoint = Some(90);
        data.ingress.records.push(record);
        let dir = std::env::temp_dir().join(format!("rappin-retirement-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("ingress.json");
        persist(&path, &data).unwrap();
        let store = Arc::new(Journal::open(path.clone()).unwrap());
        let (url, mut requests, server) = crate::support::mock(vec![
            crate::support::response(200, crate::support::profile()),
            crate::support::response(200, json!({"events":[event],"last":100})),
        ])
        .await;
        let client = Client::builder("fixture").base_url(url).build();
        let hash = canonical_event_fingerprint(&event).unwrap();
        let approved = prepare_event_retirement(
            &client,
            store.clone(),
            EventRetirementRequest::new(bot, 100, &hash, &hash),
            Default::default(),
        )
        .await
        .unwrap()
        .approve(
            OperationId::new("operator/decision-1").unwrap(),
            EventRetirementAssessment::AcceptUnknown,
            OperationId::new("operator/evidence-1").unwrap(),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(requests.recv().await.unwrap().starts_with("GET /me "));
        assert!(requests.recv().await.unwrap().starts_with("GET /events?"));
        assert!(requests.recv().await.is_none());
        drop(store);
        let store = Journal::open(path.clone()).unwrap();
        let restored = store.load(bot).await.unwrap();
        assert_eq!(
            restored.event_retirements[0].state,
            EventRetirementState::Approved
        );
        assert_eq!(
            serde_json::to_value(&restored.records[0]).unwrap(),
            original
        );
        let before = fs::read(&path).unwrap();
        let mut invalid = approved.clone();
        invalid.retained_fingerprint = "f".repeat(64);
        assert!(store.commit_event_retirement(invalid).await.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        // Core integration tests establish actual polling proof. Exercise this
        // adapter's atomic transition and archive independently here.
        let mut confirmed = approved;
        confirmed.state = EventRetirementState::PollingConfirmed;
        confirmed.polling_confirmed_at = Some(chrono::Utc::now());
        confirmed.confirmation_watermark = Some(100);
        confirmed.target_observed_at_confirmation = Some(true);
        store.commit_event_retirement(confirmed).await.unwrap();
        store.checkpoint(bot, 100).await.unwrap();
        drop(store);
        let archived: Data = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(archived.ingress.records.is_empty());
        assert!(archived.ingress.event_retirements.is_empty());
        assert_eq!(archived.retirement_audit.len(), 2);
        for entry in archived.retirement_audit {
            assert_eq!(serde_json::to_value(entry.original).unwrap(), original);
            assert_eq!(
                entry.decision.assessment,
                Some(EventRetirementAssessment::AcceptUnknown)
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn journal_checkpoint_roundtrip_lease_and_failed_transaction() {
        let dir = std::env::temp_dir().join(format!("rappin-journal-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("ingress.json");
        persist(&path, &Data::default()).unwrap();
        let store = Journal::open(path.clone()).unwrap();
        assert!(Journal::open(path.clone()).is_err());
        store.checkpoint(Uuid::nil(), 12).await.unwrap();
        let before = fs::read(&path).unwrap();
        assert!(store
            .transaction(true, |data| {
                data.ingress.polling_checkpoint = Some(99);
                Err::<(), _>(Error::Config("rollback fixture".into()))
            })
            .await
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(store);
        let store = Journal::open(path).unwrap();
        assert_eq!(
            store.load(Uuid::nil()).await.unwrap().polling_checkpoint,
            Some(12)
        );
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}
