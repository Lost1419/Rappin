mod support;
use hmac::{Hmac, KeyInit, Mac};
use rappin::*;
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot, Notify},
};
use uuid::Uuid;
fn bot_id() -> Uuid {
    "11111111-1111-4111-8111-111111111111".parse().unwrap()
}
fn event(id: u64, command: &str) -> Event {
    serde_json::from_value(json!({"id":id,"type":"message","data":{"id":Uuid::from_u128(id as u128),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::nil(),"command":command,"content":"fixture"}})).unwrap()
}
fn verified(event: Event) -> webhook::VerifiedEvent {
    let body = serde_json::to_vec(&event).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(&body);
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    webhook::verify_bot_event(
        "fixture-secret",
        bot_id(),
        &bot_id().to_string(),
        &format!("sha256={sig}"),
        &body,
    )
    .unwrap()
}
struct Http {
    url: String,
    polls: mpsc::UnboundedReceiver<oneshot::Sender<Value>>,
    posts: Arc<AtomicUsize>,
    poll_after: Arc<Mutex<Vec<u64>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Http {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Http {
    async fn new(responses: Vec<support::Response>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, polls) = mpsc::unbounded_channel();
        let posts = Arc::new(AtomicUsize::new(0));
        let count = posts.clone();
        let poll_after = Arc::new(Mutex::new(vec![]));
        let afters = poll_after.clone();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accept=listener.accept()=>{let (mut socket,_)=accept.unwrap();let tx=tx.clone();let count=count.clone();let responses=responses.clone();let afters=afters.clone();connections.spawn(async move {
                        let mut bytes=vec![];let mut buf=[0;4096];loop {let n=socket.read(&mut buf).await.unwrap();if n==0{return;}bytes.extend_from_slice(&buf[..n]);if let Some(end)=bytes.windows(4).position(|b|b==b"\r\n\r\n") {let headers=String::from_utf8_lossy(&bytes[..end]);let size=headers.lines().find_map(|l|l.split_once(':').filter(|(k,_)|k.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.trim().parse::<usize>().unwrap())).unwrap_or(0);if bytes.len()>=end+4+size {break;}}}
                        let request=String::from_utf8_lossy(&bytes);let line=request.lines().next().unwrap();
                        let (status,headers,body)=if line.starts_with("GET /me "){support::response(200,support::profile())}else if line.starts_with("GET /events?"){afters.lock().unwrap().push(line.split("after=").nth(1).unwrap().split(['&',' ']).next().unwrap().parse().unwrap());let (send,receive)=oneshot::channel();if tx.send(send).is_err(){return;}let mut closed=[0u8;1];let value=tokio::select!{result=receive=>{let Ok(value)=result else{return;};value},_=socket.read(&mut closed)=>return};support::response(200,value)}else if line.starts_with("POST "){count.fetch_add(1,Ordering::SeqCst);responses.lock().unwrap().pop_front().expect("unexpected mutation")}else{panic!("unexpected request {line}")};
                        let mut reply=format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",body.len());for (k,v) in headers {reply.push_str(&format!("{k}: {v}\r\n"));}reply.push_str("\r\n");reply.push_str(&body);let _=socket.write_all(reply.as_bytes()).await;
                    });},
                    _=connections.join_next(),if !connections.is_empty()=>(),
                }
            }
        });
        Self {
            url,
            polls,
            posts,
            poll_after,
            task,
        }
    }
    async fn next(&mut self) -> oneshot::Sender<Value> {
        loop {
            let sender = tokio::time::timeout(Duration::from_secs(3), self.polls.recv())
                .await
                .unwrap()
                .unwrap();
            if !sender.is_closed() {
                return sender;
            }
        }
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition stalled");
}
fn builder(http: &Http) -> BotBuilder {
    Bot::builder("fixture")
        .base_url(&http.url)
        .sync_commands(false)
        .log(false)
        .configure(|settings| settings.persistence.ingress = Some(IngressConfig::default()))
}

#[tokio::test]
async fn simultaneous_webhooks_polling_sparse_ids_conflicts_and_identity_are_shared() {
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(Mutex::new(vec![]));
    let c = calls.clone();
    let mut config = IngressConfig::default();
    config.conflict_policy = IngressConflictPolicy::RetainCompletedAndPoll;
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(config))
        .command("ping", "record", move |ctx| {
            let c = c.clone();
            async move {
                c.lock()
                    .unwrap()
                    .push(ctx.message().expect("message identity").event.id);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let first = http.next().await;
    let value = verified(event(100, "ping"));
    let (a, b) = tokio::join!(ingress.admit(value.clone()), ingress.admit(value));
    assert_eq!(
        [a.status, b.status]
            .iter()
            .filter(|&&s| s == AdmissionStatus::Accepted)
            .count(),
        1
    );
    let mut receipt = a.receipt.unwrap();
    assert_eq!(receipt.wait().await.stage, ReceiptStage::Completed);
    assert!(!receipt.status().committed);
    assert_eq!(receipt.status().durability, AdmissionDurability::Volatile);
    assert_eq!(
        ingress.admit(verified(event(100, "changed"))).await.status,
        AdmissionStatus::Error
    );
    let other = Uuid::new_v4();
    let bytes = serde_json::to_vec(&event(101, "ping")).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(&bytes);
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let wrong = webhook::verify_bot_event(
        "fixture-secret",
        other,
        &other.to_string(),
        &format!("sha256={sig}"),
        &bytes,
    )
    .unwrap();
    assert_eq!(ingress.admit(wrong).await.status, AdmissionStatus::Error);
    first
        .send(json!({"events":[event(20,"ping"),event(40,"ping"),event(40,"ping")],"last":50}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 50).await;
    assert!(!receipt.status().committed);
    assert_eq!(calls.lock().unwrap().len(), 3);
    let second = http.next().await;
    second
        .send(json!({"events":[event(100,"ping")],"last":100}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 100).await;
    assert!(receipt.status().committed);
    assert_eq!(calls.lock().unwrap().len(), 3);
    let retired = ingress.admit(verified(event(20, "ping"))).await;
    assert_eq!(retired.status, AdmissionStatus::Duplicate);
    assert!(!retired.receipt.unwrap().status().payload_retained);
    handle.stop();
    run.await.unwrap().unwrap();
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn saturation_retains_unresolved_claims_and_cancellation_drains_started_work() {
    let mut http = Http::new(vec![]).await;
    let start = Arc::new(AtomicUsize::new(0));
    let started = start.clone();
    let release = Arc::new(Notify::new());
    let wait = release.clone();
    let mut config = IngressConfig::default();
    config.max_claims = 4;
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(config))
        .configure(|settings| settings.execution.pending_batches = 2)
        .configure(|settings| settings.execution.concurrency = 1)
        .command("hold", "wait", move |_| {
            let started = started.clone();
            let wait = wait.clone();
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                wait.notified().await;
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let first = ingress.admit(verified(event(10, "hold"))).await;
    assert_eq!(first.status, AdmissionStatus::Accepted);
    until(|| start.load(Ordering::SeqCst) == 1).await;
    let mut full = false;
    for id in 11..20 {
        let admission = ingress.admit(verified(event(id, "hold"))).await;
        if admission.status == AdmissionStatus::Full {
            full = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(full);
    assert!(ingress.status().retained <= 4);
    assert_eq!(
        ingress.admit(verified(event(10, "hold"))).await.status,
        AdmissionStatus::Duplicate
    );
    handle.stop();
    release.notify_one();
    // Already-admitted executor work drains; release each sequential waiter.
    for _ in 0..8 {
        release.notify_one();
        tokio::task::yield_now().await;
    }
    let drained = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap();
    if let Err(Error::EventProcessing { report }) = drained {
        assert!(report
            .failed_events
            .iter()
            .all(|e| e.error.details["outcome"] == "unknown"));
    }
    assert_eq!(
        ingress.admit(verified(event(30, "hold"))).await.status,
        AdmissionStatus::Stopped
    );
}

#[derive(Default)]
struct Store {
    data: Mutex<IngressRecovery>,
    fail_checkpoint: AtomicUsize,
    block_queued: AtomicUsize,
    queued: Notify,
    release: Notify,
    fail_conflict: AtomicUsize,
    block_conflict: AtomicBool,
    conflict_waiting: Notify,
    release_conflict: Notify,
    fail_reconciliation: AtomicUsize,
    fail_retirement: AtomicUsize,
    retirement_writes: AtomicUsize,
    retirement_audit: Mutex<Vec<(EventRetirement, IngressRecord)>>,
    reconciliation_writes: AtomicUsize,
    block_confirmation: AtomicBool,
    confirmation_waiting: Notify,
    release_confirmation: Notify,
}
#[async_trait::async_trait]
impl IngressStore for Store {
    async fn load(&self, _: Uuid) -> Result<IngressRecovery> {
        Ok(self.data.lock().unwrap().clone())
    }
    async fn record(&self, record: IngressRecord) -> Result<()> {
        if record.status.stage == ReceiptStage::Queued
            && self.block_queued.load(Ordering::SeqCst) == record.event.id as usize
        {
            self.queued.notify_one();
            self.release.notified().await;
        }
        let mut data = self.data.lock().unwrap();
        data.records.retain(|r| r.event.id != record.event.id);
        data.records.push(record);
        Ok(())
    }
    async fn record_conflict(&self, report: IngressConflict) -> Result<AdmissionDurability> {
        if self.block_conflict.load(Ordering::SeqCst) {
            self.conflict_waiting.notify_one();
            self.release_conflict.notified().await;
        }
        let fail = self.fail_conflict.load(Ordering::SeqCst);
        if fail == 1 || fail == 2 && report.disposition == ConflictDisposition::PollingConfirmed {
            return Err(Error::Api {
                status: 503,
                message: "fixture conflict persistence failed".into(),
                retry_after: Some(0.1),
            });
        }
        let mut data = self.data.lock().unwrap();
        data.conflicts.retain(|r| {
            !(r.event_id == report.event_id
                && r.incoming_fingerprint == report.incoming_fingerprint
                && r.incoming_source == report.incoming_source)
        });
        data.conflicts.push(report);
        Ok(AdmissionDurability::Durable)
    }
    async fn commit_timestamp_reconciliation(
        &self,
        decision: TimestampReconciliation,
    ) -> Result<()> {
        self.reconciliation_writes.fetch_add(1, Ordering::SeqCst);
        if decision.state == TimestampReconciliationState::Confirmed
            && self.block_confirmation.load(Ordering::SeqCst)
        {
            self.confirmation_waiting.notify_one();
            self.release_confirmation.notified().await;
        }
        let mut data = self.data.lock().unwrap();
        decision.validate_write(&data)?;
        let fail = self.fail_reconciliation.load(Ordering::SeqCst);
        if fail == 1 || fail == 3 && decision.state == TimestampReconciliationState::Confirmed {
            return Err(Error::Api {
                status: 503,
                message: "fixture transaction rolled back".into(),
                retry_after: Some(0.5),
            });
        }
        data.timestamp_reconciliations
            .retain(|r| r.event_id != decision.event_id);
        data.timestamp_reconciliations.push(decision);
        if fail == 2
            || fail == 4
                && data.timestamp_reconciliations.last().unwrap().state
                    == TimestampReconciliationState::Confirmed
        {
            return Err(Error::Api {
                status: 503,
                message: "fixture commit response lost".into(),
                retry_after: Some(0.5),
            });
        }
        Ok(())
    }
    async fn commit_event_retirement(&self, decision: EventRetirement) -> Result<()> {
        self.retirement_writes.fetch_add(1, Ordering::SeqCst);
        let mut data = self.data.lock().unwrap();
        decision.validate_write(&data)?;
        let fail = self.fail_retirement.load(Ordering::SeqCst);
        if fail == 1 || fail == 3 && decision.state == EventRetirementState::PollingConfirmed {
            return Err(Error::Api {
                status: 503,
                message: "fixture retirement transaction rollback".into(),
                retry_after: Some(0.2),
            });
        }
        let original = data
            .records
            .iter()
            .find(|r| r.event.id == decision.event_id)
            .unwrap()
            .clone();
        data.event_retirements
            .retain(|r| r.event_id != decision.event_id);
        data.event_retirements.push(decision.clone());
        self.retirement_audit
            .lock()
            .unwrap()
            .push((decision.clone(), original));
        if fail == 2 || fail == 4 && decision.state == EventRetirementState::PollingConfirmed {
            return Err(Error::Api {
                status: 503,
                message: "fixture retirement commit response lost".into(),
                retry_after: Some(0.2),
            });
        }
        Ok(())
    }
    async fn checkpoint(&self, _: Uuid, cursor: u64) -> Result<()> {
        if cursor > 0 && self.fail_checkpoint.load(Ordering::SeqCst) > 0 {
            return Err(Error::Config(
                "fixture checkpoint persistence failed".into(),
            ));
        }
        let mut data = self.data.lock().unwrap();
        data.polling_checkpoint = Some(cursor);
        data.records.retain(|r| r.event.id > cursor);
        let retired = data
            .timestamp_reconciliations
            .iter()
            .filter(|r| r.event_id <= cursor)
            .map(|r| r.event_id)
            .collect::<Vec<_>>();
        let mut retired = retired;
        retired.extend(
            data.event_retirements
                .iter()
                .filter(|r| r.event_id <= cursor)
                .map(|r| r.event_id),
        );
        data.event_retirements.retain(|r| r.event_id > cursor);
        data.conflicts.retain(|r| !retired.contains(&r.event_id));
        data.timestamp_reconciliations
            .retain(|r| r.event_id > cursor);
        Ok(())
    }
}
#[tokio::test]
async fn unpublished_lower_claim_blocks_checkpoint_and_recovers_without_duplicate_effects() {
    let mut http = Http::new(vec![]).await;
    let store = Arc::new(Store::default());
    store.block_queued.store(40, Ordering::SeqCst);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .configure(|settings| settings.execution.pending_batches = 3)
        .command("ping", "test", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    let input = ingress.clone();
    let admission = tokio::spawn(async move { input.admit(verified(event(40, "ping"))).await });
    store.queued.notified().await;
    poll.send(json!({"events":[event(20,"ping")],"last":50}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 39).await;
    store.release.notify_one();
    let mut receipt = admission.await.unwrap().receipt.unwrap();
    assert_eq!(receipt.wait().await.stage, ReceiptStage::Completed);
    until(|| ingress.status().committed_cursor == 50).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(receipt.status().durability, AdmissionDurability::Durable);
    handle.stop();
    run.await.unwrap().unwrap();
}

#[tokio::test]
async fn failed_checkpoint_keeps_durable_completion_and_restart_never_reexecutes_it() {
    let root = std::env::temp_dir().join(format!("rappin-ingress-mirror-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("cursor.json");
    EventCursor::new(&path).save(0).await.unwrap();
    let store = Arc::new(Store::default());
    store.fail_checkpoint.store(1, Ordering::SeqCst);
    let calls = Arc::new(AtomicUsize::new(0));
    for round in 0..2 {
        let mut http = Http::new(vec![]).await;
        let c = calls.clone();
        let bot = builder(&http)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&path))
            })
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "test", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let ingress = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        http.next()
            .await
            .send(json!({"events":[event(10,"ping")],"last":10}))
            .unwrap();
        if round == 0 {
            let error = run.await.unwrap().unwrap_err();
            let report = match error {
                Error::EventProcessing { report } => report,
                other => panic!("{other:?}"),
            };
            assert!(report.cursor_error.is_some());
            assert_eq!(report.committed_cursor, 0);
            assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(10));
            assert_eq!(report.completed_uncommitted_events, [10]);
            store.fail_checkpoint.store(0, Ordering::SeqCst);
        } else {
            until(|| ingress.status().committed_cursor == 10).await;
            handle.stop();
            run.await.unwrap().unwrap();
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn collectors_keep_receiving_with_one_handler_and_duplicate_buttons_ack_once() {
    let mut http = Http::new(vec![support::response(200, json!({}))]).await;
    let ready = Arc::new(Notify::new());
    let r = ready.clone();
    let buttons = Arc::new(AtomicUsize::new(0));
    let b = buttons.clone();
    let bot = builder(&http)
        .configure(|settings| settings.execution.pending_batches = 1)
        .configure(|settings| settings.execution.concurrency = 1)
        .configure(|settings| settings.execution.auto_ack_after = Some(Duration::from_millis(5)))
        .command("question", "collect", move |ctx| {
            let r = r.clone();
            async move {
                let collector = ctx
                    .message()
                    .expect("message collector")
                    .replies()
                    .filter(|message| message.content.as_deref() == Some("answer"))
                    .timeout(Duration::from_secs(2))
                    .subscribe()?;
                r.notify_one();
                let message = collector.wait().await?;
                assert_eq!(message.id, Uuid::from_u128(11));
                Ok(())
            }
        })
        .button("ack:", move |ctx| {
            let b = b.clone();
            async move {
                b.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                ctx.ack().await
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    // The polling read reserves the one queue slot; use that read for the question.
    http.next()
        .await
        .send(json!({"events":[event(10,"question")],"last":10}))
        .unwrap();
    ready.notified().await;
    let mut answer = event(11, "unused");
    answer.data.as_object_mut().unwrap().remove("command");
    answer.data["content"] = json!("answer");
    http.next()
        .await
        .send(json!({"events":[answer.clone(),answer],"last":11}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 11).await;
    let interaction:Event=serde_json::from_value(json!({"id":12,"type":"interaction","data":{"id":Uuid::from_u128(12),"token":"interaction_12","custom_id":"ack:test","channel_id":Uuid::nil(),"user_id":Uuid::nil()}})).unwrap();
    http.next()
        .await
        .send(json!({"events":[interaction.clone(),interaction.clone()],"last":12}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 12).await;
    assert_eq!(
        ingress.admit(verified(interaction)).await.status,
        AdmissionStatus::Duplicate
    );
    assert_eq!(buttons.load(Ordering::SeqCst), 1);
    assert_eq!(http.posts.load(Ordering::SeqCst), 1);
    handle.stop();
    run.await.unwrap().unwrap();
}

#[tokio::test]
async fn handled_failures_continue_but_failed_replies_preserve_causes_and_barriers() {
    for failed_reply in [false, true] {
        let response = if failed_reply {
            (
                502,
                vec![("Retry-After", "0.25")],
                json!({"error":"unknown reply"}).to_string(),
            )
        } else {
            support::response(
                201,
                json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-11-05T00:00:00Z"}}),
            )
        };
        let mut http = Http::new(vec![response]).await;
        let callbacks = Arc::new(AtomicUsize::new(0));
        let cb = callbacks.clone();
        let later = Arc::new(AtomicUsize::new(0));
        let l = later.clone();
        let bot = builder(&http)
            .configure(|settings| settings.execution.concurrency = 1)
            .command("reject", "reject", |_| async {
                Err(CommandError::Denied("expected rejection".into()).into())
            })
            .command("later", "later", move |_| {
                let l = l.clone();
                async move {
                    l.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .on_command_failure(move |_, failure| {
                let cb = cb.clone();
                async move {
                    assert_eq!(failure.class, FailureClass::Rejected);
                    assert_eq!(failure.error.kind, "command");
                    cb.fetch_add(1, Ordering::SeqCst);
                    Ok(FailureDisposition::Default)
                }
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let ingress = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        http.next().await.send(json!({"events":[event(10,"reject"),event(10,"reject"),event(20,"later")],"last":20})).unwrap();
        if failed_reply {
            let error = run.await.unwrap().unwrap_err();
            let report = match error {
                Error::EventProcessing { report } => report,
                other => panic!("{other:?}"),
            };
            assert_eq!(report.committed_cursor, 0);
            let error = &report.failed_events[0].error;
            assert_eq!(error.kind, "failure_handling");
            assert_eq!(error.status, Some(502));
            assert_eq!(error.retry_after, Some(0.25));
            assert_eq!(error.details["source"]["kind"], "command");
            assert_eq!(later.load(Ordering::SeqCst), 0);
        } else {
            until(|| ingress.status().committed_cursor == 20).await;
            assert_eq!(later.load(Ordering::SeqCst), 1);
            handle.stop();
            run.await.unwrap().unwrap();
        }
        assert_eq!(callbacks.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn uncertain_effects_and_cancelled_started_records_block_restart() {
    for cancelled in [false, true] {
        let mut http = Http::new(vec![]).await;
        let store = Arc::new(Store::default());
        let started = Arc::new(Notify::new());
        let notify = started.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let bot = builder(&http)
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("effect", "uncertain", move |_| {
                let notify = notify.clone();
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    notify.notify_one();
                    if cancelled {
                        std::future::pending::<()>().await;
                    }
                    Err(Error::PartialWrite {
                        completed: vec![json!({"receipt":"confirmed"})],
                        failed_step: 2,
                        source: Box::new(Error::Api {
                            status: 502,
                            message: "unknown write".into(),
                            retry_after: None,
                        }),
                    })
                }
            })
            .build()
            .expect("valid bot configuration");
        let ingress = bot.handle().ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        http.next()
            .await
            .send(json!({"events":[event(10,"effect")],"last":10}))
            .unwrap();
        started.notified().await;
        if cancelled {
            run.abort();
            let _ = run.await;
        } else {
            let error = run.await.unwrap().unwrap_err();
            let report = match error {
                Error::EventProcessing { report } => report,
                other => panic!("{other:?}"),
            };
            assert_eq!(
                report.failed_events[0].error.details["completed_steps"][0]["receipt"],
                "confirmed"
            );
        }
        let c = calls.clone();
        let bot = builder(&http)
            .configure(|settings| settings.persistence.store = Some(store))
            .command("effect", "no replay", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let next = bot.handle().ingress().unwrap();
        let error = bot.run().await.unwrap_err();
        assert_eq!(error.to_json()["kind"], "ingress_recovery");
        assert!(next.wait_ready().await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn filesystem_cursor_failure_keeps_completed_evidence_and_durable_receipts() {
    let mut http = Http::new(vec![]).await;
    let store = Arc::new(Store::default());
    let root = std::env::temp_dir().join(format!("rappin-ingress-cursor-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("cursor.json");
    EventCursor::new(&path).save(0).await.unwrap();
    let sabotage = path.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .configure(|settings| settings.persistence.cursor = Some(rappin::EventCursor::new(&path)))
        .command("work", "test", move |_| {
            let path = sabotage.clone();
            async move {
                std::fs::remove_file(&path)?;
                std::fs::create_dir(&path)?;
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    http.next()
        .await
        .send(json!({"events":[event(10,"work")],"last":10}))
        .unwrap();
    let report = match run.await.unwrap().unwrap_err() {
        Error::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(report.cursor_error.unwrap().kind, "io");
    assert_eq!(report.completed_uncommitted_events, [10]);
    assert_eq!(
        store.data.lock().unwrap().records[0].status.stage,
        ReceiptStage::Completed
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "hot-reload")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_gates_duplicate_delivery_and_handled_failures_share_ingress() {
    use rappin::hot_reload::{HotReload, WorkerProject};
    let root = std::env::temp_dir().join(format!("rappin-ingress-worker-{}", Uuid::new_v4()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"),format!("[package]\nname=\"shared-ingress-worker\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin={{package=\"rappin-rs\",path={},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\n",json!(env!("CARGO_MANIFEST_DIR")))).unwrap();
    std::fs::write(root.join("src/main.rs"),r#"
use rappin::{*, hot_reload::WorkerCog};
#[tokio::main] async fn main()->Result<()> {
 WorkerCog::new("shared-worker")
 .command("worker","reply",|ctx|async move {ctx.reply("worker executed once").await})
 .command("reject","reject",|_|async {Err(CommandError::Denied("worker policy denied".into()).into())})
 .run().await
}
"#).unwrap();
    let message = json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-11-05T00:00:00Z"}});
    let mut http = Http::new(vec![
        support::response(201, message.clone()),
        support::response(201, message),
    ])
    .await;
    let failures = Arc::new(AtomicUsize::new(0));
    let f = failures.clone();
    let bot = builder(&http)
        .message_gate(|ctx| async move {
            Ok(if ctx.content() == "denied" {
                MessageDisposition::Handled
            } else {
                MessageDisposition::Continue
            })
        })
        .on_command_failure(move |_, failure| {
            let f = f.clone();
            async move {
                assert_eq!(failure.error.kind, "command");
                f.fetch_add(1, Ordering::SeqCst);
                Ok(FailureDisposition::Default)
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let reload = HotReload::load(
        handle.clone(),
        WorkerProject::new(root.join("Cargo.toml"), "shared-ingress-worker").offline(true),
    )
    .await
    .unwrap();
    let pid = reload.process_id().await;
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    let mut denied = event(10, "worker");
    denied.data["content"] = json!("denied");
    let mut denied_receipt = ingress
        .admit(verified(denied.clone()))
        .await
        .receipt
        .unwrap();
    assert_eq!(denied_receipt.wait().await.stage, ReceiptStage::Completed);
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    let mut valid = ingress
        .admit(verified(event(20, "worker")))
        .await
        .receipt
        .unwrap();
    assert_eq!(valid.wait().await.stage, ReceiptStage::Completed);
    poll.send(json!({"events":[denied,event(20,"worker"),event(30,"reject"),event(30,"reject")],"last":30})).unwrap();
    until(|| ingress.status().committed_cursor == 30).await;
    assert_eq!(http.posts.load(Ordering::SeqCst), 2);
    assert_eq!(failures.load(Ordering::SeqCst), 1);
    assert_eq!(reload.process_id().await, pid);
    handle.stop();
    run.await.unwrap().unwrap();
    reload.unload().await.unwrap();
    // Unload releases the generation; its asynchronous shutdown must finish
    // before Windows can remove the worker's current working directory.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match std::fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(error) if cfg!(windows) && error.raw_os_error() == Some(32) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("worker project cleanup failed: {error}"),
            }
        }
    })
    .await
    .expect("unloaded worker did not release its working directory");
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn webhook_gateway_and_polling_share_claims_and_report_gateway_payload_conflicts() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (send, mut frames) = mpsc::unbounded_channel::<Event>();
    let ready = Arc::new(Notify::new());
    let connected = ready.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":100})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(
            json!({"op":"ready","seq":0,"requests":false})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        connected.notify_one();
        loop {
            tokio::select! {
                Some(event)=frames.recv()=>{ws.send(Message::Text(json!({"op":"event","d":event}).to_string().into())).await.unwrap();},
                frame=ws.next()=>{let Some(Ok(Message::Text(frame)))=frame else {break;};let value:Value=serde_json::from_str(&frame).unwrap();if value["op"]=="heartbeat"{ws.send(Message::Text(json!({"op":"heartbeat_ack"}).to_string().into())).await.unwrap();}}
            }
        }
    });
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| {
            settings.transport.gateway_url = Some(format!("ws://{address}"));
            settings.transport.mode = rappin::TransportMode::GatewayPreferred;
        })
        .command("ping", "test", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    ready.notified().await;
    let poll = http.next().await;
    send.send(event(10, "ping")).unwrap();
    let admission = ingress.admit(verified(event(10, "ping"))).await;
    assert!(matches!(
        admission.status,
        AdmissionStatus::Accepted | AdmissionStatus::Duplicate
    ));
    poll.send(json!({"events":[event(10,"ping")],"last":10}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 10).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut receipt = ingress
        .admit(verified(event(20, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    send.send(event(20, "ping")).unwrap();
    until(|| ingress.status().duplicates >= 2).await;
    let mut conflict = event(20, "ping");
    conflict.data["content"] = json!("conflicting content");
    send.send(conflict).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let report = match error {
        Error::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(report.committed_cursor, 10);
    let error = report.intake_error.unwrap();
    assert_eq!(error.kind, "ingress_conflict");
    assert_eq!(error.details["conflict"]["incoming_source"], "gateway");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    server.abort();
}

struct PausedCache(Notify);
#[async_trait::async_trait]
impl cache::LookupCache for PausedCache {
    async fn get_member(&self, _: Uuid, _: Uuid) -> Option<MemberDetail> {
        None
    }
    async fn get_roles(&self, _: Uuid) -> Option<Vec<Role>> {
        None
    }
    async fn store_member(&self, _: Uuid, _: Uuid, _: &MemberDetail) {}
    async fn store_roles(&self, _: Uuid, _: &[Role]) {}
    async fn invalidate_member(&self, _: Uuid, _: Uuid) {
        self.0.notify_one();
        std::future::pending::<()>().await;
    }
    async fn invalidate_server(&self, _: Uuid) {}
}
#[tokio::test]
async fn cancelling_admission_after_intake_started_stops_without_waiting_for_poll_delivery() {
    let mut http = Http::new(vec![]).await;
    let cache = Arc::new(PausedCache(Notify::new()));
    let store = Arc::new(Store::default());
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store))
        .lookup_cache_backend(cache.clone())
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let event:Event=serde_json::from_value(json!({"id":10,"type":"member_join","data":{"server_id":Uuid::nil(),"user_id":Uuid::nil()}})).unwrap();
    let input = ingress.clone();
    let admission = tokio::spawn(async move { input.admit(verified(event)).await });
    cache.0.notified().await;
    admission.abort();
    let _ = admission.await;
    let error = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let report = match error {
        Error::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(report.intake_error.unwrap().kind, "handler_interrupted");
    let receipt = ingress.receipt(10).unwrap().status();
    assert_eq!(receipt.stage, ReceiptStage::Failed);
    assert_eq!(receipt.error.unwrap().details["outcome"], "unknown");
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
}

fn recover_config() -> IngressConfig {
    let mut config = IngressConfig::default();
    config.conflict_policy = IngressConflictPolicy::RetainCompletedAndPoll;
    config
}
fn pipeline_report(error: Error) -> Box<EventProcessingFailure> {
    match error {
        Error::EventProcessing { report } => report,
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn conflicting_poll_timestamp_has_bounded_source_diagnostics_without_changing_completion() {
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    let mut original = event(10, "ping");
    original.data["content"] = json!("PRIVATE CONTENT AND CREDENTIAL");
    original.data["created_at"] = json!("2026-11-05T00:00:00Z");
    let mut receipt = ingress
        .admit(verified(original.clone()))
        .await
        .receipt
        .unwrap();
    assert_eq!(receipt.wait().await.stage, ReceiptStage::Completed);
    let mut incoming = original.clone();
    incoming.created_at = Some("2026-11-05T00:00:00Z".parse().unwrap());
    poll.send(json!({"events":[incoming],"last":10})).unwrap();
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(report.completed_uncommitted_events, [10]);
    assert!(report.failed_events.is_empty());
    let error = report.intake_error.unwrap().to_json();
    assert_eq!(error["kind"], "ingress_conflict");
    let conflict = &error["conflict"];
    assert_eq!(conflict["existing_source"], "webhook");
    assert_eq!(conflict["incoming_source"], "polling");
    assert_eq!(conflict["differences"][0]["path"], "/created_at");
    assert_eq!(conflict["differences"][0]["existing_type"], "null");
    assert_eq!(conflict["differences"][0]["incoming_type"], "string");
    assert_ne!(
        conflict["existing_fingerprint"],
        conflict["incoming_fingerprint"]
    );
    assert!(!error.to_string().contains("PRIVATE"));
    assert_eq!(receipt.status().stage, ReceiptStage::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fast_webhook_conflict_stops_by_default_and_keeps_original_receipt() {
    let mut http = Http::new(vec![]).await;
    let bot = builder(&http)
        .command("ping", "record", |_| async { Ok(()) })
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let mut receipt = ingress
        .admit(verified(event(10, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    let mut incoming = event(10, "ping");
    incoming.data["author_id"] = json!(Uuid::from_u128(999));
    let admission = ingress.admit(verified(incoming)).await;
    assert_eq!(admission.status, AdmissionStatus::Error);
    assert_eq!(
        admission.receipt.unwrap().status().stage,
        ReceiptStage::Completed
    );
    let error = admission.error.unwrap().to_json();
    assert_eq!(
        error["conflict"]["differences"][0]["path"],
        "/data/author_id"
    );
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(report.completed_uncommitted_events, [10]);
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(receipt.status().stage, ReceiptStage::Completed);
}

#[tokio::test]
async fn completed_conflict_requires_matching_polling_payload_not_just_a_watermark() {
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    let original = event(10, "ping");
    let mut receipt = ingress
        .admit(verified(original.clone()))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    let mut changed = original.clone();
    changed.data["content"] = json!("contradictory");
    let admission = ingress.admit(verified(changed)).await;
    assert_eq!(admission.status, AdmissionStatus::Error);
    assert_eq!(
        admission.error.unwrap().to_json()["conflict"]["disposition"],
        "awaiting_polling"
    );
    poll.send(json!({"events":[],"last":20})).unwrap();
    let next = http.next().await;
    until(|| ingress.status().polling_watermark == 20).await;
    assert_eq!(*http.poll_after.lock().unwrap().last().unwrap(), 9);
    assert!(ingress.status().committed_cursor < 10);
    assert!(!receipt.status().committed);
    next.send(json!({"events":[original],"last":20})).unwrap();
    until(|| ingress.status().committed_cursor == 20).await;
    assert_eq!(
        ingress.conflicts()[0].disposition,
        ConflictDisposition::PollingConfirmed
    );
    assert_eq!(ingress.status().pending_conflicts, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(receipt.status().committed);
    handle.stop();
    run.await.unwrap().unwrap();
}

#[tokio::test]
async fn durable_restarts_keep_conflict_evidence_and_original_completion_without_repeated_effects()
{
    let store = Arc::new(Store::default());
    let calls = Arc::new(AtomicUsize::new(0));
    for phase in 0..3 {
        let mut http = Http::new(vec![]).await;
        let c = calls.clone();
        let bot = builder(&http)
            .configure(|settings| settings.persistence.ingress = Some(recover_config()))
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "record", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let ingress = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        let poll = http.next().await;
        if phase == 0 {
            let mut receipt = ingress
                .admit(verified(event(10, "ping")))
                .await
                .receipt
                .unwrap();
            receipt.wait().await;
        } else if phase == 1 {
            let mut changed = event(10, "ping");
            changed.data["server_id"] = json!(Uuid::from_u128(77));
            let result = ingress.admit(verified(changed)).await;
            let error = result.error.unwrap().to_json();
            assert_eq!(error["conflict"]["existing_source"], "recovery");
            assert_eq!(error["conflict"]["recovered_source"], "webhook");
            assert_eq!(
                store.data.lock().unwrap().records[0].event,
                event(10, "ping")
            );
            assert_eq!(
                store.data.lock().unwrap().conflicts[0].disposition,
                ConflictDisposition::AwaitingPolling
            );
        } else {
            assert_eq!(ingress.status().pending_conflicts, 1);
            poll.send(json!({"events":[event(10,"ping")],"last":10}))
                .unwrap();
            until(|| ingress.status().committed_cursor == 10).await;
            assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(10));
            assert_eq!(
                store.data.lock().unwrap().conflicts[0].disposition,
                ConflictDisposition::PollingConfirmed
            );
            handle.stop();
            run.await.unwrap().unwrap();
            continue;
        }
        handle.stop();
        run.await.unwrap().unwrap();
        assert_eq!(
            store.data.lock().unwrap().records[0].status.stage,
            ReceiptStage::Completed
        );
        assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(0));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn durable_conflict_confirmation_and_checkpoint_failures_never_reexecute_completed_work() {
    for fail_confirmation in [false, true] {
        let mut http = Http::new(vec![]).await;
        let store = Arc::new(Store::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let bot = builder(&http)
            .configure(|settings| settings.persistence.ingress = Some(recover_config()))
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "record", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let ingress = bot.handle().ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        let poll = http.next().await;
        let mut receipt = ingress
            .admit(verified(event(10, "ping")))
            .await
            .receipt
            .unwrap();
        receipt.wait().await;
        let mut changed = event(10, "ping");
        changed.data["content"] = json!("changed");
        assert_eq!(
            ingress.admit(verified(changed)).await.status,
            AdmissionStatus::Error
        );
        if fail_confirmation {
            store.fail_conflict.store(2, Ordering::SeqCst);
        } else {
            store.fail_checkpoint.store(1, Ordering::SeqCst);
        }
        poll.send(json!({"events":[event(10,"ping")],"last":10}))
            .unwrap();
        let report = pipeline_report(run.await.unwrap().unwrap_err());
        assert_eq!(report.committed_cursor, 0);
        assert_eq!(report.completed_uncommitted_events, [10]);
        let error = if fail_confirmation {
            report.intake_error.unwrap()
        } else {
            report.cursor_error.unwrap()
        };
        if fail_confirmation {
            assert_eq!(error.kind, "failure_handling");
            assert_eq!(error.status, Some(503));
            assert_eq!(error.details["handling_error"]["kind"], "ingress_storage");
            assert_eq!(
                Error::from(error.clone())
                    .ingress_conflict()
                    .unwrap()
                    .event_id,
                10
            );
        } else {
            assert_eq!(error.kind, "ingress_storage");
        }
        assert_eq!(receipt.status().stage, ReceiptStage::Completed);
        assert_eq!(
            store.data.lock().unwrap().records[0].status.stage,
            ReceiptStage::Completed
        );
        store.fail_conflict.store(0, Ordering::SeqCst);
        store.fail_checkpoint.store(0, Ordering::SeqCst);
        let mut restart_http = Http::new(vec![]).await;
        let c = calls.clone();
        let restart = builder(&restart_http)
            .configure(|settings| settings.persistence.ingress = Some(recover_config()))
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "record", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let handle = restart.handle();
        let input = handle.ingress().unwrap();
        let run = tokio::spawn(restart.run());
        input.wait_ready().await.unwrap();
        restart_http
            .next()
            .await
            .send(json!({"events":[event(10,"ping")],"last":10}))
            .unwrap();
        until(|| input.status().committed_cursor == 10).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        handle.stop();
        run.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn conflicting_polling_batch_is_rejected_before_any_event_effects() {
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let mut changed = event(10, "ping");
    changed.data["content"] = json!("changed");
    http.next()
        .await
        .send(json!({"events":[event(10,"ping"),changed],"last":10}))
        .unwrap();
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    let error = report.intake_error.unwrap().to_json();
    assert_eq!(error["kind"], "ingress_conflict");
    assert_eq!(error["conflict"]["existing_source"], "polling");
    assert_eq!(error["conflict"]["incoming_source"], "polling");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(report.committed_cursor, 0);
}

#[tokio::test]
async fn unanswered_recovery_times_out_and_uncertain_handler_effects_cannot_use_recovery_policy() {
    let mut http = Http::new(vec![]).await;
    let mut config = recover_config();
    config.conflict_recovery_timeout = Duration::from_millis(40);
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(config))
        .command("ping", "record", |_| async { Ok(()) })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let mut receipt = input
        .admit(verified(event(10, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    assert_eq!(
        input.admit(verified(event(10, "different"))).await.status,
        AdmissionStatus::Error
    );
    let report = pipeline_report(
        tokio::time::timeout(Duration::from_secs(3), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
    );
    assert_eq!(
        report.intake_error.unwrap().to_json()["conflict"]["disposition"],
        "timed_out"
    );
    assert_eq!(receipt.status().stage, ReceiptStage::Completed);
    assert_eq!(report.committed_cursor, 0);

    let mut http = Http::new(vec![(
        502,
        vec![("Retry-After", "0.25")],
        json!({"error":"uncertain write"}).to_string(),
    )])
    .await;
    let ready = Arc::new(Notify::new());
    let started = ready.clone();
    let release = Arc::new(Notify::new());
    let wait = release.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .command("write", "write", move |ctx| {
            let started = started.clone();
            let wait = wait.clone();
            async move {
                let error = ctx.reply("attempt exactly once").await.unwrap_err();
                started.notify_one();
                wait.notified().await;
                Err(error)
            }
        })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let receipt = input
        .admit(verified(event(10, "write")))
        .await
        .receipt
        .unwrap();
    ready.notified().await;
    let admission = input.admit(verified(event(10, "different"))).await;
    assert_eq!(
        admission.error.unwrap().to_json()["conflict"]["disposition"],
        "stop"
    );
    release.notify_one();
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(http.posts.load(Ordering::SeqCst), 1);
    assert_eq!(receipt.status().stage, ReceiptStage::Failed);
    assert_eq!(report.failed_events[0].error.status, Some(502));
    assert_eq!(report.failed_events[0].error.retry_after, Some(0.25));
    assert_eq!(report.committed_cursor, 0);
}

#[tokio::test]
async fn durable_recovery_refuses_an_unimplemented_conflict_store_hook() {
    struct Legacy(Arc<Store>);
    #[async_trait::async_trait]
    impl IngressStore for Legacy {
        async fn load(&self, bot: Uuid) -> Result<IngressRecovery> {
            self.0.load(bot).await
        }
        async fn record(&self, record: IngressRecord) -> Result<()> {
            self.0.record(record).await
        }
        async fn checkpoint(&self, bot: Uuid, cursor: u64) -> Result<()> {
            self.0.checkpoint(bot, cursor).await
        }
    }
    let mut http = Http::new(vec![]).await;
    let store = Arc::new(Store::default());
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .configure(|settings| settings.persistence.store = Some(Arc::new(Legacy(store.clone()))))
        .command("ping", "record", |_| async { Ok(()) })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let mut receipt = input
        .admit(verified(event(10, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    let result = input.admit(verified(event(10, "different"))).await;
    let error = result.error.unwrap();
    assert_eq!(error.ingress_conflict().unwrap().event_id, 10);
    assert_eq!(error.to_json()["handling_error"]["kind"], "configuration");
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(report.completed_uncommitted_events, [10]);
    assert_eq!(
        store.data.lock().unwrap().records[0].status.stage,
        ReceiptStage::Completed
    );
}

#[tokio::test]
async fn simultaneous_conflicting_claims_never_run_the_second_handler_or_resolve_unfinished_effects(
) {
    let mut http = Http::new(vec![]).await;
    let store = Arc::new(Store::default());
    store.block_queued.store(10, Ordering::SeqCst);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let admission_input = input.clone();
    let admission =
        tokio::spawn(async move { admission_input.admit(verified(event(10, "ping"))).await });
    store.queued.notified().await;
    let result = input.admit(verified(event(10, "different"))).await;
    assert_eq!(
        result
            .error
            .unwrap()
            .ingress_conflict()
            .unwrap()
            .disposition,
        ConflictDisposition::Stop
    );
    store.release.notify_one();
    let first = admission.await.unwrap();
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    if let Some(receipt) = first.receipt {
        assert_ne!(receipt.status().stage, ReceiptStage::Completed);
    }
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn gateway_conflict_can_wait_for_polling_without_repeating_the_completed_handler() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (send, mut frames) = mpsc::unbounded_channel::<Event>();
    let ready = Arc::new(Notify::new());
    let connected = ready.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":100})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(
            json!({"op":"ready","seq":0,"requests":false})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        connected.notify_one();
        loop {
            tokio::select! {
                Some(event)=frames.recv()=>{ws.send(Message::Text(json!({"op":"event","d":event}).to_string().into())).await.unwrap();},
                frame=ws.next()=>{let Some(Ok(Message::Text(frame)))=frame else{break;};let value:Value=serde_json::from_str(&frame).unwrap();if value["op"]=="heartbeat"{ws.send(Message::Text(json!({"op":"heartbeat_ack"}).to_string().into())).await.unwrap();}}
            }
        }
    });
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .configure(|settings| {
            settings.transport.gateway_url = Some(format!("ws://{address}"));
            settings.transport.mode = rappin::TransportMode::GatewayPreferred;
        })
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let input = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    ready.notified().await;
    let poll = http.next().await;
    let mut receipt = input
        .admit(verified(event(10, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    let mut altered = event(10, "ping");
    altered.created_at = Some("2026-11-05T00:00:00Z".parse().unwrap());
    send.send(altered).unwrap();
    until(|| input.status().pending_conflicts == 1).await;
    assert_eq!(input.conflicts()[0].incoming_source, IngressSource::Gateway);
    poll.send(json!({"events":[event(10,"ping")],"last":10}))
        .unwrap();
    until(|| input.status().committed_cursor == 10).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        input.conflicts()[0].disposition,
        ConflictDisposition::PollingConfirmed
    );
    handle.stop();
    run.await.unwrap().unwrap();
    server.abort();
}

#[tokio::test]
async fn cancelled_conflict_publication_keeps_diagnostic_cause_and_completion_barrier() {
    let mut http = Http::new(vec![]).await;
    let store = Arc::new(Store::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let _poll = http.next().await;
    let mut receipt = input
        .admit(verified(event(10, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    store.block_conflict.store(true, Ordering::SeqCst);
    let admission_input = input.clone();
    let admission = tokio::spawn(async move {
        admission_input
            .admit(verified(event(10, "different")))
            .await
    });
    store.conflict_waiting.notified().await;
    admission.abort();
    let _ = admission.await;
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    let error = Error::from(report.intake_error.unwrap());
    assert_eq!(error.ingress_conflict().unwrap().event_id, 10);
    assert_eq!(report.completed_uncommitted_events, [10]);
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(receipt.status().stage, ReceiptStage::Completed);
    assert_eq!(
        store.data.lock().unwrap().records[0].status.stage,
        ReceiptStage::Completed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stopping_during_confirmation_publication_reports_the_retained_conflict_barrier() {
    let mut http = Http::new(vec![]).await;
    let store = Arc::new(Store::default());
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(recover_config()))
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "record", |_| async { Ok(()) })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let input = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let poll = http.next().await;
    let mut receipt = input
        .admit(verified(event(10, "ping")))
        .await
        .receipt
        .unwrap();
    receipt.wait().await;
    input
        .admit(verified(event(10, "different")))
        .await
        .error
        .unwrap();
    store.block_conflict.store(true, Ordering::SeqCst);
    poll.send(json!({"events":[event(10,"ping")],"last":10}))
        .unwrap();
    store.conflict_waiting.notified().await;
    handle.stop();
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(
        Error::from(report.intake_error.unwrap())
            .ingress_conflict()
            .unwrap()
            .event_id,
        10
    );
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(report.completed_uncommitted_events, [10]);
    assert_eq!(receipt.status().stage, ReceiptStage::Completed);
    assert_eq!(
        store.data.lock().unwrap().records[0].status.stage,
        ReceiptStage::Completed
    );
}

#[tokio::test]
async fn diagnostic_capacity_keeps_unresolved_evidence_and_reports_the_new_conflict() {
    let mut http = Http::new(vec![]).await;
    let mut config = recover_config();
    config.max_conflict_reports = 1;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(config))
        .command("ping", "record", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let input = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let _poll = http.next().await;
    for id in [10, 20] {
        let mut receipt = input
            .admit(verified(event(id, "ping")))
            .await
            .receipt
            .unwrap();
        receipt.wait().await;
    }
    input
        .admit(verified(event(10, "different")))
        .await
        .error
        .unwrap();
    let error = input
        .admit(verified(event(20, "different")))
        .await
        .error
        .unwrap();
    assert_eq!(error.ingress_conflict().unwrap().event_id, 20);
    assert_eq!(input.conflicts().len(), 1);
    assert_eq!(input.conflicts()[0].event_id, 10);
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(report.completed_uncommitted_events, [10, 20]);
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// Synthetic scoped server event; all IDs/content are fixtures.
fn legacy_event() -> Event {
    let mut value = event(100, "ping");
    value.data["server_id"] = json!(Uuid::from_u128(201));
    value.data["channel_id"] = json!(Uuid::from_u128(202));
    value.data["author_id"] = json!(Uuid::from_u128(203));
    value.data["created_at"] = json!("2026-01-01T01:02:03Z");
    value
}
fn event_hash(event: &Event) -> String {
    use sha2::Digest;
    // The default JSON map is sorted; recursively sort via BTreeMap even if a
    // downstream crate unifies serde_json/preserve_order.
    fn canonical(v: Value) -> Value {
        match v {
            Value::Object(map) => map
                .into_iter()
                .map(|(k, v)| (k, canonical(v)))
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_iter()
                .collect(),
            Value::Array(a) => Value::Array(a.into_iter().map(canonical).collect()),
            other => other,
        }
    }
    Sha256::digest(serde_json::to_vec(&canonical(serde_json::to_value(event).unwrap())).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
async fn legacy_fixture(http: &mut Http) -> (Arc<Store>, Event, Arc<AtomicUsize>) {
    let store = Arc::new(Store::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = builder(http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "fixture", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    let mut receipt = ingress
        .admit(verified(legacy_event()))
        .await
        .receipt
        .unwrap();
    assert_eq!(receipt.wait().await.stage, ReceiptStage::Completed);
    let mut incoming = legacy_event();
    incoming.created_at = Some("2026-02-03T04:05:06Z".parse().unwrap());
    poll.send(json!({"events":[incoming.clone()],"last":100}))
        .unwrap();
    assert!(run.await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Model a legacy persisted receipt without original transport provenance.
    // This is synthetic fixture construction, not a live payload rewrite.
    {
        let mut snapshot = store.data.lock().unwrap();
        snapshot.records[0].source = None;
        snapshot.conflicts[0].existing_source = IngressSource::Recovery;
        snapshot.conflicts[0].recovered_source = None;
    }
    (store, incoming, calls)
}
fn timestamp_request(incoming: &Event) -> TimestampReconciliationRequest {
    TimestampReconciliationRequest::new(
        bot_id(),
        100,
        event_hash(&legacy_event()),
        event_hash(incoming),
    )
}
async fn prepare_fixture(
    http: &mut Http,
    store: Arc<Store>,
    incoming: &Event,
    options: TimestampReconciliationOptions,
) -> Result<PreparedTimestampReconciliation> {
    let client = Client::builder("fixture").base_url(&http.url).build();
    let request = timestamp_request(incoming);
    let task = tokio::spawn(async move {
        prepare_timestamp_reconciliation(&client, store, request, options).await
    });
    http.next()
        .await
        .send(json!({"events":[incoming],"last":100}))
        .unwrap();
    task.await.unwrap()
}
#[tokio::test]
async fn explicit_timestamp_approval_preserves_completion_and_requires_normal_polling() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut http).await;
    let original = serde_json::to_value(&store.data.lock().unwrap().records).unwrap();
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    assert_eq!(
        prepared.evidence().state,
        TimestampReconciliationState::Prepared
    );
    assert!(store
        .data
        .lock()
        .unwrap()
        .timestamp_reconciliations
        .is_empty());
    assert_eq!(http.poll_after.lock().unwrap().last(), Some(&99));
    let approval = prepared
        .approve(OperationId::new("fixture/approval/1").unwrap())
        .await
        .unwrap();
    assert_eq!(approval.state, TimestampReconciliationState::Approved);
    assert_eq!(
        serde_json::to_value(&store.data.lock().unwrap().records).unwrap(),
        original
    );
    assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(0));
    // A fresh process restores only the original Completed receipt and approval.
    let c = calls.clone();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "fixture", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    assert_eq!(
        ingress.conflicts()[0].disposition,
        ConflictDisposition::AwaitingReconciliationPolling
    );
    let admission = ingress.admit(verified(incoming.clone())).await;
    assert_eq!(admission.status, AdmissionStatus::Error);
    assert_eq!(ingress.status().committed_cursor, 0);
    // Empty polling/higher watermark is not approval evidence.
    poll.send(json!({"events":[],"last":100})).unwrap();
    let poll = http.next().await;
    assert!(ingress.status().committed_cursor < 100);
    assert_eq!(
        store.data.lock().unwrap().timestamp_reconciliations[0].state,
        TimestampReconciliationState::Approved
    );
    poll.send(json!({"events":[incoming.clone()],"last":100}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 100).await;
    assert_eq!(
        ingress.conflicts()[0].disposition,
        ConflictDisposition::Reconciled
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    handle.stop();
    run.await.unwrap().unwrap();
    // A third start from the committed checkpoint still does not replay.
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "fixture", |_| async {
            panic!("completed handler replayed")
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    http.next()
        .await
        .send(json!({"events":[incoming],"last":100}))
        .unwrap();
    let _next = http.next().await;
    handle.stop();
    run.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn timestamp_approval_atomic_failures_and_ambiguous_commits_do_not_replay() {
    for failure in [1, 2, 3] {
        let mut http = Http::new(vec![]).await;
        let (store, incoming, calls) = legacy_fixture(&mut http).await;
        let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
            .await
            .unwrap();
        store.fail_reconciliation.store(failure, Ordering::SeqCst);
        let result = prepared
            .approve(OperationId::new("fixture/approved").unwrap())
            .await;
        if failure < 3 {
            let error = result.unwrap_err();
            assert_eq!(error.status_code(), Some(503));
            assert_eq!(error.retry_after(), Some(Duration::from_secs_f64(0.5)));
            assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 1);
            assert_eq!(
                store
                    .data
                    .lock()
                    .unwrap()
                    .timestamp_reconciliations
                    .is_empty(),
                failure == 1
            );
        } else {
            result.unwrap();
        }
        assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(0));
        assert_eq!(store.data.lock().unwrap().records[0].event.created_at, None);
        store
            .fail_reconciliation
            .store(if failure == 3 { 3 } else { 0 }, Ordering::SeqCst);
        let bot = builder(&http)
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "never replay", |_| async { panic!("replayed") })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let ingress = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        if failure == 1 {
            assert!(run.await.unwrap().is_err());
        } else {
            ingress.wait_ready().await.unwrap();
            http.next()
                .await
                .send(json!({"events":[incoming],"last":100}))
                .unwrap();
            if failure == 3 {
                let error = run.await.unwrap().unwrap_err();
                assert_eq!(error.status_code(), Some(503));
                assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(0));
                assert_eq!(
                    store.data.lock().unwrap().timestamp_reconciliations[0].state,
                    TimestampReconciliationState::Approved
                );
            } else {
                until(|| ingress.status().committed_cursor == 100).await;
                handle.stop();
                run.await.unwrap().unwrap();
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn timestamp_preparation_rejects_changed_scope_payload_and_wrong_fingerprints() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut http).await;
    for key in [
        "content",
        "author_id",
        "server_id",
        "channel_id",
        "id",
        "created_at",
    ] {
        let mut changed = incoming.clone();
        changed.data[key] = json!("changed");
        // Bind the request to the changed fingerprint: full comparison must still reject it.
        assert!(
            prepare_fixture(&mut http, store.clone(), &changed, Default::default())
                .await
                .is_err()
        );
    }
    let client = Client::builder("fixture").base_url(&http.url).build();
    let mut request = timestamp_request(&incoming);
    request.polling_fingerprint = "a".repeat(64);
    let s = store.clone();
    let task = tokio::spawn(async move {
        prepare_timestamp_reconciliation(&client, s, request, Default::default()).await
    });
    http.next()
        .await
        .send(json!({"events":[incoming.clone()],"last":100}))
        .unwrap();
    assert!(task.await.unwrap().is_err());
    // Empty response is not proof, even with a higher watermark.
    let client = Client::builder("fixture").base_url(&http.url).build();
    let s = store.clone();
    let request = timestamp_request(&incoming);
    let task = tokio::spawn(async move {
        prepare_timestamp_reconciliation(&client, s, request, Default::default()).await
    });
    http.next()
        .await
        .send(json!({"events":[],"last":999}))
        .unwrap();
    assert!(task.await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn timestamp_approval_rejects_stale_preparation_expiry_and_ineligible_receipts() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, _) = legacy_fixture(&mut http).await;
    let mut options = TimestampReconciliationOptions::default();
    options.approval_ttl = Duration::from_millis(1);
    // A busy runner may expire this deliberately tiny lifetime during
    // preparation itself. Either phase must reject specifically for expiry.
    let expired = match prepare_fixture(&mut http, store.clone(), &incoming, options).await {
        Ok(prepared) => {
            tokio::time::sleep(Duration::from_millis(3)).await;
            prepared
                .approve(OperationId::new("expired").unwrap())
                .await
                .unwrap_err()
        }
        Err(error) => error,
    }
    .to_remote();
    assert_eq!(expired.kind, "timestamp_reconciliation");
    assert!(matches!(
        expired.details.get("reason").and_then(Value::as_str),
        Some("expired_preparation" | "stale_preparation")
    ));
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    store.data.lock().unwrap().polling_checkpoint = Some(1);
    assert!(prepared
        .approve(OperationId::new("stale").unwrap())
        .await
        .is_err());
    store.data.lock().unwrap().polling_checkpoint = Some(0);
    for stage in [
        ReceiptStage::Started,
        ReceiptStage::Failed,
        ReceiptStage::Queued,
    ] {
        store.data.lock().unwrap().records[0].status.stage = stage;
        let client = Client::builder("fixture").base_url(&http.url).build();
        assert!(prepare_timestamp_reconciliation(
            &client,
            store.clone(),
            timestamp_request(&incoming),
            Default::default()
        )
        .await
        .is_err());
    }
    store.data.lock().unwrap().records[0].status.stage = ReceiptStage::Completed;
    store.data.lock().unwrap().records[0].status.error =
        Some(Error::Config("uncertain fixture".into()).to_remote());
    let client = Client::builder("fixture").base_url(&http.url).build();
    assert!(prepare_timestamp_reconciliation(
        &client,
        store.clone(),
        timestamp_request(&incoming),
        Default::default()
    )
    .await
    .is_err());
    assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn timestamp_confirmation_lost_response_and_failed_checkpoint_recover_from_audit() {
    for lost_response in [true, false] {
        let mut http = Http::new(vec![]).await;
        let (store, incoming, calls) = legacy_fixture(&mut http).await;
        let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
            .await
            .unwrap();
        prepared
            .approve(OperationId::new("fixture/approval").unwrap())
            .await
            .unwrap();
        if lost_response {
            store.fail_reconciliation.store(4, Ordering::SeqCst);
        } else {
            store.fail_checkpoint.store(1, Ordering::SeqCst);
        }
        let bot = builder(&http)
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "no replay", |_| async { panic!("replayed") })
            .build()
            .expect("valid bot configuration");
        let ingress = bot.handle().ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        http.next()
            .await
            .send(json!({"events":[incoming.clone()],"last":100}))
            .unwrap();
        assert!(run.await.unwrap().is_err());
        {
            let data = store.data.lock().unwrap();
            assert_eq!(data.polling_checkpoint, Some(0));
            assert_eq!(
                data.timestamp_reconciliations[0].state,
                TimestampReconciliationState::Confirmed
            );
            assert!(data.records[0].event.created_at.is_none());
            assert_eq!(data.records[0].status.stage, ReceiptStage::Completed);
        }
        store.fail_reconciliation.store(0, Ordering::SeqCst);
        store.fail_checkpoint.store(0, Ordering::SeqCst);
        let bot = builder(&http)
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "no replay", |_| async { panic!("replayed") })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let ingress = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        http.next()
            .await
            .send(json!({"events":[incoming],"last":100}))
            .unwrap();
        until(|| ingress.status().committed_cursor == 100).await;
        handle.stop();
        run.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn cancelled_timestamp_confirmation_keeps_original_completion_and_checkpoint() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut http).await;
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    prepared
        .approve(OperationId::new("fixture/approval").unwrap())
        .await
        .unwrap();
    store.block_confirmation.store(true, Ordering::SeqCst);
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    http.next()
        .await
        .send(json!({"events":[incoming.clone()],"last":100}))
        .unwrap();
    store.confirmation_waiting.notified().await;
    run.abort();
    let _ = run.await;
    // The cancelled task must not publish a successful confirmation/checkpoint.
    until(|| !ingress.status().running).await;
    {
        let data = store.data.lock().unwrap();
        assert_eq!(data.polling_checkpoint, Some(0));
        assert_eq!(data.records[0].status.stage, ReceiptStage::Completed);
        assert_eq!(
            data.timestamp_reconciliations[0].state,
            TimestampReconciliationState::Approved
        );
    }
    store.block_confirmation.store(false, Ordering::SeqCst);
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    http.next()
        .await
        .send(json!({"events":[incoming],"last":100}))
        .unwrap();
    until(|| ingress.status().committed_cursor == 100).await;
    handle.stop();
    run.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn original_timestamp_polling_does_not_confirm_a_different_approved_fingerprint() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, _) = legacy_fixture(&mut http).await;
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    prepared
        .approve(OperationId::new("fixture/approval").unwrap())
        .await
        .unwrap();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let ingress = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    http.next()
        .await
        .send(json!({"events":[legacy_event()],"last":100}))
        .unwrap();
    let next = http.next().await;
    assert!(ingress.status().committed_cursor < 100);
    assert_eq!(
        store.data.lock().unwrap().timestamp_reconciliations[0].state,
        TimestampReconciliationState::Approved
    );
    next.send(json!({"events":[incoming],"last":100})).unwrap();
    until(|| ingress.status().committed_cursor == 100).await;
    handle.stop();
    run.await.unwrap().unwrap();
}

#[tokio::test]
async fn timestamp_decisions_bind_receipt_and_conflicts_and_cannot_replace_prior_approval() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut http).await;
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    store.data.lock().unwrap().records[0].event.data["server_id"] = json!(Uuid::from_u128(999));
    assert!(prepared
        .approve(OperationId::new("stale/receipt").unwrap())
        .await
        .is_err());
    store.data.lock().unwrap().records[0].event = legacy_event();
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    store.data.lock().unwrap().conflicts[0].incoming_fingerprint = "a".repeat(64);
    assert!(prepared
        .approve(OperationId::new("stale/report").unwrap())
        .await
        .is_err());
    store.data.lock().unwrap().conflicts[0].incoming_fingerprint = event_hash(&incoming);
    let first = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    let second = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    first
        .approve(OperationId::new("first/approval").unwrap())
        .await
        .unwrap();
    assert!(second
        .approve(OperationId::new("competing/approval").unwrap())
        .await
        .is_err());
    assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 1);
    // A stored decision cannot be used after its binding is corrupted.
    store.data.lock().unwrap().timestamp_reconciliations[0].polling_fingerprint = "b".repeat(64);
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .build()
        .expect("valid bot configuration");
    assert!(bot.run().await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(0));
}
#[tokio::test]
async fn unapproved_payload_after_timestamp_approval_stops_and_retains_checkpoint() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut http).await;
    let prepared = prepare_fixture(&mut http, store.clone(), &incoming, Default::default())
        .await
        .unwrap();
    prepared
        .approve(OperationId::new("fixture/approval").unwrap())
        .await
        .unwrap();
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .build()
        .expect("valid bot configuration");
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    let poll = http.next().await;
    let mut changed = incoming;
    changed.data["content"] = json!("unapproved content");
    poll.send(json!({"events":[changed],"last":100})).unwrap();
    let report = pipeline_report(run.await.unwrap().unwrap_err());
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(0));
    assert_eq!(
        store.data.lock().unwrap().timestamp_reconciliations[0].state,
        TimestampReconciliationState::Approved
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let bot = builder(&http)
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .build()
        .expect("valid bot configuration");
    assert!(bot.run().await.is_err());
}

#[tokio::test]
async fn timestamp_probe_zero_wait_snapshot_and_default_history_wait_are_distinct() {
    let mut fixture = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut fixture).await;
    let original = serde_json::to_value(store.data.lock().unwrap().clone()).unwrap();
    let (url, mut requests, server) = support::mock(vec![
        support::response(200, support::profile()),
        support::response(200, json!({"events":[],"last":99})),
        support::response(200, support::profile()),
        support::response(200, json!({"events":[incoming.clone()],"last":105})),
    ])
    .await;
    let client = Client::builder("fixture").base_url(url).build();
    let mut zero = TimestampReconciliationOptions::default();
    zero.polling_wait = 0;
    let error = prepare_timestamp_reconciliation(
        &client,
        store.clone(),
        timestamp_request(&incoming),
        zero,
    )
    .await
    .err()
    .unwrap()
    .to_remote();
    assert_eq!(error.details["reason"], "authoritative_event_unavailable");
    let prepared = prepare_timestamp_reconciliation(
        &client,
        store.clone(),
        timestamp_request(&incoming),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        prepared.evidence().state,
        TimestampReconciliationState::Prepared
    );
    server.await.unwrap();
    assert!(requests.recv().await.unwrap().starts_with("GET /me "));
    let zero = requests.recv().await.unwrap();
    assert!(zero.contains("after=99&wait=0&max_age=86400 "));
    assert!(requests.recv().await.unwrap().starts_with("GET /me "));
    let historical = requests.recv().await.unwrap();
    assert!(historical.contains("after=99&wait=1&max_age=86400 "));
    assert!(requests.recv().await.is_none());
    assert_eq!(
        serde_json::to_value(store.data.lock().unwrap().clone()).unwrap(),
        original
    );
    assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn timestamp_history_probe_still_rejects_missing_scope_hashes_and_changed_content() {
    let mut fixture = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut fixture).await;
    let original = serde_json::to_value(store.data.lock().unwrap().clone()).unwrap();
    for case in [
        "wrong_bot",
        "retained_hash",
        "polling_hash",
        "content",
        "unavailable",
        "retry",
    ] {
        let mut request = timestamp_request(&incoming);
        let mut returned = incoming.clone();
        let mut responses = vec![support::response(200, support::profile())];
        let expected = match case {
            "wrong_bot" => {
                request.bot_id = Uuid::from_u128(999);
                "authenticated_bot_mismatch"
            }
            "retained_hash" => {
                request.retained_fingerprint = "a".repeat(64);
                "retained_fingerprint_or_timestamp_mismatch"
            }
            "polling_hash" => {
                request.polling_fingerprint = "b".repeat(64);
                "polling_fingerprint_mismatch"
            }
            "content" => {
                returned.data["content"] = json!("changed");
                request.polling_fingerprint = event_hash(&returned);
                "non_metadata_difference"
            }
            "unavailable" => "authoritative_event_unavailable",
            _ => "retry",
        };
        if !matches!(case, "wrong_bot" | "retained_hash") {
            responses.push(if case == "retry" {
                support::response(503, json!({"message":"fixture unavailable"}))
            } else {
                support::response(
                    200,
                    json!({"events":if case=="unavailable"{vec![]}else{vec![returned]},"last":105}),
                )
            });
        }
        let (url, mut requests, server) = support::mock(responses).await;
        let client = Client::builder("fixture")
            .base_url(url)
            .max_retries(4)
            .build();
        let error =
            prepare_timestamp_reconciliation(&client, store.clone(), request, Default::default())
                .await
                .err()
                .unwrap();
        if expected == "retry" {
            assert_eq!(error.status_code(), Some(503));
        } else {
            assert_eq!(error.to_remote().details["reason"], expected);
        }
        server.await.unwrap();
        let mut count = 0;
        while let Some(request) = requests.recv().await {
            count += 1;
            assert!(request.starts_with("GET "));
            if request.starts_with("GET /events?") {
                assert!(request.contains("wait=1"));
            }
        }
        assert_eq!(
            count,
            if matches!(case, "wrong_bot" | "retained_hash") {
                1
            } else {
                2
            }
        );
        assert_eq!(
            serde_json::to_value(store.data.lock().unwrap().clone()).unwrap(),
            original
        );
    }
    assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn timestamp_probe_deadlines_and_cancellation_leave_no_approval_or_checkpoint() {
    let mut http = Http::new(vec![]).await;
    let (store, incoming, calls) = legacy_fixture(&mut http).await;
    let original = serde_json::to_value(store.data.lock().unwrap().clone()).unwrap();
    for phase in ["polling", "preparation", "cancellation"] {
        let client = Client::builder("fixture")
            .base_url(&http.url)
            .max_retries(4)
            .build();
        let mut options = TimestampReconciliationOptions::default();
        if phase == "polling" {
            options.timeout = Duration::from_secs(1);
        } else {
            options.preparation_timeout = Some(Duration::from_secs(1));
        }
        let s = store.clone();
        let request = timestamp_request(&incoming);
        let before = http.poll_after.lock().unwrap().len();
        let task = tokio::spawn(async move {
            prepare_timestamp_reconciliation(&client, s, request, options).await
        });
        let _unanswered = http.next().await;
        if phase == "cancellation" {
            task.abort();
            assert!(matches!(task.await,Err(error) if error.is_cancelled()));
        } else {
            let error = match task.await.unwrap() {
                Ok(_) => panic!("pending poll unexpectedly succeeded"),
                Err(error) => error.to_remote(),
            };
            assert_eq!(error.details["phase"], phase);
            assert_eq!(error.details["outcome"], "not_applied");
        }
        assert_eq!(http.poll_after.lock().unwrap().len(), before + 1);
        assert_eq!(
            serde_json::to_value(store.data.lock().unwrap().clone()).unwrap(),
            original
        );
    }
    assert_eq!(store.reconciliation_writes.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn timestamp_probe_rejects_invalid_wait_and_total_deadline_before_io() {
    let store = Arc::new(Store::default());
    let mut incoming = legacy_event();
    incoming.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
    let client = Client::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .build();
    for (wait, budget) in [
        (26, None),
        (1, Some(Duration::ZERO)),
        (1, Some(Duration::from_secs(361))),
    ] {
        let mut options = TimestampReconciliationOptions::default();
        options.polling_wait = wait;
        options.preparation_timeout = budget;
        let error = prepare_timestamp_reconciliation(
            &client,
            store.clone(),
            timestamp_request(&incoming),
            options,
        )
        .await
        .err()
        .unwrap()
        .to_remote();
        assert_eq!(error.details["reason"], "invalid_request_or_bounds");
    }
}

#[cfg(feature = "gateway")]
#[path = "support/gateway_drain.rs"]
mod draining;

async fn failed_retirement_fixture(
    http: &mut Http,
    started: bool,
) -> (Arc<Store>, Event, Arc<AtomicUsize>) {
    let (store, canonical, calls) = legacy_fixture(http).await;
    {
        let mut data = store.data.lock().unwrap();
        let original_hash = event_hash(&canonical);
        let rejected_hash = event_hash(&data.records[0].event);
        let record = &mut data.records[0];
        record.event = canonical.clone();
        record.source = Some(IngressSource::Gateway);
        record.status.stage = if started {
            ReceiptStage::Started
        } else {
            ReceiptStage::Failed
        };
        record.status.error = if started {
            None
        } else {
            Some(
                Error::PartialWrite {
                    completed: vec![json!({"receipt_id":"fixture-confirmed-first-step"})],
                    failed_step: 2,
                    source: Box::new(Error::Remote(RemoteError {
                        kind: "gateway_request".into(),
                        message: "fixture submitted response unknown".into(),
                        status: None,
                        retry_after: None,
                        details: std::collections::BTreeMap::from([
                            ("submitted".into(), true.into()),
                            ("outcome".into(), "unknown".into()),
                        ]),
                    })),
                }
                .to_remote(),
            )
        };
        data.polling_checkpoint = Some(90);
        let conflict = &mut data.conflicts[0];
        conflict.existing_fingerprint = original_hash;
        conflict.incoming_fingerprint = rejected_hash;
        conflict.retained_stage = Some(ReceiptStage::Started);
        conflict.existing_source = IngressSource::Gateway;
        conflict.incoming_source = IngressSource::Webhook;
        conflict.differences[0].existing_type = PayloadType::String;
        conflict.differences[0].incoming_type = PayloadType::Null;
    }
    (store, canonical, calls)
}
async fn prepare_retirement_fixture(
    http: &mut Http,
    store: Arc<Store>,
    incoming: &Event,
) -> Result<PreparedEventRetirement> {
    let retained_hash = event_hash(&store.data.lock().unwrap().records[0].event);
    let request = EventRetirementRequest::new(bot_id(), 100, retained_hash, event_hash(incoming));
    let client = Client::builder("fixture").base_url(&http.url).build();
    let task = tokio::spawn(async move {
        prepare_event_retirement(&client, store, request, Default::default()).await
    });
    http.next()
        .await
        .send(json!({"events":[incoming],"last":100}))
        .unwrap();
    task.await.unwrap()
}
#[tokio::test]
async fn explicit_failed_retirement_preserves_original_error_and_unknown_assessment_across_restart()
{
    for started in [false, true] {
        let mut http = Http::new(vec![]).await;
        let (store, canonical, calls) = failed_retirement_fixture(&mut http, started).await;
        let original = serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap();
        let prepared = prepare_retirement_fixture(&mut http, store.clone(), &canonical)
            .await
            .unwrap();
        assert_eq!(prepared.evidence().state, EventRetirementState::Prepared);
        assert!(prepared.evidence().assessment.is_none());
        assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 0);
        let approved = prepared
            .approve(
                OperationId::new("fixture/explicit-approval").unwrap(),
                EventRetirementAssessment::AcceptUnknown,
                OperationId::new("fixture/retained-error-and-receipts").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(approved.state, EventRetirementState::Approved);
        assert_eq!(
            serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap(),
            original
        );
        let mut config = IngressConfig::default();
        config.admission_policy = IngressAdmissionPolicy::PollingOnly;
        let bot = builder(&http)
            .configure(|settings| settings.persistence.ingress = Some(config))
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "must not replay", |_| async {
                panic!("retired handler replayed")
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let input = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        input.wait_ready().await.unwrap();
        assert_eq!(
            input.admit(verified(canonical.clone())).await.status,
            AdmissionStatus::PollingRequired
        );
        let first = http.next().await;
        let receipt = input.receipt(100).unwrap();
        first.send(json!({"events":[],"last":100})).unwrap();
        let poll = http.next().await;
        assert!(input.status().committed_cursor < 100);
        assert_eq!(input.retirements()[0].state, EventRetirementState::Approved);
        poll.send(json!({"events":[canonical],"last":100})).unwrap();
        until(|| input.status().committed_cursor == 100).await;
        assert_eq!(
            receipt.status().stage,
            if started {
                ReceiptStage::Started
            } else {
                ReceiptStage::Failed
            }
        );
        assert!(receipt.status().committed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
        for (decision, retained) in store.retirement_audit.lock().unwrap().iter() {
            assert_eq!(serde_json::to_value(retained).unwrap(), original);
            assert_eq!(
                decision.assessment,
                Some(EventRetirementAssessment::AcceptUnknown)
            );
        }
        assert_eq!(store.retirement_audit.lock().unwrap().len(), 2);
        handle.stop();
        run.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn operator_retirement_atomic_failures_and_stale_evidence_never_replay() {
    for failure in [1, 2, 3, 4] {
        let mut http = Http::new(vec![]).await;
        let (store, canonical, calls) = failed_retirement_fixture(&mut http, false).await;
        let original = serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap();
        let prepared = prepare_retirement_fixture(&mut http, store.clone(), &canonical)
            .await
            .unwrap();
        store.fail_retirement.store(failure, Ordering::SeqCst);
        let result = prepared
            .approve(
                OperationId::new("fixture/approved").unwrap(),
                EventRetirementAssessment::AcceptUnknown,
                OperationId::new("fixture/assessment").unwrap(),
            )
            .await;
        if failure < 3 {
            let error = result.unwrap_err();
            assert_eq!(error.status_code(), Some(503));
            assert_eq!(error.retry_after(), Some(Duration::from_secs_f64(0.2)));
            assert_eq!(
                store.data.lock().unwrap().event_retirements.is_empty(),
                failure == 1
            );
        } else {
            result.unwrap();
        }
        assert_eq!(
            serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap(),
            original
        );
        if failure == 1 {
            let bot = builder(&http)
                .configure(|settings| settings.persistence.store = Some(store.clone()))
                .build()
                .expect("valid bot configuration");
            assert!(bot.run().await.is_err());
            continue;
        }
        if failure == 2 {
            store.fail_retirement.store(0, Ordering::SeqCst);
        }
        let bot = builder(&http)
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .command("ping", "no replay", |_| async {
                panic!("replayed failed work")
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let input = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        input.wait_ready().await.unwrap();
        http.next()
            .await
            .send(json!({"events":[canonical.clone()],"last":100}))
            .unwrap();
        if failure >= 3 {
            let error = tokio::time::timeout(Duration::from_secs(3), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(error.status_code(), Some(503));
            {
                let snapshot = store.data.lock().unwrap();
                assert_eq!(snapshot.polling_checkpoint, Some(90));
                assert_eq!(
                    serde_json::to_value(&snapshot.records[0]).unwrap(),
                    original
                );
                assert_eq!(
                    snapshot.event_retirements[0].state,
                    if failure == 3 {
                        EventRetirementState::Approved
                    } else {
                        EventRetirementState::PollingConfirmed
                    }
                );
            }
            store.fail_retirement.store(0, Ordering::SeqCst);
            let bot = builder(&http)
                .configure(|settings| settings.persistence.store = Some(store.clone()))
                .command("ping", "no replay", |_| async { panic!("replayed") })
                .build()
                .expect("valid bot configuration");
            let handle = bot.handle();
            let input = handle.ingress().unwrap();
            let run = tokio::spawn(bot.run());
            input.wait_ready().await.unwrap();
            http.next()
                .await
                .send(json!({"events":[canonical],"last":100}))
                .unwrap();
            until(|| input.status().committed_cursor == 100).await;
            handle.stop();
            run.await.unwrap().unwrap();
        } else {
            until(|| input.status().committed_cursor == 100).await;
            handle.stop();
            run.await.unwrap().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
        for (_, record) in store.retirement_audit.lock().unwrap().iter() {
            assert_eq!(serde_json::to_value(record).unwrap(), original);
        }
    }
    let mut http = Http::new(vec![]).await;
    let (store, canonical, _) = failed_retirement_fixture(&mut http, false).await;
    let prepared = prepare_retirement_fixture(&mut http, store.clone(), &canonical)
        .await
        .unwrap();
    store.data.lock().unwrap().records[0].status.error =
        Some(Error::Config("changed failure facts".into()).to_remote());
    assert!(prepared
        .approve(
            OperationId::new("stale").unwrap(),
            EventRetirementAssessment::AttestedNotApplied,
            OperationId::new("proof").unwrap()
        )
        .await
        .is_err());
    assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn operator_retirement_refuses_changed_payload_and_completed_timestamp_api_remains_narrow() {
    let mut http = Http::new(vec![]).await;
    let (store, canonical, _) = failed_retirement_fixture(&mut http, false).await;
    let client = Client::builder("fixture").base_url(&http.url).build();
    let request = TimestampReconciliationRequest::new(
        bot_id(),
        100,
        event_hash(&canonical),
        event_hash(&canonical),
    );
    assert!(
        prepare_timestamp_reconciliation(&client, store.clone(), request, Default::default())
            .await
            .is_err()
    );
    for field in ["author_id", "server_id", "channel_id", "id", "content"] {
        let mut changed = canonical.clone();
        changed.data[field] = json!("different");
        assert!(
            prepare_retirement_fixture(&mut http, store.clone(), &changed)
                .await
                .is_err()
        );
    }
    assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 0);
    assert!(store.data.lock().unwrap().event_retirements.is_empty());
}
#[tokio::test]
async fn polling_only_admission_ignores_both_webhook_timestamp_representations() {
    for supplied_timestamp in [false, true] {
        let mut http = Http::new(vec![]).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let mut policy = IngressConfig::default();
        policy.admission_policy = IngressAdmissionPolicy::PollingOnly;
        let bot = builder(&http)
            .configure(|settings| settings.persistence.ingress = Some(policy))
            .command("ping", "canonical", move |_| {
                let called = called.clone();
                async move {
                    called.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let input = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        input.wait_ready().await.unwrap();
        let poll = http.next().await;
        let mut canonical = event(10, "ping");
        canonical.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
        let first = if supplied_timestamp {
            canonical.clone()
        } else {
            event(10, "ping")
        };
        assert_eq!(
            input.admit(verified(first)).await.status,
            AdmissionStatus::PollingRequired
        );
        assert!(input.receipt(10).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        poll.send(json!({"events":[canonical],"last":10})).unwrap();
        until(|| input.status().committed_cursor == 10).await;
        assert_eq!(
            input
                .admit(verified(event(10, "changed-content")))
                .await
                .status,
            AdmissionStatus::PollingRequired
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(input.status().conflicts, 0);
        handle.stop();
        run.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn retained_execution_requires_explicit_unknown_acceptance_and_normal_polling_coverage() {
    let mut http = Http::new(vec![]).await;
    let (store, canonical, calls) = failed_retirement_fixture(&mut http, false).await;
    let original = serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap();
    async fn prepare_missing(
        http: &mut Http,
        store: Arc<Store>,
        canonical: &Event,
    ) -> PreparedEventRetirement {
        let client = Client::builder("fixture").base_url(&http.url).build();
        let request =
            EventRetirementRequest::retained_execution(bot_id(), 100, event_hash(canonical));
        let task = tokio::spawn(async move {
            prepare_event_retirement(&client, store, request, Default::default()).await
        });
        http.next()
            .await
            .send(json!({"events":[],"last":110}))
            .unwrap();
        task.await.unwrap().unwrap()
    }
    let prepared = prepare_missing(&mut http, store.clone(), &canonical).await;
    assert!(!prepared.evidence().target_observed);
    assert_eq!(
        prepared.evidence().evidence,
        EventRetirementEvidence::RetainedExecutionCoverage
    );
    assert!(prepared
        .approve(
            OperationId::new("unproven-success").unwrap(),
            EventRetirementAssessment::AttestedApplied,
            OperationId::new("fixture").unwrap()
        )
        .await
        .is_err());
    assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 0);
    let prepared = prepare_missing(&mut http, store.clone(), &canonical).await;
    prepared
        .approve(
            OperationId::new("accept-original-unknown").unwrap(),
            EventRetirementAssessment::AcceptUnknown,
            OperationId::new("operator-reviewed-original-error-and-partial-receipts").unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(90));
    assert_eq!(
        serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap(),
        original
    );
    let mut config = IngressConfig::default();
    config.admission_policy = IngressAdmissionPolicy::PollingOnly;
    let bot = builder(&http)
        .configure(|settings| settings.persistence.ingress = Some(config))
        .configure(|settings| settings.persistence.store = Some(store.clone()))
        .command("ping", "must not replay", |_| async {
            panic!("replayed unknown work")
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let input = handle.ingress().unwrap();
    let run = tokio::spawn(bot.run());
    input.wait_ready().await.unwrap();
    let receipt = input.receipt(100).unwrap();
    http.next()
        .await
        .send(json!({"events":[],"last":99}))
        .unwrap();
    let second = http.next().await;
    assert!(input.status().committed_cursor < 100);
    assert_eq!(input.retirements()[0].state, EventRetirementState::Approved);
    second.send(json!({"events":[],"last":110})).unwrap();
    until(|| input.status().committed_cursor == 110).await;
    assert_eq!(receipt.status().stage, ReceiptStage::Failed);
    assert!(receipt.status().committed);
    assert_eq!(
        receipt.status().error.as_ref().unwrap().to_json(),
        original["status"]["error"]
    );
    {
        let audit = store.retirement_audit.lock().unwrap();
        let decision = &audit.last().unwrap().0;
        assert_eq!(
            decision.assessment,
            Some(EventRetirementAssessment::AcceptUnknown)
        );
        assert_eq!(decision.target_observed_at_confirmation, Some(false));
        assert_eq!(decision.confirmation_watermark, Some(110));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    handle.stop();
    run.await.unwrap().unwrap();
}
#[tokio::test]
async fn retained_execution_does_not_accept_a_changed_returned_event_or_unproved_range() {
    for invalid in ["content", "range"] {
        let mut http = Http::new(vec![]).await;
        let (store, canonical, _) = failed_retirement_fixture(&mut http, false).await;
        let request =
            EventRetirementRequest::retained_execution(bot_id(), 100, event_hash(&canonical));
        let client = Client::builder("fixture").base_url(&http.url).build();
        let s = store.clone();
        let task = tokio::spawn(async move {
            prepare_event_retirement(&client, s, request, Default::default()).await
        });
        let mut changed = canonical;
        changed.data["author_id"] = json!(Uuid::from_u128(999));
        http.next()
            .await
            .send(if invalid == "content" {
                json!({"events":[changed],"last":110})
            } else {
                json!({"events":[],"last":99})
            })
            .unwrap();
        assert!(task.await.unwrap().is_err());
        assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 0);
        assert_eq!(store.data.lock().unwrap().polling_checkpoint, Some(90));
    }
}

#[tokio::test]
async fn retirement_preparation_unavailable_identity_mismatch_and_cancellation_write_nothing() {
    let mut http = Http::new(vec![]).await;
    let (store, canonical, calls) = failed_retirement_fixture(&mut http, false).await;
    let original = serde_json::to_value(store.data.lock().unwrap().clone()).unwrap();
    let request = EventRetirementRequest::new(
        bot_id(),
        100,
        event_hash(&canonical),
        event_hash(&canonical),
    );
    let client = Client::builder("fixture").base_url(&http.url).build();
    let mut wrong = request.clone();
    wrong.bot_id = Uuid::new_v4();
    assert_eq!(
        prepare_event_retirement(&client, store.clone(), wrong, Default::default())
            .await
            .err()
            .unwrap()
            .to_json()["reason"],
        "authenticated_bot_mismatch"
    );
    let s = store.clone();
    let c = client.clone();
    let r = request.clone();
    let missing =
        tokio::spawn(async move { prepare_event_retirement(&c, s, r, Default::default()).await });
    http.next()
        .await
        .send(json!({"events":[],"last":120}))
        .unwrap();
    assert_eq!(
        missing.await.unwrap().err().unwrap().to_json()["reason"],
        "authoritative_event_unavailable"
    );
    let s = store.clone();
    let c = client.clone();
    let cancelled =
        tokio::spawn(
            async move { prepare_event_retirement(&c, s, request, Default::default()).await },
        );
    let _pending_poll = http.next().await;
    cancelled.abort();
    assert!(matches!(cancelled.await, Err(error) if error.is_cancelled()));
    assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        serde_json::to_value(store.data.lock().unwrap().clone()).unwrap(),
        original
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn retirement_without_conflict_reports_times_out_or_retains_failed_checkpoint_for_restart() {
    for missing_confirmation in [false, true] {
        let mut http = Http::new(vec![]).await;
        let (store, canonical, calls) = failed_retirement_fixture(&mut http, false).await;
        store.data.lock().unwrap().conflicts.clear();
        let original = serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap();
        prepare_retirement_fixture(&mut http, store.clone(), &canonical)
            .await
            .unwrap()
            .approve(
                OperationId::new("operator/test").unwrap(),
                EventRetirementAssessment::AcceptUnknown,
                OperationId::new("operator/evidence").unwrap(),
            )
            .await
            .unwrap();
        let mut config = IngressConfig::default();
        config.conflict_recovery_timeout = Duration::from_millis(100);
        let bot = builder(&http)
            .configure(|settings| settings.persistence.ingress = Some(config))
            .configure(|settings| settings.persistence.store = Some(store.clone()))
            .build()
            .expect("valid bot configuration");
        let input = bot.handle().ingress().unwrap();
        let run = tokio::spawn(bot.run());
        input.wait_ready().await.unwrap();
        let poll = http.next().await;
        if !missing_confirmation {
            store.fail_checkpoint.store(1, Ordering::SeqCst);
            poll.send(json!({"events":[canonical.clone()],"last":100}))
                .unwrap();
        }
        let report = pipeline_report(
            tokio::time::timeout(Duration::from_secs(3), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
        );
        assert_eq!(report.committed_cursor, 90);
        assert_eq!(
            serde_json::to_value(&store.data.lock().unwrap().records[0]).unwrap(),
            original
        );
        if missing_confirmation {
            let error = report.intake_error.unwrap();
            assert_eq!(error.kind, "event_retirement");
            assert_eq!(error.details["reason"], "polling_confirmation_timeout");
            assert_eq!(store.retirement_writes.load(Ordering::SeqCst), 1);
        } else {
            assert!(report.cursor_error.is_some());
            assert_eq!(
                store.data.lock().unwrap().event_retirements[0].state,
                EventRetirementState::PollingConfirmed
            );
            store.fail_checkpoint.store(0, Ordering::SeqCst);
            let bot = builder(&http)
                .configure(|settings| settings.persistence.store = Some(store.clone()))
                .build()
                .expect("valid bot configuration");
            let handle = bot.handle();
            let input = handle.ingress().unwrap();
            let run = tokio::spawn(bot.run());
            input.wait_ready().await.unwrap();
            http.next()
                .await
                .send(json!({"events":[],"last":100}))
                .unwrap();
            until(|| input.status().committed_cursor == 100).await;
            handle.stop();
            run.await.unwrap().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
    }
}
