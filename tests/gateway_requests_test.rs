#![cfg(feature = "gateway")]
mod support;
use futures_util::{SinkExt, StreamExt};
use rappin::{
    gateway::{Gateway, GatewayRequestConfig, RequestTransport},
    Api, Client,
};
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
async fn text(ws: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        let frame = ws.next().await.unwrap().unwrap();
        if let Message::Text(t) = frame {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "heartbeat" {
                ws.send(Message::Text(
                    json!({"op":"heartbeat_ack"}).to_string().into(),
                ))
                .await
                .unwrap();
                continue;
            }
            return v;
        }
    }
}
async fn ready(ws: &mut WebSocketStream<TcpStream>, capability: bool) {
    ws.send(Message::Text(
        json!({"op":"hello","heartbeat_interval":100})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(text(ws).await["op"], "identify");
    ws.send(Message::Text(
        json!({"op":"ready","seq":0,"requests":capability})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
}
async fn reply(ws: &mut WebSocketStream<TcpStream>, id: Value, status: u16, body: Value) {
    ws.send(Message::Text(
        json!({"op":"response","id":id,"status":status,"body":body})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
}
async fn no_http() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let _ = listener.accept().await.unwrap();
            c.fetch_add(1, Ordering::SeqCst);
        }
    });
    (format!("http://{addr}"), count, task)
}
#[tokio::test]
async fn correlated_out_of_order_requests_keep_events_and_heartbeats_responsive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, done) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        let a = text(&mut ws).await;
        let b = text(&mut ws).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        // Read and acknowledge the heartbeat while requests remain pending.
        let frame = ws.next().await.unwrap().unwrap();
        assert!(frame.to_text().unwrap().contains("heartbeat"));
        ws.send(Message::Text(
            json!({"op":"heartbeat_ack"}).to_string().into(),
        ))
        .await
        .unwrap();
        reply(&mut ws, json!("unknown"), 200, json!({})).await;
        reply(&mut ws, b["id"].clone(), 200, json!({"path":b["path"]})).await;
        reply(&mut ws, a["id"].clone(), 200, json!({"path":a["path"]})).await;
        reply(&mut ws, a["id"].clone(), 200, json!({})).await;
        ws.send(Message::Text(
            json!({"op":"event","d":support::event(1,"ping")})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let _ = done.await;
    });
    let (base, calls, http) = no_http().await;
    let client = Client::builder("secret").base_url(base).build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    assert!(client
        .send_message(uuid::Uuid::nil(), &rappin::CreateMessage::new(""))
        .await
        .is_err());
    assert_eq!(client.gateway_request_status().submitted, 0);
    let (a, b, event) = tokio::join!(
        client.request(Method::GET, "/one", None),
        client.request(Method::GET, "/two", None),
        gateway.poll(0)
    );
    assert_eq!(a.unwrap()["path"], "/one");
    assert_eq!(b.unwrap()["path"], "/two");
    assert_eq!(event.unwrap().last, 1);
    let status = client.gateway_request_status();
    assert_eq!(status.selected, RequestTransport::Gateway);
    assert_eq!(status.submitted, 2);
    assert_eq!(status.completed, 2);
    assert_eq!(status.unknown_responses, 2);
    assert!(gateway.status().heartbeat_rtt.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    stop.send(()).unwrap();
    server.await.unwrap();
    http.abort();
}

#[tokio::test]
async fn uncertain_disconnect_timeout_and_cancellation_never_fallback_or_replay() {
    for mode in ["disconnect", "timeout", "cancel"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, received) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            ready(&mut ws, true).await;
            let request = text(&mut ws).await;
            assert_eq!(request["method"], "POST");
            seen.send(()).unwrap();
            if mode != "disconnect" {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        let (base, calls, http) = no_http().await;
        let client = Client::builder("secret")
            .base_url(base)
            .max_retries(5)
            .build();
        let mut config = GatewayRequestConfig::default();
        config.timeout = Duration::from_millis(100);
        client.configure_gateway_requests(Some(config)).unwrap();
        let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
        gateway.poll(0).await.unwrap();
        let c = client.clone();
        let request = tokio::spawn(async move {
            c.request_once(Method::POST, "/mutate", Some(json!({"content":"once"})))
                .await
        });
        received.await.unwrap();
        if mode == "cancel" {
            request.abort();
            let _ = request.await;
            tokio::task::yield_now().await;
            assert!(client.gateway_request_status().uncertain_completions > 0);
        } else {
            let error = request.await.unwrap().unwrap_err();
            assert_eq!(error.to_json()["outcome"], "unknown");
            assert_eq!(error.to_json()["submitted"], true);
            assert_eq!(error.to_json()["retry_safety"], "do_not_replay_workflow");
        }
        assert_eq!(client.gateway_request_status().submitted, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        server.abort();
        http.abort();
        drop(gateway);
    }
}

#[tokio::test]
async fn saturation_is_bounded_and_does_not_submit_a_ninth_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, all) = oneshot::channel();
    let (stop, done) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        for _ in 0..8 {
            assert_eq!(text(&mut ws).await["op"], "request");
        }
        seen.send(()).unwrap();
        let _ = done.await;
    });
    let (base, calls, http) = no_http().await;
    let client = Client::builder("secret").base_url(base).build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    let mut tasks = vec![];
    for _ in 0..8 {
        let c = client.clone();
        tasks.push(tokio::spawn(async move {
            c.request(Method::POST, "/mutate", None).await
        }));
    }
    all.await.unwrap();
    let error = client
        .request(Method::POST, "/mutate", None)
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["submitted"], false);
    assert_eq!(error.to_json()["outcome"], "not_applied");
    assert_eq!(client.gateway_request_status().in_flight, 8);
    assert_eq!(client.gateway_request_status().submitted, 8);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    stop.send(()).unwrap();
    server.await.unwrap();
    for t in tasks {
        assert!(t.await.unwrap().is_err());
    }
    http.abort();
}

#[tokio::test]
async fn server_advised_429_uses_shared_budget_but_request_once_stays_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, done) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        let first = text(&mut ws).await;
        reply(
            &mut ws,
            first["id"].clone(),
            429,
            json!({"error":"wait","retry_after":0.01}),
        )
        .await;
        let second = text(&mut ws).await;
        reply(&mut ws, second["id"].clone(), 200, json!({"ok":true})).await;
        let once = text(&mut ws).await;
        reply(
            &mut ws,
            once["id"].clone(),
            429,
            json!({"error":"wait once","retry_after":0.02}),
        )
        .await;
        let _ = done.await;
    });
    let (base, calls, http) = no_http().await;
    let client = Client::builder("secret").base_url(base).build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    assert_eq!(
        client.request(Method::POST, "/mutate", None).await.unwrap()["ok"],
        true
    );
    let error = client
        .request_once(Method::POST, "/mutate", None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.retry_after(), Some(Duration::from_millis(20)));
    assert_eq!(client.gateway_request_status().submitted, 3);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    stop.send(()).unwrap();
    server.await.unwrap();
    http.abort();
}

#[tokio::test]
async fn capability_changes_and_ineligible_paths_select_https_before_submission() {
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({"via":"https"})),
        support::response(200, json!({"events":[],"last":0})),
        support::response(200, json!({"via":"disabled"})),
    ])
    .await;
    let client = Client::builder("secret").base_url(base).build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (next, continue_server) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, false).await;
        let _ = continue_server.await;
        drop(ws);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        let request = text(&mut ws).await;
        reply(
            &mut ws,
            request["id"].clone(),
            403,
            json!({"error":"permission denied"}),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let url = format!("ws://{addr}");
    let mut gateway = Gateway::new(client.clone()).url(&url);
    gateway.poll(0).await.unwrap();
    assert_eq!(
        client.request(Method::GET, "/read", None).await.unwrap()["via"],
        "https"
    );
    assert_eq!(
        client.gateway_request_status().selected,
        RequestTransport::Https
    );
    next.send(()).unwrap();
    drop(gateway);
    let mut gateway = Gateway::new(client.clone()).url(&url);
    gateway.poll(0).await.unwrap();
    let err = client
        .request(Method::DELETE, "/resource", None)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), Some(403));
    assert_eq!(err.to_json()["outcome"], "rejected");
    client.request(Method::GET, "/events", None).await.unwrap();
    assert_eq!(
        client.gateway_request_status().selected,
        RequestTransport::Https
    );
    client.configure_gateway_requests(None).unwrap();
    assert_eq!(
        client.request(Method::GET, "/read", None).await.unwrap()["via"],
        "disabled"
    );
    http.await.unwrap();
    server.await.unwrap();
    let mut count = 0;
    while requests.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 3);
}

#[tokio::test]
async fn socket_rate_window_never_submits_more_than_one_hundred_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, done) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        for _ in 0..100 {
            let request = text(&mut ws).await;
            reply(&mut ws, request["id"].clone(), 200, json!({})).await;
        }
        let _ = done.await;
    });
    let (base, calls, http) = no_http().await;
    let client = Client::builder("secret")
        .base_url(base)
        .max_retries(0)
        .build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    for _ in 0..100 {
        client.request(Method::POST, "/mutate", None).await.unwrap();
    }
    let error = client
        .request(Method::POST, "/mutate", None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(429));
    assert!(error.retry_after().is_some());
    assert_eq!(client.gateway_request_status().submitted, 100);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    stop.send(()).unwrap();
    server.await.unwrap();
    http.abort();
}

#[tokio::test]
async fn request_pump_disconnect_keeps_availability_reads_retryable() {
    use rappin::{AvailabilityPolicy, Bot, EventCursor};
    let (base, _requests, http) =
        support::mock(vec![support::response(200, support::profile())]).await;
    let path = std::env::temp_dir().join(format!(
        "rappin-pump-availability-{}.json",
        uuid::Uuid::new_v4()
    ));
    EventCursor::new(&path).save(0).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, done) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        drop(ws);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        ws.send(Message::Text(
            json!({"op":"event","d":support::event(1,"stop")})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let _ = done.await;
    });
    let policy = AvailabilityPolicy::new(3, Duration::from_secs(3))
        .unwrap()
        .backoff(Duration::from_millis(1), Duration::from_millis(1))
        .unwrap()
        .jitter_percent(0)
        .unwrap();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&path)
        .gateway_only(format!("ws://{address}"))
        .availability_policy(policy)
        .command("stop", "stop", |ctx| async move {
            ctx.bot().unwrap().stop();
            Ok(())
        })
        .build();
    bot.client()
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), bot.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(1));
    stop.send(()).unwrap();
    server.await.unwrap();
    http.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn oversized_json_selects_https_without_submitting_a_socket_frame() {
    let (base, mut requests, http) =
        support::mock(vec![support::response(200, json!({"via":"https"}))]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, done) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready(&mut ws, true).await;
        let _ = done.await;
    });
    let client = Client::builder("secret").base_url(base).build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    let body = json!({"data":"x".repeat(1024*1024)});
    let response = client
        .request(Method::POST, "/large", Some(body))
        .await
        .unwrap();
    assert_eq!(response["via"], "https");
    assert_eq!(
        client.gateway_request_status().selected,
        RequestTransport::Https
    );
    assert_eq!(client.gateway_request_status().submitted, 0);
    assert!(requests.recv().await.unwrap().starts_with("POST /large "));
    http.await.unwrap();
    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn unknown_gateway_mutation_is_not_replayed_by_the_outer_client_retry_loop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let posts = count.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ready(&mut ws, true).await;
        let request = text(&mut ws).await;
        assert_eq!(request["method"], "POST");
        posts.fetch_add(1, Ordering::SeqCst);
    });
    let (base, https, http) = no_http().await;
    let client = Client::builder("fixture")
        .base_url(base)
        .max_retries(5)
        .build();
    client
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(client.clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        client.request(Method::POST, "/mutate", Some(json!({"once":true}))),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.to_json()["outcome"], "unknown");
    assert_eq!(error.to_json()["submitted"], true);
    assert!(error.to_json()["reason"].is_string());
    server.await.unwrap();
    http.abort();
    assert_eq!(https.load(Ordering::SeqCst), 0);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
