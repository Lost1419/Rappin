use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_async,
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    },
};

#[tokio::test]
async fn server_message_gate_completes_rejections_and_retains_uncertain_gateway_barriers() {
    for fail in [false, true] {
        let cursor = Cursor::new().await;
        let final_event = event(3, if fail { "bad" } else { "next" });
        let mut events = vec![event(2, "reject"), event(2, "reject"), final_event];
        if fail {
            events.push(event(4, "next"));
        }
        let (url, ws) = gateway(events, false).await;
        let (base, _requests, http) = routed_mock(vec![
            ("GET /me".into(), vec![response(200, profile())]),
            (
                format!("POST /channels/{}/messages", uuid::Uuid::nil()),
                vec![response(201, message())],
            ),
        ])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .cursor_file(&cursor.0)
            .handler_concurrency(1)
            .gateway_only(url)
            .message_gate(|ctx| async move {
                match ctx.event.command.as_deref() {
                    Some("reject") => Err(rejected()),
                    Some("bad") => Err(uncertain()),
                    _ => Ok(MessageDisposition::Continue),
                }
            })
            .command("reject", "must not execute", |_| async {
                panic!("denied command executed")
            })
            .command("next", "continue", move |ctx| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    ctx.bot().unwrap().stop();
                    Ok(())
                }
            })
            .build();
        let result = run(bot).await;
        assert_eq!(cursor.last().await, if fail { 2 } else { 3 });
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(!fail));
        if fail {
            let report = report(result.unwrap_err());
            assert_eq!(report.failed_events[0].event_id, 3);
            assert_eq!(
                report.failed_events[0].error.to_json(),
                uncertain().to_json()
            );
        } else {
            result.unwrap();
        }
        http.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), ws)
            .await
            .unwrap()
            .unwrap();
    }
}

async fn gateway(events: Vec<Value>, deny: bool) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        if deny {
            let _ = ws
                .close(Some(CloseFrame {
                    code: CloseCode::Library(4004),
                    reason: "invalid authentication".into(),
                }))
                .await;
            return;
        }
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":100})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let identify = ws.next().await.unwrap().unwrap();
        let identify: Value = serde_json::from_str(identify.to_text().unwrap()).unwrap();
        assert!(identify["op"] == "identify" || identify["op"] == "resume");
        ws.send(Message::Text(
            json!({"op":"ready","seq":1}).to_string().into(),
        ))
        .await
        .unwrap();
        for event in events {
            if ws
                .send(Message::Text(
                    json!({"op":"event","s":event["id"],"d":event})
                        .to_string()
                        .into(),
                ))
                .await
                .is_err()
            {
                return;
            }
        }
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
            if let Ok(text) = message.to_text() {
                if text.contains("heartbeat") {
                    let _ = ws
                        .send(Message::Text(
                            json!({"op":"heartbeat_ack"}).to_string().into(),
                        ))
                        .await;
                }
            }
        }
    });
    (format!("ws://{address}"), task)
}

#[tokio::test]
async fn handled_command_and_button_failures_continue_and_checkpoint_over_gateway() {
    for is_button in [false, true] {
        let cursor = Cursor::new().await;
        let bad = if is_button {
            button(2, "bad:")
        } else {
            event(2, "bad")
        };
        let (url, ws) = gateway(vec![bad, event(3, "next")], false).await;
        let mut routes = vec![("GET /me".into(), vec![response(200, profile())])];
        if !is_button {
            routes.push((
                format!("POST /channels/{}/messages", uuid::Uuid::nil()),
                vec![response(200, message())],
            ));
        }
        let (base, _requests, http) = routed_mock(routes).await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let a = attempts.clone();
        let continued = Arc::new(AtomicUsize::new(0));
        let c = continued.clone();
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .cursor_file(&cursor.0)
            .handler_concurrency(1)
            .gateway_only(url)
            .command("bad", "Reject", move |_| {
                let a = a.clone();
                async move {
                    a.fetch_add(1, Ordering::SeqCst);
                    Err(rejected())
                }
            })
            .button("bad:", |_| async { Err(uncertain()) })
            .on_button_failure(|_, failure| async move {
                assert_eq!(failure.error.to_json(), uncertain().to_json());
                Ok(FailureDisposition::Handled)
            })
            .command("next", "Continue", move |ctx| {
                let c = c.clone();
                async move {
                    assert_eq!(
                        ctx.bot().unwrap().transport_status().active,
                        Some(EventTransport::Gateway)
                    );
                    c.fetch_add(1, Ordering::SeqCst);
                    ctx.bot().unwrap().stop();
                    Ok(())
                }
            })
            .build();
        run(bot).await.unwrap();
        assert_eq!(cursor.last().await, 3);
        assert_eq!(continued.load(Ordering::SeqCst), 1);
        assert_eq!(attempts.load(Ordering::SeqCst), usize::from(!is_button));
        http.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), ws)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn unprocessed_gateway_failure_retains_prefix_and_never_runs_later_command() {
    let cursor = Cursor::new().await;
    let (url, ws) = gateway(
        vec![event(2, "reject"), event(3, "bad"), event(4, "later")],
        false,
    )
    .await;
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            format!("POST /channels/{}/messages", uuid::Uuid::nil()),
            vec![response(200, message())],
        ),
    ])
    .await;
    let later = Arc::new(AtomicUsize::new(0));
    let l = later.clone();
    let failure = report(
        run(Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .cursor_file(&cursor.0)
            .handler_concurrency(1)
            .gateway_only(url)
            .command("reject", "Reject", |_| async { Err(rejected()) })
            .command("bad", "Uncertain", |_| async { Err(uncertain()) })
            .command("later", "Never start", move |_| {
                let l = l.clone();
                async move {
                    l.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build())
        .await
        .unwrap_err(),
    );
    assert_eq!(cursor.last().await, 2);
    assert_eq!(failure.failed_events[0].event_id, 3);
    assert_eq!(
        failure.failed_events[0].error.to_json(),
        uncertain().to_json()
    );
    assert_eq!(later.load(Ordering::SeqCst), 0);
    http.await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), ws)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn gateway_authentication_failure_is_not_a_recoverable_command_failure() {
    let cursor = Cursor::new().await;
    let (url, ws) = gateway(vec![], true).await;
    let (base, _requests, http) =
        routed_mock(vec![("GET /me".into(), vec![response(200, profile())])]).await;
    let callbacks = Arc::new(AtomicUsize::new(0));
    let c = callbacks.clone();
    let error = run(Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&cursor.0)
        .gateway_only(url)
        .on_command_failure(move |_, _| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(FailureDisposition::Handled)
            }
        })
        .build())
    .await
    .unwrap_err();
    assert!(error.is_unauthorized());
    assert!(report(error).intake_error.is_some());
    assert_eq!(callbacks.load(Ordering::SeqCst), 0);
    assert_eq!(cursor.last().await, 1);
    http.await.unwrap();
    ws.await.unwrap();
}
