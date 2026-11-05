mod support;
use hmac::{Hmac, KeyInit, Mac};
use rappin::*;
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
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
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accept=listener.accept()=>{let (mut socket,_)=accept.unwrap();let tx=tx.clone();let count=count.clone();let responses=responses.clone();connections.spawn(async move {
                        let mut bytes=vec![];let mut buf=[0;4096];loop {let n=socket.read(&mut buf).await.unwrap();if n==0{return;}bytes.extend_from_slice(&buf[..n]);if let Some(end)=bytes.windows(4).position(|b|b==b"\r\n\r\n") {let headers=String::from_utf8_lossy(&bytes[..end]);let size=headers.lines().find_map(|l|l.split_once(':').filter(|(k,_)|k.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.trim().parse::<usize>().unwrap())).unwrap_or(0);if bytes.len()>=end+4+size {break;}}}
                        let request=String::from_utf8_lossy(&bytes);let line=request.lines().next().unwrap();
                        let (status,headers,body)=if line.starts_with("GET /me "){support::response(200,support::profile())}else if line.starts_with("GET /events?"){let (send,receive)=oneshot::channel();if tx.send(send).is_err(){return;}let mut closed=[0u8;1];let value=tokio::select!{result=receive=>{let Ok(value)=result else{return;};value},_=socket.read(&mut closed)=>return};support::response(200,value)}else if line.starts_with("POST "){count.fetch_add(1,Ordering::SeqCst);responses.lock().unwrap().pop_front().expect("unexpected mutation")}else{panic!("unexpected request {line}")};
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
        .shared_ingress(IngressConfig::default())
}

#[tokio::test]
async fn simultaneous_webhooks_polling_sparse_ids_conflicts_and_identity_are_shared() {
    let mut http = Http::new(vec![]).await;
    let calls = Arc::new(Mutex::new(vec![]));
    let c = calls.clone();
    let bot = builder(&http)
        .command("ping", "record", move |ctx| {
            let c = c.clone();
            async move {
                c.lock().unwrap().push(ctx.event.id);
                Ok(())
            }
        })
        .build();
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
        .shared_ingress(config)
        .pending_batches(2)
        .handler_concurrency(1)
        .command("hold", "wait", move |_| {
            let started = started.clone();
            let wait = wait.clone();
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                wait.notified().await;
                Ok(())
            }
        })
        .build();
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
    if let Err(YappinError::EventProcessing { report }) = drained {
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
    async fn checkpoint(&self, _: Uuid, cursor: u64) -> Result<()> {
        if cursor > 0 && self.fail_checkpoint.load(Ordering::SeqCst) > 0 {
            return Err(YappinError::Config(
                "fixture checkpoint persistence failed".into(),
            ));
        }
        let mut data = self.data.lock().unwrap();
        data.polling_checkpoint = Some(cursor);
        data.records.retain(|r| r.event.id > cursor);
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
        .ingress_store(store.clone())
        .pending_batches(3)
        .command("ping", "test", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();
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
            .cursor_file(&path)
            .ingress_store(store.clone())
            .command("ping", "test", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build();
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
                YappinError::EventProcessing { report } => report,
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
        .pending_batches(1)
        .handler_concurrency(1)
        .auto_ack_after(Duration::from_millis(5))
        .command("question", "collect", move |ctx| {
            let r = r.clone();
            async move {
                let collector = ctx
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
        .build();
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
            .handler_concurrency(1)
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
            .build();
        let handle = bot.handle();
        let ingress = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        ingress.wait_ready().await.unwrap();
        http.next().await.send(json!({"events":[event(10,"reject"),event(10,"reject"),event(20,"later")],"last":20})).unwrap();
        if failed_reply {
            let error = run.await.unwrap().unwrap_err();
            let report = match error {
                YappinError::EventProcessing { report } => report,
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
            .ingress_store(store.clone())
            .command("effect", "uncertain", move |_| {
                let notify = notify.clone();
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    notify.notify_one();
                    if cancelled {
                        std::future::pending::<()>().await;
                    }
                    Err(YappinError::PartialWrite {
                        completed: vec![json!({"receipt":"confirmed"})],
                        failed_step: 2,
                        source: Box::new(YappinError::Api {
                            status: 502,
                            message: "unknown write".into(),
                            retry_after: None,
                        }),
                    })
                }
            })
            .build();
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
                YappinError::EventProcessing { report } => report,
                other => panic!("{other:?}"),
            };
            assert_eq!(
                report.failed_events[0].error.details["completed_steps"][0]["receipt"],
                "confirmed"
            );
        }
        let c = calls.clone();
        let bot = builder(&http)
            .ingress_store(store)
            .command("effect", "no replay", move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build();
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
        .ingress_store(store.clone())
        .cursor_file(&path)
        .command("work", "test", move |_| {
            let path = sabotage.clone();
            async move {
                std::fs::remove_file(&path)?;
                std::fs::create_dir(&path)?;
                Ok(())
            }
        })
        .build();
    let ingress = bot.handle().ingress().unwrap();
    let run = tokio::spawn(bot.run());
    ingress.wait_ready().await.unwrap();
    http.next()
        .await
        .send(json!({"events":[event(10,"work")],"last":10}))
        .unwrap();
    let report = match run.await.unwrap().unwrap_err() {
        YappinError::EventProcessing { report } => report,
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
use rappin::{*,hot_reload::WorkerCog};
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
        .build();
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
    handle.remove_cog("shared-worker").await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
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
        .gateway(format!("ws://{address}"))
        .command("ping", "test", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();
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
        YappinError::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(report.committed_cursor, 10);
    assert_eq!(
        report.intake_error.unwrap().details["field"],
        "ingress.payload"
    );
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
        .ingress_store(store)
        .lookup_cache_backend(cache.clone())
        .build();
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
        YappinError::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(report.committed_cursor, 0);
    assert_eq!(report.intake_error.unwrap().kind, "handler_interrupted");
    let receipt = ingress.receipt(10).unwrap().status();
    assert_eq!(receipt.stage, ReceiptStage::Failed);
    assert_eq!(receipt.error.unwrap().details["outcome"], "unknown");
    assert_eq!(http.posts.load(Ordering::SeqCst), 0);
}
