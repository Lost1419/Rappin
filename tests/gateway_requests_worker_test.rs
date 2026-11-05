#![cfg(all(feature = "gateway", feature = "hot-reload"))]
mod support;
use futures_util::{SinkExt, StreamExt};
use rappin::{
    gateway::{Gateway, GatewayRequestConfig},
    hot_reload::{HotReload, WorkerProject},
    *,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use uuid::Uuid;
struct Project(PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_single_attempt_requests_keep_partial_and_unknown_outcomes_without_https_calls() {
    let root =
        Project(std::env::temp_dir().join(format!("rappin-gateway-worker-{}", Uuid::new_v4())));
    std::fs::create_dir_all(root.0.join("src")).unwrap();
    std::fs::write(root.0.join("Cargo.toml"),format!("[package]\nname=\"gateway-request-worker\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin={{package=\"rappin-rs\",path={},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\nserde_json=\"1\"\n",json!(env!("CARGO_MANIFEST_DIR")))).unwrap();
    std::fs::write(
        root.0.join("src/main.rs"),
        r#"
use rappin::{*,hot_reload::WorkerCog};
#[tokio::main] async fn main()->Result<()> {
 WorkerCog::new("gateway-worker").single_attempt_api()
 .command("once","request",|ctx|async move {ctx.api_once("POST","/mutate",None).await?;Ok(())})
 .command("partial","partial",|ctx|async move {
    let receipt=ctx.api_once("POST","/mutate",None).await?;
    let source=ctx.api_once("POST","/mutate",None).await.unwrap_err();
    Err(YappinError::PartialWrite{completed:vec![receipt],failed_step:2,source:Box::new(source)})
 }).run().await
}
"#,
    )
    .unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", http.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let http = tokio::spawn(async move {
        loop {
            http.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
        }
    });
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(root.0.join("Cargo.toml"), "gateway-request-worker").offline(true),
    )
    .await
    .unwrap();
    let pid = reload.process_id().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, all) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":30000})
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
        for index in 0..4 {
            let frame = ws.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(request["method"], "POST");
            if index == 3 {
                seen.send(()).unwrap();
                return;
            }
            let (status, body) = if index == 2 {
                (502, json!({"error":"uncertain","retry_after":0.25}))
            } else {
                (200, json!({"receipt":index}))
            };
            ws.send(Message::Text(
                json!({"op":"response","id":request["id"],"status":status,"body":body})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        }
    });
    bot.client()
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))
        .unwrap();
    let mut gateway = Gateway::new(bot.client().clone()).url(format!("ws://{address}"));
    gateway.poll(0).await.unwrap();
    let first: Event = serde_json::from_value(support::event(1, "once")).unwrap();
    bot.process_event(first).await.unwrap();
    let partial = bot
        .process_event(serde_json::from_value(support::event(2, "partial")).unwrap())
        .await
        .unwrap_err();
    assert_eq!(partial.status_code(), Some(502));
    assert_eq!(partial.retry_after(), Some(Duration::from_millis(250)));
    assert_eq!(partial.to_json()["completed_steps"][0]["receipt"], 1);
    let unknown = bot
        .process_event(serde_json::from_value(support::event(3, "once")).unwrap())
        .await
        .unwrap_err();
    all.await.unwrap();
    assert_eq!(unknown.to_json()["outcome"], "unknown");
    assert_eq!(unknown.to_json()["retry_safety"], "do_not_replay_workflow");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(bot.client().gateway_request_status().submitted, 4);
    assert_eq!(reload.process_id().await, pid);
    server.await.unwrap();
    http.abort();
    bot.handle().remove_cog("gateway-worker").await.unwrap();
}
