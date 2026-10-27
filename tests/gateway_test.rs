#![cfg(feature = "gateway")]
mod support;
use futures_util::{SinkExt, StreamExt};
use rappin::{gateway::Gateway, Client};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_async,
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    },
};
#[tokio::test]
async fn gateway_resumes_and_delivers_original_event() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":30000})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let frame = ws.next().await.unwrap().unwrap();
        let auth: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(auth["op"], "resume");
        assert_eq!(auth["seq"], 10);
        assert_eq!(auth["events"], json!(["message"]));
        ws.send(Message::Text(
            json!({"op":"ready","seq":99}).to_string().into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            json!({"op":"event","s":11,"d":support::event(11,"ping")})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    });
    let mut gateway = Gateway::new(Client::new("secret"))
        .url(format!("ws://{address}"))
        .events(vec!["message".into()]);
    let response = gateway.poll(10).await.unwrap();
    assert_eq!(response.last, 11);
    assert_eq!(response.events.len(), 1);
    assert_eq!(
        gateway.status().active,
        Some(rappin::EventTransport::Gateway)
    );
    assert_eq!(gateway.status().received_cursor, 11);
    task.await.unwrap();
}
#[tokio::test]
async fn beta_denial_falls_back_to_rest_and_invalid_auth_stops() {
    let (base, _requests, http) =
        support::mock(vec![support::response(200, json!({"events":[],"last":2}))]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.close(Some(CloseFrame {
            code: CloseCode::Library(4003),
            reason: "beta".into(),
        }))
        .await
        .unwrap();
    });
    let mut gateway = Gateway::new(Client::builder("secret").base_url(base).build())
        .url(format!("ws://{address}"));
    assert_eq!(gateway.poll(1).await.unwrap().last, 2);
    assert_eq!(
        gateway.status().active,
        Some(rappin::EventTransport::Polling)
    );
    assert_eq!(
        gateway.status().fallback,
        Some(rappin::FallbackReason::AccessDenied)
    );
    assert!(gateway.status().heartbeat_rtt.is_none());
    http.await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn invalid_gateway_auth_is_terminal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.close(Some(CloseFrame {
            code: CloseCode::Library(4004),
            reason: "wrong token".into(),
        }))
        .await
        .unwrap();
    });
    let mut gateway = Gateway::new(
        Client::builder("secret")
            .base_url("http://127.0.0.1:1")
            .build(),
    )
    .url(format!("ws://{address}"));
    assert!(gateway.poll(10).await.unwrap_err().is_unauthorized());
    task.await.unwrap();
}

#[tokio::test]
async fn gateway_heartbeats_before_delayed_event() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
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
        let heartbeat = ws.next().await.unwrap().unwrap();
        assert!(heartbeat.to_text().unwrap().contains("heartbeat"));
        ws.send(Message::Text(
            json!({"op":"heartbeat_ack","seq":10}).to_string().into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            json!({"op":"event","s":11,"d":support::event(11,"ping")})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    });
    let mut gateway = Gateway::new(Client::new("secret")).url(format!("ws://{address}"));
    assert_eq!(gateway.poll(10).await.unwrap().last, 11);
    assert!(gateway.status().heartbeat_rtt.is_some());
    task.await.unwrap();
}

#[tokio::test]
async fn gateway_only_returns_denial_without_attempting_rest() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .close(Some(CloseFrame {
                code: CloseCode::Library(4003),
                reason: "beta".into(),
            }))
            .await
            .unwrap();
    });
    let mut gateway = Gateway::new(
        Client::builder("secret")
            .base_url("http://127.0.0.1:1")
            .build(),
    )
    .url(format!("ws://{address}"))
    .gateway_only();
    assert_eq!(
        gateway.poll(10).await.unwrap_err().status_code(),
        Some(4003)
    );
    assert_eq!(gateway.status().mode, rappin::TransportMode::GatewayOnly);
    assert!(gateway.status().active.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn bot_gateway_keeps_heartbeating_while_a_single_handler_waits_for_a_reply() {
    use rappin::{Bot, MessageEvent};
    use std::{sync::Arc, time::Duration};
    let (base, mut requests, http) =
        support::mock(vec![support::response(200, support::profile())]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registered = Arc::new(tokio::sync::Notify::new());
    let gate = registered.clone();
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
        let auth = ws.next().await.unwrap().unwrap();
        let auth: serde_json::Value = serde_json::from_str(auth.to_text().unwrap()).unwrap();
        assert_eq!(auth["op"], "identify");
        ws.send(Message::Text(
            json!({"op":"ready","seq":10}).to_string().into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            json!({"op":"event","s":11,"d":support::event(11,"hold")})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        gate.notified().await;
        let heartbeat = ws.next().await.unwrap().unwrap();
        assert!(heartbeat.to_text().unwrap().contains("heartbeat"));
        ws.send(Message::Text(
            json!({"op":"heartbeat_ack","seq":11}).to_string().into(),
        ))
        .await
        .unwrap();
        let mut answer = support::event(19, "unused");
        answer["data"].as_object_mut().unwrap().remove("command");
        answer["data"]["content"] = json!("release");
        ws.send(Message::Text(
            json!({"op":"event","s":19,"d":answer}).to_string().into(),
        ))
        .await
        .unwrap();
        // Keep the fixture connected until the bot ends its run.
        while ws.next().await.is_some() {}
    });
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .handler_concurrency(1)
        .gateway_only(format!("ws://{address}"))
        .command(
            "hold",
            "Wait without blocking gateway heartbeats",
            move |ctx| {
                let registered = registered.clone();
                async move {
                    let answer = ctx
                        .client
                        .events()
                        .collect::<MessageEvent>()
                        .filter(|message| message.content.as_deref() == Some("release"))
                        .timeout(Duration::from_secs(2))
                        .subscribe()?;
                    registered.notify_one();
                    answer.wait().await?;
                    assert!(ctx
                        .bot()
                        .unwrap()
                        .transport_status()
                        .heartbeat_rtt
                        .is_some());
                    ctx.bot().unwrap().stop();
                    Ok(())
                }
            },
        )
        .build();
    let handle = bot.handle();
    tokio::time::timeout(Duration::from_secs(3), bot.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handle.transport_status().active, None);
    assert_eq!(handle.events().subscriber_count(), 0);
    assert!(requests.recv().await.unwrap().starts_with("GET /me "));
    assert!(requests.try_recv().is_err());
    http.await.unwrap();
    server.await.unwrap();
}
