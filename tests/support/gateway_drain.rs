use super::*;
use futures_util::{SinkExt, StreamExt};
use rappin::gateway::GatewayRequestConfig;
use tokio_tungstenite::{accept_async, tungstenite::Message};
async fn serve(
    mode: &'static str,
) -> (
    String,
    mpsc::UnboundedReceiver<()>,
    mpsc::UnboundedSender<()>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (seen, received) = mpsc::unbounded_channel();
    let (release, mut control) = mpsc::unbounded_channel();
    let posts = Arc::new(AtomicUsize::new(0));
    let count = posts.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":100})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(
            json!({"op":"ready","seq":1,"requests":true})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            json!({"op":"event","d":event(2,"work")}).to_string().into(),
        ))
        .await
        .unwrap();
        if mode == "failure" {
            ws.send(Message::Text(
                json!({"op":"event","d":event(3,"fail")}).to_string().into(),
            ))
            .await
            .unwrap();
        }
        let mut pending = None;
        loop {
            tokio::select! {
                message=ws.next()=>{
                    let Some(Ok(Message::Text(text)))=message else {return;};let value:Value=serde_json::from_str(&text).unwrap();
                    match value["op"].as_str() {
                        Some("heartbeat")=>{if ws.send(Message::Text(json!({"op":"heartbeat_ack"}).to_string().into())).await.is_err(){return;}},
                        Some("request")=>{
                            if value["method"]=="POST" {
                                count.fetch_add(1,Ordering::SeqCst);pending=Some(value["id"].clone());seen.send(()).unwrap();
                                if mode=="backpressure" {for id in 3..90 {if ws.send(Message::Text(json!({"op":"event","d":event(id,"noop")}).to_string().into())).await.is_err(){return;}}}
                            }else {assert_eq!(value["method"],"PATCH");if mode=="worker_partial" {return;}ws.send(Message::Text(json!({"op":"response","id":value["id"],"status":200,"body":{"edited":true}}).to_string().into())).await.unwrap();}
                        },_=>(),
                    }
                },
                Some(())=control.recv()=>{if let Some(id)=pending.take() {ws.send(Message::Text(json!({"op":"response","id":id,"status":200,"body":{"created":true}}).to_string().into())).await.unwrap();}},
            }
        }
    });
    (url, received, release, posts, task)
}
#[tokio::test]
async fn admitted_requests_survive_intake_stop_and_keep_their_followup_edit() {
    for shared in [false, true] {
        for mode in ["stop", "failure", "backpressure", "conflict"] {
            if mode == "conflict" && !shared {
                continue;
            }
            eprintln!("drain fixture shared={shared} mode={mode}");
            let http = Http::new(vec![]).await;
            let (url, mut seen, release, posts, server) = serve(mode).await;
            let path = std::env::temp_dir().join(format!("rappin-drain-{}", Uuid::new_v4()));
            rappin::events::EventCursor::new(&path)
                .save(1)
                .await
                .unwrap();
            let failed = Arc::new(Notify::new());
            let notify = failed.clone();
            let completed = Arc::new(AtomicUsize::new(0));
            let done = completed.clone();
            let mut b = Bot::builder("fixture")
                .base_url(&http.url)
                .sync_commands(false)
                .log(false)
                .cursor_file(&path)
                .gateway(url)
                .drain_timeout(Some(Duration::from_secs(2)))
                .pending_batches(if mode == "failure" { 2 } else { 1 })
                .command("work", "fixture", move |ctx| {
                    let done = done.clone();
                    async move {
                        let client = ctx.bot().unwrap().client().clone();
                        client
                            .request_once(
                                reqwest::Method::POST,
                                "/mutate",
                                Some(json!({"once":true})),
                            )
                            .await?;
                        client
                            .request_once(
                                reqwest::Method::PATCH,
                                "/mutate/id",
                                Some(json!({"edit":true})),
                            )
                            .await?;
                        done.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .command("fail", "fixture", move |_| {
                    let notify = notify.clone();
                    async move {
                        notify.notified().await;
                        Err(YappinError::Config("fixture handler failure".into()))
                    }
                })
                .command("noop", "fixture", |_| async { Ok(()) });
            if shared {
                b = b.ingress_store(Arc::new(Store::default()));
            }
            let bot = b.build();
            let handle = bot.handle();
            handle
                .client()
                .configure_gateway_requests(Some(GatewayRequestConfig::default()))
                .unwrap();
            let ingress = handle.ingress();
            let run = tokio::spawn(bot.run());
            tokio::time::timeout(Duration::from_secs(3), seen.recv())
                .await
                .unwrap()
                .unwrap();
            match mode {
                "failure" => failed.notify_one(),
                "conflict" => {
                    let mut changed = event(2, "work");
                    changed.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
                    assert_eq!(
                        ingress
                            .as_ref()
                            .unwrap()
                            .admit(verified(changed))
                            .await
                            .status,
                        AdmissionStatus::Error
                    );
                }
                "backpressure" => {
                    until(|| {
                        handle.transport_status().fallback == Some(FallbackReason::Backpressure)
                    })
                    .await;
                    handle.stop();
                }
                _ => handle.stop(),
            }
            until(|| handle.is_draining()).await;
            assert!(handle.client().gateway_request_status().connected);
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            release.send(()).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(3), run)
                .await
                .unwrap()
                .unwrap();
            if matches!(mode, "failure" | "conflict") {
                assert!(result.is_err());
            } else if shared && mode == "backpressure" {
                if let Err(error) = result {
                    let report = pipeline_report(error);
                    assert!(report.failed_events.iter().all(|e| e.event_id > 2));
                    assert!(report.completed_uncommitted_events.contains(&2));
                }
            } else {
                result.unwrap();
            }
            assert_eq!(completed.load(Ordering::SeqCst), 1);
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            assert_eq!(http.posts.load(Ordering::SeqCst), 0);
            assert!(!handle.is_draining());
            assert!(!handle.client().gateway_request_status().connected);
            if let Some(ingress) = ingress {
                assert_eq!(
                    ingress.receipt(2).unwrap().status().stage,
                    ReceiptStage::Completed
                );
            }
            server.abort();
            let _ = std::fs::remove_file(path);
        }
    }
}
#[tokio::test]
async fn drain_timeout_and_hard_cancellation_retain_unknown_submitted_work() {
    for shared in [false, true] {
        for cancelled in [false, true] {
            let http = Http::new(vec![]).await;
            let (url, mut seen, _release, posts, server) = serve("timeout").await;
            let path =
                std::env::temp_dir().join(format!("rappin-drain-timeout-{}", Uuid::new_v4()));
            rappin::events::EventCursor::new(&path)
                .save(1)
                .await
                .unwrap();
            let callbacks = Arc::new(AtomicUsize::new(0));
            let called = callbacks.clone();
            let store = Arc::new(Store::default());
            let mut b = Bot::builder("fixture")
                .base_url(&http.url)
                .sync_commands(false)
                .log(false)
                .cursor_file(&path)
                .gateway(url)
                .drain_timeout(Some(Duration::from_millis(50)))
                .command("work", "fixture", |ctx| async move {
                    ctx.bot()
                        .unwrap()
                        .client()
                        .request_once(reqwest::Method::POST, "/mutate", None)
                        .await?;
                    Ok(())
                })
                .on_command_failure(move |_, _| {
                    let called = called.clone();
                    async move {
                        called.fetch_add(1, Ordering::SeqCst);
                        Ok(FailureDisposition::Stop)
                    }
                });
            if shared {
                b = b.ingress_store(store.clone());
            }
            let bot = b.build();
            let handle = bot.handle();
            let ingress = handle.ingress();
            handle
                .client()
                .configure_gateway_requests(Some(GatewayRequestConfig::default()))
                .unwrap();
            let run = tokio::spawn(bot.run());
            tokio::time::timeout(Duration::from_secs(3), seen.recv())
                .await
                .unwrap()
                .unwrap();
            if cancelled {
                run.abort();
                assert!(matches!(run.await,Err(error) if error.is_cancelled()));
            } else {
                handle.stop();
                let report = pipeline_report(
                    tokio::time::timeout(Duration::from_secs(3), run)
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap_err(),
                );
                let failed = report
                    .failed_events
                    .iter()
                    .find(|e| e.event_id == 2)
                    .expect("started work retained");
                assert_eq!(failed.error.kind, "event_drain");
                assert_eq!(failed.error.details["outcome"], "unknown");
                assert_eq!(report.committed_cursor, 1);
            }
            assert_eq!(callbacks.load(Ordering::SeqCst), 0);
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            assert_eq!(http.posts.load(Ordering::SeqCst), 0);
            let status = handle.client().gateway_request_status();
            assert!(!status.connected);
            assert_eq!(
                status.last_disconnect.unwrap().details["reason"],
                if cancelled {
                    "run_cancelled"
                } else {
                    "drain_timeout"
                }
            );
            if let Some(ingress) = ingress {
                assert_eq!(
                    ingress.receipt(2).unwrap().status().stage,
                    ReceiptStage::Failed
                );
                let data = store.data.lock().unwrap();
                assert_eq!(data.records[0].status.stage, ReceiptStage::Started);
                assert_eq!(data.polling_checkpoint, Some(1));
            }
            server.abort();
            let _ = std::fs::remove_file(path);
        }
    }
}

// A controlled event source makes transport arrival order deterministic.
async fn source() -> (
    String,
    mpsc::UnboundedSender<Event>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (send, mut frames) = mpsc::unbounded_channel::<Event>();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":100})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(
            json!({"op":"ready","seq":0,"requests":true})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        loop {
            tokio::select! {
                Some(event)=frames.recv()=>{if ws.send(Message::Text(json!({"op":"event","d":event}).to_string().into())).await.is_err(){break;}},
                frame=ws.next()=>{let Some(Ok(Message::Text(text)))=frame else {break;};let value:Value=serde_json::from_str(&text).unwrap();if value["op"]=="heartbeat" && ws.send(Message::Text(json!({"op":"heartbeat_ack"}).to_string().into())).await.is_err(){break;}}
            }
        }
    });
    (url, send, task)
}
#[tokio::test]
async fn timestamp_conflicts_keep_started_work_in_both_real_transport_orders() {
    for gateway_first in [false, true] {
        let mut http = Http::new(vec![]).await;
        let (url, send, server) = source().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let release = Arc::new(Notify::new());
        let released = release.clone();
        let bot = builder(&http)
            .gateway(url)
            .drain_timeout(Some(Duration::from_secs(2)))
            .command("ping", "fixture", move |_| {
                let called = called.clone();
                let released = released.clone();
                async move {
                    called.fetch_add(1, Ordering::SeqCst);
                    released.notified().await;
                    Ok(())
                }
            })
            .build();
        let handle = bot.handle();
        handle
            .client()
            .configure_gateway_requests(Some(GatewayRequestConfig::default()))
            .unwrap();
        let input = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        input.wait_ready().await.unwrap();
        let _poll = http.next().await;
        let mut canonical = event(10, "ping");
        canonical.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
        if gateway_first {
            send.send(canonical.clone()).unwrap();
        } else {
            assert_eq!(
                input.admit(verified(event(10, "ping"))).await.status,
                AdmissionStatus::Accepted
            );
        }
        until(|| calls.load(Ordering::SeqCst) == 1).await;
        if gateway_first {
            assert_eq!(
                input.admit(verified(event(10, "ping"))).await.status,
                AdmissionStatus::Error
            );
        } else {
            send.send(canonical).unwrap();
        }
        until(|| handle.is_draining()).await;
        assert_eq!(
            input.receipt(10).unwrap().status().stage,
            ReceiptStage::Started
        );
        assert!(handle.client().gateway_request_status().connected);
        release.notify_one();
        let report = pipeline_report(
            tokio::time::timeout(Duration::from_secs(3), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
        );
        assert_eq!(report.committed_cursor, 0);
        assert!(report.completed_uncommitted_events.contains(&10));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            input.conflicts()[0].incoming_source,
            if gateway_first {
                IngressSource::Webhook
            } else {
                IngressSource::Gateway
            }
        );
        assert_eq!(
            input.conflicts()[0].retained_stage,
            Some(ReceiptStage::Started)
        );
        assert_eq!(http.posts.load(Ordering::SeqCst), 0);
        server.abort();
    }
}
#[tokio::test]
async fn polling_authority_ignores_real_gateway_and_webhook_arrival_order() {
    for gateway_first in [false, true] {
        let mut http = Http::new(vec![]).await;
        let (url, send, server) = source().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let mut config = IngressConfig::default();
        config.admission_policy = IngressAdmissionPolicy::PollingOnly;
        let bot = builder(&http)
            .shared_ingress(config)
            .gateway(url)
            .command("ping", "fixture", move |_| {
                let called = called.clone();
                async move {
                    called.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build();
        let handle = bot.handle();
        handle
            .client()
            .configure_gateway_requests(Some(GatewayRequestConfig::default()))
            .unwrap();
        let input = handle.ingress().unwrap();
        let run = tokio::spawn(bot.run());
        input.wait_ready().await.unwrap();
        let poll = http.next().await;
        let mut canonical = event(10, "ping");
        canonical.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
        if gateway_first {
            send.send(canonical.clone()).unwrap();
            until(|| handle.transport_status().received_cursor == 10).await;
        }
        assert_eq!(
            input.admit(verified(event(10, "ping"))).await.status,
            AdmissionStatus::PollingRequired
        );
        if !gateway_first {
            send.send(canonical.clone()).unwrap();
            until(|| handle.transport_status().received_cursor == 10).await;
        }
        assert!(input.receipt(10).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(handle.client().gateway_request_status().connected);
        poll.send(json!({"events":[canonical],"last":10})).unwrap();
        until(|| input.status().committed_cursor == 10).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(input.conflicts().is_empty());
        handle.stop();
        run.await.unwrap().unwrap();
        server.abort();
    }
}

#[cfg(feature = "hot-reload")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_requests_drain_and_preserve_partial_or_uncertain_failures() {
    use rappin::hot_reload::{HotReload, WorkerProject};
    struct Project(std::path::PathBuf);
    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let project =
        Project(std::env::temp_dir().join(format!("rappin-drain-worker-{}", Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    std::fs::write(project.0.join("Cargo.toml"),format!("[package]\nname=\"drain-rpc-worker\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin={{package=\"rappin-rs\",path={},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\n",json!(env!("CARGO_MANIFEST_DIR")))).unwrap();
    std::fs::write(project.0.join("src/main.rs"),r#"
use rappin::{*,hot_reload::WorkerCog};
#[tokio::main] async fn main()->Result<()> {
 WorkerCog::new("drain-rpc").single_attempt_api().command("work","fixture",|ctx|async move {
   let first=ctx.api_once("POST","/mutate",None).await?;
   match ctx.api_once("PATCH","/mutate/id",None).await {Ok(_)=>Ok(()),Err(source)=>Err(YappinError::PartialWrite{completed:vec![first],failed_step:2,source:Box::new(source)})}
 }).run().await
}
"#).unwrap();
    for shared in [false, true] {
        for mode in [
            "worker",
            "worker_partial",
            "worker_timeout",
            "conflict",
            "backpressure",
        ] {
            if mode == "conflict" && !shared {
                continue;
            }
            let http = Http::new(vec![]).await;
            let (url, mut seen, release, posts, server) = serve(mode).await;
            let path = project.0.join(format!("cursor-{shared}-{mode}.json"));
            rappin::events::EventCursor::new(&path)
                .save(1)
                .await
                .unwrap();
            let errors = Arc::new(Mutex::new(vec![]));
            let observed = errors.clone();
            let mut b = Bot::builder("fixture")
                .base_url(&http.url)
                .sync_commands(false)
                .log(false)
                .cursor_file(path)
                .gateway(url)
                .drain_timeout(Some(if mode == "worker_timeout" {
                    Duration::from_millis(80)
                } else {
                    Duration::from_secs(3)
                }))
                .on_command_failure(move |_, failure| {
                    let observed = observed.clone();
                    async move {
                        observed.lock().unwrap().push(failure.error.to_json());
                        Ok(FailureDisposition::Stop)
                    }
                });
            if shared {
                b = b.ingress_store(Arc::new(Store::default()));
            }
            let bot = b.build();
            let handle = bot.handle();
            bot.cache()
                .set_me(serde_json::from_value(support::profile()).unwrap())
                .await;
            let worker = HotReload::load(
                handle.clone(),
                WorkerProject::new(project.0.join("Cargo.toml"), "drain-rpc-worker").offline(true),
            )
            .await
            .unwrap();
            handle
                .client()
                .configure_gateway_requests(Some(GatewayRequestConfig::default()))
                .unwrap();
            let run = tokio::spawn(bot.run());
            tokio::time::timeout(Duration::from_secs(3), seen.recv())
                .await
                .unwrap()
                .unwrap();
            if mode == "conflict" {
                let mut changed = event(2, "work");
                changed.created_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
                assert_eq!(
                    handle
                        .ingress()
                        .unwrap()
                        .admit(verified(changed))
                        .await
                        .status,
                    AdmissionStatus::Error
                );
            } else {
                if mode == "backpressure" {
                    until(|| {
                        handle.transport_status().fallback == Some(FallbackReason::Backpressure)
                    })
                    .await;
                }
                handle.stop();
            }
            until(|| handle.is_draining()).await;
            if mode != "worker_timeout" {
                release.send(()).unwrap();
            }
            let result = tokio::time::timeout(Duration::from_secs(4), run)
                .await
                .unwrap()
                .unwrap();
            match mode {
                "worker" => {
                    result.unwrap();
                    assert!(errors.lock().unwrap().is_empty());
                }
                "conflict" | "backpressure" => {
                    if let Err(error) = result {
                        let report = pipeline_report(error);
                        assert!(report.failed_events.iter().all(|event| event.event_id > 2));
                        assert!(report.completed_uncommitted_events.contains(&2));
                    } else {
                        assert_eq!(mode, "backpressure");
                    }
                    assert!(errors.lock().unwrap().is_empty());
                }
                "worker_partial" => {
                    let report = pipeline_report(result.unwrap_err());
                    assert_eq!(report.failed_events.len(), 1);
                    let errors = errors.lock().unwrap();
                    assert_eq!(errors.len(), 1);
                    assert_eq!(errors[0]["kind"], "partial_write");
                    assert_eq!(errors[0]["completed_steps"].as_array().unwrap().len(), 1);
                    assert_eq!(errors[0]["submitted"], true);
                    assert_eq!(errors[0]["outcome"], "unknown");
                    assert!(errors[0]["reason"].is_string());
                }
                _ => {
                    let report = pipeline_report(result.unwrap_err());
                    assert_eq!(report.failed_events.len(), 1);
                    assert!(errors.lock().unwrap().is_empty());
                    assert_eq!(
                        handle
                            .client()
                            .gateway_request_status()
                            .last_disconnect
                            .unwrap()
                            .details["reason"],
                        "drain_timeout"
                    );
                }
            }
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            assert_eq!(http.posts.load(Ordering::SeqCst), 0);
            worker.unload().await.unwrap();
            server.abort();
        }
    }
}
