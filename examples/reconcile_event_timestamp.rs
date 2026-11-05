//! Explicit maintenance for a consumer-owned JSON ingress journal (Unix).
//! Run with --help. The journal must already be the stopped consumer's durable
//! source of truth, not an edited/exported copy of another database. No handlers
//! run here. Consumers using a database should implement the same atomic hook
//! in their own transaction; do not copy this journal over their database.
#[cfg(all(test, unix))]
#[path = "../tests/support/mod.rs"]
mod support;

#[cfg(unix)]
mod consumer {
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
    }
    #[derive(Serialize, Deserialize)]
    struct AuditEntry {
        decision: TimestampReconciliation,
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
    struct Journal(Arc<Inner>);
    impl Journal {
        fn open(path: PathBuf) -> Result<Self> {
            if !path.is_file() {
                return Err(YappinError::Config(
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
                    .map_err(|_| YappinError::Config("journal lock poisoned".into()))?;
                if fs::metadata(&journal.path)?.len() > MAX_JOURNAL_BYTES {
                    return Err(YappinError::Config(
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
            .map_err(|_| {
                YappinError::Config("journal task interrupted; inspect durable state".into())
            })?
        }
    }
    fn persist(path: &Path, data: &Data) -> Result<()> {
        let bytes = serde_json::to_vec(data)?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(YappinError::Config("journal size bound exceeded".into()));
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
                    return Err(YappinError::Config("journal bot scope mismatch".into()));
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
        async fn checkpoint(&self, _bot: Uuid, cursor: u64) -> Result<()> {
            self.transaction(true, move |data| {
                data.ingress.polling_checkpoint = Some(cursor);
                data.ingress.records.retain(|r| r.event.id > cursor);
                data.ingress.conflicts.retain(|r| r.event_id > cursor);
                data.ingress
                    .timestamp_reconciliations
                    .retain(|r| r.event_id > cursor);
                Ok(())
            })
            .await
        }
    }
    pub async fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() == 1 && args[0] == "--help" {
            println!("YAPIN_BOT_TOKEN=... cargo run --example reconcile_event_timestamp -- JOURNAL BOT_UUID EVENT_ID RETAINED_SHA256 POLLING_SHA256 [--approve AUDIT_ID]\nWithout --approve, only fresh GET /me, GET /events and journal reads occur. Hold the same exclusive lease as the stopped consumer. Journal format: {{\"ingress\": <IngressRecovery>, \"audit\": []}}. No source of truth is created or rewritten from an export.");
            return Ok(());
        }
        if !(args.len() == 5 || args.len() == 7 && args[5] == "--approve") {
            return Err(YappinError::Config("use --help for arguments".into()));
        }
        let bot = args[1]
            .parse()
            .map_err(|_| YappinError::Config("invalid bot UUID".into()))?;
        let event = args[2]
            .parse()
            .map_err(|_| YappinError::Config("invalid event ID".into()))?;
        let token = std::env::var("YAPIN_BOT_TOKEN")
            .map_err(|_| YappinError::Config("set YAPIN_BOT_TOKEN".into()))?;
        let store = Arc::new(Journal::open(PathBuf::from(&args[0]))?);
        let prepared = prepare_timestamp_reconciliation(
            &Client::new(token),
            store,
            TimestampReconciliationRequest::new(bot, event, &args[3], &args[4]),
            Default::default(),
        )
        .await?;
        println!("{}", serde_json::to_string_pretty(prepared.evidence())?);
        if args.len() == 7 {
            // This flag stands for the consumer's approval policy, not SDK authority.
            let decision = prepared.approve(OperationId::new(&args[6])?).await?;
            println!("Approved {}. Resume the consumer with the SAME journal adapter; normal matching polling must confirm before checkpoint release.",decision.decision_id);
        }
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[tokio::test]
        async fn timestamp_decision_roundtrip_archives_original_receipt() {
            use serde_json::json;
            use sha2::{Digest, Sha256};
            fn hash(event: &Event) -> String {
                fn sort(value: serde_json::Value) -> serde_json::Value {
                    match value {
                        serde_json::Value::Object(map) => map
                            .into_iter()
                            .map(|(k, v)| (k, sort(v)))
                            .collect::<std::collections::BTreeMap<_, _>>()
                            .into_iter()
                            .collect(),
                        serde_json::Value::Array(a) => {
                            serde_json::Value::Array(a.into_iter().map(sort).collect())
                        }
                        other => other,
                    }
                }
                Sha256::digest(
                    serde_json::to_vec(&sort(serde_json::to_value(event).unwrap())).unwrap(),
                )
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
            }
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
            let retained_hash = hash(&original);
            let incoming_hash = hash(&incoming);
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
            let dir =
                std::env::temp_dir().join(format!("rappin-reconciliation-{}", Uuid::new_v4()));
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
                    Err::<(), _>(YappinError::Config("rollback fixture".into()))
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
}
#[cfg(unix)]
#[tokio::main]
async fn main() -> rappin::Result<()> {
    consumer::run().await
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The SDK API is portable; this example's fsync/rename journal adapter requires Unix. Use a transactional database adapter on other systems.");
}
