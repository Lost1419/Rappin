#![cfg(feature = "hot-reload")]
mod support;
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    EventCursor, *,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

struct Project(PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn event(id: u64, content: &str) -> Value {
    json!({"id":id,"type":"message","data":{"id":Uuid::from_u128(id as u128),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::nil(),"content":content}})
}
fn message() -> Value {
    json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-11-03T00:00:00Z"}})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parent_gate_and_worker_message_outcomes_preserve_failures_and_active_routes() {
    let project =
        Project(std::env::temp_dir().join(format!("rappin-message-worker-{}", Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    let sdk = serde_json::to_string(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(
        project.0.join("Cargo.toml"),
        format!(
            r#"
[package]
name="message-worker"
version="0.0.0"
edition="2021"
[workspace]
[dependencies]
rappin={{package="rappin-rs",path={sdk},features=["hot-reload"]}}
tokio={{version="1",features=["macros","rt-multi-thread"]}}
serde_json="1"
"#
        ),
    )
    .unwrap();
    std::fs::write(project.0.join("src/main.rs"),r#"
use rappin::{hot_reload::WorkerCog, MessageDisposition as D, CommandError, Result, Error};
#[tokio::main] async fn main()->Result<()> {
    WorkerCog::new("messages")
    .command("wping","test",|ctx|async move {ctx.say("worker command").await?;Ok(())})
    .on_message(|ctx|async move {
        let message=ctx.event.message().unwrap();
        match message.content.as_deref().unwrap_or("") {
            "reject"|"clarify"=>Err(CommandError::Denied("worker rejected input".into()).into()),
            "partial"=>{
                let receipt=ctx.say("confirmed first").await?;
                match ctx.say("uncertain second").await {
                    Ok(_)=>panic!("fixture must fail"),
                    Err(source)=>Err(Error::PartialWrite {completed:vec![serde_json::json!({"message":receipt})],failed_step:2,source:Box::new(source)})
                }
            },
            "stop"=>Ok(D::Stop),
            _=>{ctx.say("worker listener").await?;Ok(D::Handled)}
        }
    }).run().await
}


"#).unwrap();
    let cursor = project.0.join("cursor.json");
    EventCursor::new(&cursor).save(1).await.unwrap();
    let mut posts = vec![support::response(201, message()); 5];
    posts.push((
        502,
        vec![("Retry-After", "0.25")],
        json!({"error":"unknown second write"}).to_string(),
    ));
    posts.extend(vec![support::response(201, message()); 3]);
    posts.push((
        502,
        vec![("Retry-After", "0.25")],
        json!({"error":"unknown second write"}).to_string(),
    ));
    let (base,mut requests,http)=support::routed_mock(vec![
        (format!("POST /channels/{}/messages", Uuid::nil()), posts),
        ("GET /me".into(), vec![support::response(200, support::profile())]),
        ("GET /events".into(), vec![support::response(200, json!({"events":[event(10,"denied"),event(10,"denied"),event(11,"partial"),event(12,"!later")],"last":12}))]),
    ]).await;
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor)))
        .configure(|settings| settings.execution.concurrency = 1)
        .message_gate(|ctx| async move {
            Ok(if ctx.content().contains("denied") {
                MessageDisposition::Handled
            } else {
                MessageDisposition::Continue
            })
        })
        .on_message_failure(move |ctx, failure| {
            let seen = s.clone();
            async move {
                assert_eq!(failure.stage, MessageStage::Listener);
                assert_eq!(failure.handler, "messages");
                seen.lock().unwrap().push(failure);
                if ctx.content() == "clarify" {
                    ctx.reply("clarification completed").await?;
                    Ok(FailureDisposition::Handled)
                } else {
                    Ok(FailureDisposition::Default)
                }
            }
        })
        .command("later", "must not execute", |_| async {
            panic!("cursor barrier crossed")
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "message-worker").offline(true),
    )
    .await
    .unwrap();
    let pid = reload.process_id().await;
    for (id, text) in [
        (2, "!wping denied"),
        (3, "!wping"),
        (4, "reject"),
        (5, "clarify"),
    ] {
        bot.process_event(serde_json::from_value(event(id, text)).unwrap())
            .await
            .unwrap();
    }
    let partial = bot
        .process_event(serde_json::from_value(event(6, "partial")).unwrap())
        .await
        .unwrap_err();
    let data = partial.to_json();
    assert_eq!(data["kind"], "partial_write");
    assert_eq!(data["completed_steps"].as_array().unwrap().len(), 1);
    assert_eq!(partial.status_code(), Some(502));
    assert_eq!(partial.retry_after(), Some(Duration::from_millis(250)));
    assert_eq!(data["outcome"], "unknown");
    bot.process_event(serde_json::from_value(event(7, "!wping")).unwrap())
        .await
        .unwrap();
    let stopped = bot
        .process_event(serde_json::from_value(event(8, "stop")).unwrap())
        .await
        .unwrap_err();
    assert_eq!(stopped.to_json()["kind"], "message_stopped");
    assert_eq!(reload.process_id().await, pid);
    assert_eq!(reload.generation(), 1);
    let failure = tokio::time::timeout(Duration::from_secs(10), bot.run())
        .await
        .unwrap()
        .unwrap_err();
    let report = match failure {
        Error::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(EventCursor::new(&cursor).load().await.unwrap(), Some(10));
    assert_eq!(report.failed_events[0].event_id, 11);
    assert_eq!(report.failed_events[0].error.kind, "partial_write");
    assert_eq!(report.unprocessed_events, [12]);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[2].event_id, 6);
        assert_eq!(seen[2].error.to_json(), data);
    }
    let mut posts = vec![];
    while let Ok(request) = requests.try_recv() {
        if request.starts_with("POST ") {
            posts.push(request);
        }
    }
    assert_eq!(posts.len(), 10);
    assert_eq!(
        posts
            .iter()
            .filter(|p| p.contains("worker command"))
            .count(),
        2
    );
    reload.unload().await.unwrap();
    http.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_worker_message_outcome_is_uncertain_and_cannot_be_reused() {
    let project =
        Project(std::env::temp_dir().join(format!("rappin-message-protocol-{}", Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    std::fs::write(
        project.0.join("Cargo.toml"),
        "[package]\nname=\"message-protocol\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(project.0.join("src/main.rs"),r#"
use std::io::{BufRead, Write};
fn main(){
    println!("{}",r#READY#); std::io::stdout().flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        let line=line.unwrap(); if line.contains("shutdown"){break;}
        println!("{{\"op\":\"done\",\"error\":null}}");std::io::stdout().flush().unwrap();
    }
}
"#.replace("r#READY#",&format!("{:?}",json!({"op":"ready","protocol":12,"manifest":{"name":"outcome","commands":[],"buttons":[],"listeners":["server_message"]}}).to_string()))).unwrap();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "message-protocol").offline(true),
    )
    .await
    .unwrap();
    let error = bot
        .process_event(serde_json::from_value(event(2, "hello")).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "handler_interrupted");
    assert_eq!(error.to_json()["outcome"], "unknown");
    let next = bot
        .process_event(serde_json::from_value(event(3, "next")).unwrap())
        .await
        .unwrap_err();
    assert!(next.to_string().contains("no longer usable"));
    assert_eq!(reload.generation(), 1);
    reload.unload().await.unwrap();
}
