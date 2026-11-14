mod support;
use rappin::*;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use support::{event, profile, response, routed_mock};
use tokio::io::AsyncReadExt;

fn policy(attempts: u32) -> AvailabilityPolicy {
    AvailabilityPolicy::new(attempts, Duration::from_secs(3))
        .unwrap()
        .backoff(Duration::from_millis(2), Duration::from_millis(8))
        .unwrap()
        .jitter_percent(0)
        .unwrap()
}
struct Cursor(PathBuf);
impl Cursor {
    async fn new(value: Option<u64>) -> Self {
        let root =
            std::env::temp_dir().join(format!("rappin-availability-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let path = root.join("cursor.json");
        if let Some(value) = value {
            EventCursor::new(&path).save(value).await.unwrap();
        }
        Self(path)
    }
    async fn last(&self) -> Option<u64> {
        EventCursor::new(&self.0).load().await.unwrap()
    }
}
impl Drop for Cursor {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
async fn run(bot: Bot) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), bot.run())
        .await
        .expect("bot stalled")
}
async fn wait_state(
    receiver: &mut tokio::sync::watch::Receiver<AvailabilityStatus>,
    state: AvailabilityState,
    phase: AvailabilityPhase,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if receiver.borrow().state == state && receiver.borrow().phase == Some(phase) {
                break;
            }
            receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
struct Loads(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Cog for Loads {
    fn name(&self) -> &str {
        "load_probe"
    }
    async fn on_load(&self, _: &Client) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn startup_probe_and_intake_recover_without_restarting_startup_or_handlers() {
    let cursor = Cursor::new(None).await;
    let (base, mut requests, http) = routed_mock(vec![
        (
            "GET /me".into(),
            vec![
                (
                    503,
                    vec![("Retry-After", "0.001")],
                    json!({"error":"busy"}).to_string(),
                ),
                response(200, profile()),
            ],
        ),
        (
            "GET /events".into(),
            vec![
                response(520, json!({"error":"edge"})),
                response(200, json!({"events":[],"last":1})),
                response(521, json!({"error":"edge"})),
                response(522, json!({"error":"edge"})),
                response(200, json!({"events":[event(2,"finish")],"last":2})),
            ],
        ),
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let loads = Arc::new(AtomicUsize::new(0));
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.transport.availability = Some(policy(5)))
        .command("finish", "finish", move |ctx| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                ctx.bot().unwrap().stop();
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    bot.handle().add_cog(Loads(loads.clone())).await.unwrap();
    let handle = bot.handle();
    run(bot).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert_eq!(cursor.last().await, Some(2));
    let mut all = vec![];
    while let Ok(r) = requests.try_recv() {
        all.push(r);
    }
    assert_eq!(all.iter().filter(|r| r.starts_with("GET /me ")).count(), 2);
    assert_eq!(
        all.iter().filter(|r| r.starts_with("GET /events?")).count(),
        5
    );
    assert!(all.iter().all(|r| r.starts_with("GET ")));
    assert_eq!(handle.availability_status().total_retries, 4);
    assert_eq!(handle.availability_status().recoveries, 3);
    http.await.unwrap();
}

#[tokio::test]
async fn startup_attempt_budget_owns_all_retries_and_preserves_original_cause() {
    let cursor = Cursor::new(Some(7)).await;
    let failure = (
        503,
        vec![("Retry-After", "0.001")],
        json!({"error":"still busy"}).to_string(),
    );
    let (base, mut requests, http) = routed_mock(vec![("GET /me".into(), vec![failure; 3])]).await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.transport.availability = Some(policy(3)))
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let error = run(bot).await.unwrap_err();
    let failure = error.availability_failure().unwrap();
    assert_eq!(failure.reason, AvailabilityEnd::AttemptLimit);
    assert_eq!(failure.phase, AvailabilityPhase::StartupIdentity);
    assert_eq!(failure.attempts, 3);
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(error.retry_after(), Some(Duration::from_millis(1)));
    assert_eq!(
        failure.last_error.as_ref().unwrap().message,
        "API error (503): still busy"
    );
    assert_eq!(cursor.last().await, Some(7));
    assert_eq!(handle.availability_status().exhaustions, 1);
    assert_eq!(handle.client().request_metrics().attempts, 3);
    let mut count = 0;
    while let Ok(r) = requests.try_recv() {
        assert!(r.starts_with("GET /me "));
        count += 1;
    }
    assert_eq!(count, 3);
    http.await.unwrap();
}

#[tokio::test]
async fn exhausted_probe_does_not_create_a_checkpoint_or_rerun_load_hooks() {
    let cursor = Cursor::new(None).await;
    let loads = Arc::new(AtomicUsize::new(0));
    let (base, mut requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(520, json!({"error":"edge"})); 2],
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.transport.availability = Some(policy(2)))
        .build()
        .expect("valid bot configuration");
    bot.handle().add_cog(Loads(loads.clone())).await.unwrap();
    let error = run(bot).await.unwrap_err();
    assert_eq!(
        error.availability_failure().unwrap().phase,
        AvailabilityPhase::StartupCursor
    );
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert_eq!(cursor.last().await, None);
    let mut count = 0;
    while let Ok(r) = requests.try_recv() {
        assert!(r.starts_with("GET "));
        count += 1;
    }
    assert_eq!(count, 3);
    http.await.unwrap();
}

#[tokio::test]
async fn intake_exhaustion_retains_saved_cursor_and_dispatches_nothing() {
    let cursor = Cursor::new(Some(7)).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let (base, mut requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            [520, 521, 522]
                .into_iter()
                .map(|s| response(s, json!({"error":"edge unavailable"})))
                .collect(),
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.transport.availability = Some(policy(3)))
        .command("never", "never", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let error = run(bot).await.unwrap_err();
    let availability = error.availability_failure().unwrap();
    assert_eq!(availability.phase, AvailabilityPhase::Intake);
    assert_eq!(availability.attempts, 3);
    assert_eq!(error.status_code(), Some(522));
    match error {
        Error::EventProcessing { report } => {
            assert_eq!(report.committed_cursor, 7);
            assert!(report.failed_events.is_empty());
            assert!(report.intake_error.is_some());
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(cursor.last().await, Some(7));
    let mut polls = 0;
    while let Ok(r) = requests.try_recv() {
        assert!(r.starts_with("GET "));
        if r.starts_with("GET /events?") {
            assert!(r.contains("after=7"));
            polls += 1;
        }
    }
    assert_eq!(polls, 3);
    http.await.unwrap();
}

#[tokio::test]
async fn authentication_decode_and_configuration_failures_are_terminal() {
    for mode in [401, 403, 200] {
        let cursor = Cursor::new(Some(7)).await;
        let (base, mut requests, http) = routed_mock(vec![(
            "GET /me".into(),
            vec![response(mode, json!({"error":"terminal"}))],
        )])
        .await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.transport.availability = Some(policy(5)))
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let error = run(bot).await.unwrap_err();
        assert!(error.availability_failure().is_none());
        assert_eq!(
            handle.availability_status().state,
            AvailabilityState::Terminal
        );
        assert_eq!(handle.availability_status().attempts, 1);
        assert_eq!(cursor.last().await, Some(7));
        if mode != 200 {
            assert_eq!(error.status_code(), Some(mode));
        }
        assert!(requests.recv().await.unwrap().starts_with("GET /me "));
        assert!(requests.try_recv().is_err());
        http.await.unwrap();
    }
    let error = Bot::builder("fixture")
        .base_url("file:///invalid")
        .log(false)
        .configure(|settings| settings.transport.availability = Some(policy(5)))
        .build()
        .err()
        .unwrap();
    assert!(matches!(error, Error::Validation { .. }));
}

#[tokio::test]
async fn stop_cancels_startup_backoff_without_new_reads_or_cursor_changes() {
    let cursor = Cursor::new(Some(7)).await;
    let (base, mut requests, http) = routed_mock(vec![(
        "GET /me".into(),
        vec![response(521, json!({"error":"down"}))],
    )])
    .await;
    let policy = AvailabilityPolicy::new(5, Duration::from_secs(90))
        .unwrap()
        .backoff(Duration::from_secs(20), Duration::from_secs(20))
        .unwrap();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.transport.availability = Some(policy))
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let mut status = handle.subscribe_availability();
    let task = tokio::spawn(bot.run());
    wait_state(
        &mut status,
        AvailabilityState::Waiting,
        AvailabilityPhase::StartupIdentity,
    )
    .await;
    handle.stop();
    let error = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error.availability_failure().unwrap().reason,
        AvailabilityEnd::Cancelled
    );
    assert_eq!(error.status_code(), Some(521));
    assert_eq!(handle.availability_status().total_attempts, 1);
    assert_eq!(handle.availability_status().cancellations, 1);
    assert!(handle.availability_status().backoff_wait < Duration::from_secs(2));
    assert_eq!(cursor.last().await, Some(7));
    requests.recv().await.unwrap();
    assert!(requests.try_recv().is_err());
    http.await.unwrap();
}

async fn hanging_server() -> (
    String,
    Arc<tokio::sync::Notify>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(tokio::sync::Notify::new());
    let a = accepted.clone();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        a.notify_one();
        std::future::pending::<()>().await;
        drop(socket);
    });
    (format!("http://{address}"), accepted, task)
}
#[tokio::test]
async fn in_flight_read_is_cancellable_and_has_a_hard_elapsed_deadline() {
    for cancel in [true, false] {
        let (base, accepted, http) = hanging_server().await;
        let p = AvailabilityPolicy::new(
            5,
            if cancel {
                Duration::from_secs(90)
            } else {
                Duration::from_millis(200)
            },
        )
        .unwrap();
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| settings.transport.availability = Some(p))
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let task = tokio::spawn(bot.run());
        if cancel {
            tokio::time::timeout(Duration::from_secs(2), accepted.notified())
                .await
                .unwrap();
            handle.stop();
        }
        let error = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        let failure = error.availability_failure().unwrap();
        assert_eq!(
            failure.reason,
            if cancel {
                AvailabilityEnd::Cancelled
            } else {
                AvailabilityEnd::ElapsedLimit
            }
        );
        assert_eq!(failure.attempts, 1);
        assert!(failure.last_error.is_none());
        assert_eq!(handle.client().request_metrics().cancelled, 1);
        http.abort();
    }
}

#[tokio::test]
async fn intake_stop_drains_completed_work_and_cancels_backoff_cleanly() {
    let cursor = Cursor::new(Some(7)).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![
                response(200, json!({"events":[event(8,"once")],"last":8})),
                response(520, json!({"error":"down"})),
            ],
        ),
    ])
    .await;
    let p = AvailabilityPolicy::new(5, Duration::from_secs(90))
        .unwrap()
        .backoff(Duration::from_secs(20), Duration::from_secs(20))
        .unwrap();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.transport.availability = Some(p))
        .command("once", "once", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let mut status = handle.subscribe_availability();
    let mut transport = handle.subscribe_transport();
    let task = tokio::spawn(bot.run());
    wait_state(
        &mut status,
        AvailabilityState::Waiting,
        AvailabilityPhase::Intake,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while transport.borrow().committed_cursor != 8 {
            transport.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(cursor.last().await, Some(8));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        handle.availability_status().state,
        AvailabilityState::Cancelled
    );
    http.await.unwrap();
}

#[tokio::test]
async fn client_shared_wait_budget_exhaustion_is_not_rearmed_by_availability() {
    let (base, mut requests, http) = routed_mock(vec![(
        "GET /me".into(),
        vec![(
            503,
            vec![("Retry-After", "30")],
            json!({"error":"busy"}).to_string(),
        )],
    )])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .max_retry_wait(Duration::ZERO)
        .configure(|settings| settings.transport.availability = Some(policy(5)))
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    assert!(matches!(
        handle.client().get_me().await,
        Err(Error::RetryWaitExceeded { .. })
    ));
    let error = run(bot).await.unwrap_err();
    assert!(matches!(error, Error::RetryWaitExceeded { .. }));
    assert_eq!(
        handle.availability_status().state,
        AvailabilityState::Terminal
    );
    requests.recv().await.unwrap();
    assert!(requests.try_recv().is_err());
    http.await.unwrap();
}

#[tokio::test]
async fn server_advised_wait_cannot_exceed_availability_deadline() {
    let cursor = Cursor::new(Some(7)).await;
    let (base, mut requests, http) = support::mock(vec![(
        503,
        vec![("Retry-After", "60")],
        json!({"error":"wait"}).to_string(),
    )])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .max_retry_wait(Duration::from_secs(120))
        .configure(|settings| settings.transport.availability = Some(policy(4)))
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let error = run(bot).await.unwrap_err();
    let failure = error.availability_failure().unwrap();
    assert_eq!(failure.reason, AvailabilityEnd::BackoffExceedsDeadline);
    assert_eq!(failure.attempts, 1);
    assert_eq!(error.retry_after(), Some(Duration::from_secs(60)));
    assert_eq!(
        handle.availability_status().total_backoff_wait,
        Duration::ZERO
    );
    assert_eq!(cursor.last().await, Some(7));
    requests.recv().await.unwrap();
    assert!(requests.try_recv().is_err());
    http.await.unwrap();
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn gateway_recovery_resumes_saved_cursor_and_authentication_remains_terminal() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        accept_async,
        tungstenite::{
            protocol::{frame::coding::CloseCode, CloseFrame},
            Message,
        },
    };
    for deny in [false, true] {
        let cursor = Cursor::new(Some(7)).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let ws = tokio::spawn(async move {
            for attempt in 0..if deny { 1 } else { 2 } {
                let (stream, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut socket = accept_async(stream).await.unwrap();
                if deny || attempt == 0 {
                    socket
                        .close(Some(CloseFrame {
                            code: if deny {
                                CloseCode::Library(4004)
                            } else {
                                CloseCode::Away
                            },
                            reason: "fixture".into(),
                        }))
                        .await
                        .unwrap();
                    continue;
                }
                socket
                    .send(Message::Text(
                        json!({"op":"hello","heartbeat_interval":1000})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                let auth = socket.next().await.unwrap().unwrap();
                let auth: serde_json::Value =
                    serde_json::from_str(auth.to_text().unwrap()).unwrap();
                assert_eq!(auth["op"], "resume");
                assert_eq!(auth["seq"], 7);
                socket
                    .send(Message::Text(
                        json!({"op":"event","s":8,"d":event(8,"finish")})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message.is_close() {
                        break;
                    }
                    if message.to_text().is_ok_and(|t| t.contains("heartbeat")) {
                        let _ = socket
                            .send(Message::Text(
                                json!({"op":"heartbeat_ack"}).to_string().into(),
                            ))
                            .await;
                    }
                }
            }
        });
        let (base, mut requests, http) = support::mock(vec![response(200, profile())]).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.transport.availability = Some(policy(3)))
            .configure(|settings| {
                settings.transport.gateway_url = Some(format!("ws://{address}"));
                settings.transport.mode = rappin::TransportMode::GatewayOnly;
            })
            .command("finish", "finish", move |ctx| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    ctx.bot().unwrap().stop();
                    Ok(())
                }
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let result = run(bot).await;
        if deny {
            assert!(result.unwrap_err().is_unauthorized());
            assert_eq!(cursor.last().await, Some(7));
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        } else {
            result.unwrap();
            assert_eq!(cursor.last().await, Some(8));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(handle.availability_status().recoveries, 1);
        }
        assert_eq!(connections.load(Ordering::SeqCst), if deny { 1 } else { 2 });
        requests.recv().await.unwrap();
        assert!(requests.try_recv().is_err());
        http.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), ws)
            .await
            .unwrap()
            .unwrap();
    }
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn gateway_handshake_preserves_status_hints_and_does_not_retry_authentication() {
    use tokio::io::AsyncWriteExt;
    for status in [401, 403, 520] {
        let cursor = Cursor::new(Some(7)).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let attempts = if status == 520 { 2 } else { 1 };
        let ws = tokio::spawn(async move {
            for _ in 0..attempts {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                let reply=format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\nRetry-After: 0.001\r\n\r\n");
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let (base, mut requests, http) = support::mock(vec![response(200, profile())]).await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.transport.availability = Some(policy(2)))
            .configure(|settings| {
                settings.transport.gateway_url = Some(format!("ws://{address}"));
                settings.transport.mode = rappin::TransportMode::GatewayOnly;
            })
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let error = run(bot).await.unwrap_err();
        assert_eq!(error.status_code(), Some(status));
        assert_eq!(error.retry_after(), Some(Duration::from_millis(1)));
        if status == 520 {
            let failure = error.availability_failure().unwrap();
            assert_eq!(failure.attempts, 2);
            assert_eq!(failure.last_error.unwrap().status, Some(520));
        } else {
            assert!(error.availability_failure().is_none());
            assert_eq!(
                handle.availability_status().state,
                AvailabilityState::Terminal
            );
        }
        assert_eq!(cursor.last().await, Some(7));
        requests.recv().await.unwrap();
        assert!(requests.try_recv().is_err());
        http.await.unwrap();
        ws.await.unwrap();
    }
}

#[tokio::test]
async fn advised_wait_budget_spans_availability_attempts_without_rearming() {
    for budget in [Duration::ZERO, Duration::from_millis(3)] {
        let expected = if budget.is_zero() { 1 } else { 2 };
        let response = (
            503,
            vec![("Retry-After", "0.002")],
            json!({"error":"busy"}).to_string(),
        );
        let (base, mut requests, http) = support::mock(vec![response; expected]).await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .max_retry_wait(budget)
            .configure(|settings| settings.transport.availability = Some(policy(5)))
            .build()
            .expect("valid bot configuration");
        let handle = bot.handle();
        let error = run(bot).await.unwrap_err();
        assert!(matches!(error, Error::RetryWaitExceeded { .. }));
        assert_eq!(error.status_code(), Some(503));
        assert_eq!(error.retry_after(), Some(Duration::from_millis(2)));
        assert_eq!(handle.availability_status().total_attempts, expected as u64);
        assert_eq!(
            handle.availability_status().state,
            AvailabilityState::Terminal
        );
        let observed: Vec<_> = std::iter::from_fn(|| requests.try_recv().ok()).collect();
        assert_eq!(observed.len(), expected);
        assert!(observed.iter().all(|r| r.starts_with("GET /me ")));
        http.await.unwrap();
    }
}
