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
    task.await.unwrap();
}
