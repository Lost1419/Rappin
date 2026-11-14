#![cfg(feature = "hot-reload")]
mod support;
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    *,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use uuid::Uuid;
struct Project(PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_worker_targets_reuse_parsing_gate_replies_and_structured_causes() {
    let project =
        Project(std::env::temp_dir().join(format!("rappin-checked-worker-{}", Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    let sdk = serde_json::to_string(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(
        project.0.join("Cargo.toml"),
        format!(
            r#"
[package]
name="checked-invocation-worker"
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
use rappin::{*, hot_reload::WorkerCog};

struct Roll(BoundedU32<2,100>);
impl FromArgs for Roll {
 fn parse(a:&mut Arguments)->std::result::Result<Self,CommandError>{Ok(Self(a.required("sides")?))}
 fn describe()->Vec<ArgumentInfo>{vec![ArgumentInfo::required("sides","bounded")]}
}
#[tokio::main] async fn main()->Result<()> {
 let journal=std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<String,String>::new()));
 let mut cog=WorkerCog::new("checked-worker");
 if !std::path::Path::new("legacy").exists(){cog=cog.operation_identity();}
 cog
 .command("wid","identified record",move |ctx|{let journal=journal.clone();async move{
    let key=ctx.operation_id().expect("identified call").as_str().to_owned();
    let m=ctx.event.message().unwrap();assert_eq!(m.id.to_string(),"00000000-0000-0000-0000-000000000004");assert_eq!(m.author_id.to_string(),"00000000-0000-0000-0000-00000000002a");assert_eq!(m.channel_id.to_string(),"00000000-0000-0000-0000-000000000000");assert!(m.server_id.is_some());
    {let mut journal=journal.lock().unwrap();if let Some(previous)=journal.get(&key){if previous!=ctx.args(){return Err(CommandError::Denied("conflicting operation arguments".into()).into());}return Ok(());}journal.insert(key.clone(),ctx.args().into());}
    ctx.say(format!("{}:{}",key,ctx.args())).await?;Ok(())
 }})
 .command_typed::<Roll,_,_>("wroll","roll",|ctx,roll|async move {
    assert_eq!(roll.0.0,20);assert_eq!(ctx.event.message().unwrap().author_id.to_string(),"00000000-0000-0000-0000-00000000002a");
    ctx.reply("worker reply").await?;Ok(())
 })
 .command("wdeny","deny",|_|async {Err(CommandError::Denied("worker policy denied".into()).into())})
 .command("wpartial","partial",|ctx|async move {
    assert_eq!(ctx.operation_id().unwrap().as_str(),"partial/step");
    let first=ctx.say("confirmed").await?;
    let source=ctx.say("uncertain").await.unwrap_err();
    Err(Error::PartialWrite{completed:vec![serde_json::json!({"message":first})],failed_step:2,source:Box::new(source)})
 }).run().await
}
"#).unwrap();
    let message = json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-11-05T00:00:00Z"}});
    let (base, mut requests, http) = support::mock(vec![
        support::response(201, message.clone()),
        support::response(201, message.clone()),
        (
            502,
            vec![("Retry-After", "0.25")],
            json!({"error":"unknown"}).to_string(),
        ),
        support::response(201, message.clone()),
        support::response(201, message.clone()),
        support::response(201, message),
    ])
    .await;
    let allowed = Arc::new(AtomicBool::new(true));
    let a = allowed.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .message_gate(move |_| {
            let a = a.clone();
            async move {
                Ok(if a.load(Ordering::SeqCst) {
                    MessageDisposition::Continue
                } else {
                    MessageDisposition::Handled
                })
            }
        })
        .command("outer", "invoke", move |ctx| {
            let a = allowed.clone();
            async move {
                let bot = ctx.bot.as_ref().unwrap();
                let plan = if ctx.args == "denied" {
                    bot.prepare_invocation_with_operation_id(
                        &ctx,
                        "wroll",
                        "20",
                        OperationId::new("denied-step")?,
                    )
                    .await?
                } else {
                    bot.prepare_invocation(&ctx, "wroll", "20").await?
                };
                assert!(plan.report().deferred.iter().any(|s| s.contains("worker")));
                if ctx.args == "legacy" {
                    assert!(bot
                        .prepare_invocation_with_operation_id(
                            &ctx,
                            "wroll",
                            "20",
                            OperationId::new("legacy")?
                        )
                        .await
                        .is_err());
                    let denied = bot
                        .invoke_command_with_operation_id(
                            &ctx,
                            "wroll",
                            "20",
                            OperationId::new("legacy")?,
                        )
                        .await;
                    assert_eq!(denied.status, InvocationStatus::Failed);
                    assert!(!denied.effects_possible);
                    bot.invoke_prepared(plan).await.into_result()?;
                } else if ctx.args == "identity" {
                    let one = bot
                        .prepare_invocation_with_operation_id(
                            &ctx,
                            "wid",
                            "alice",
                            OperationId::new("step/1")?,
                        )
                        .await?;
                    let two = bot
                        .prepare_invocation_with_operation_id(
                            &ctx,
                            "wid",
                            "bob",
                            OperationId::new("step/2")?,
                        )
                        .await?;
                    for plan in [one, two] {
                        let key = plan.operation_id().unwrap().clone();
                        let out = bot.invoke_prepared(plan).await;
                        assert_eq!(out.operation_id(), Some(&key));
                        out.into_result()?;
                    }
                    bot.invoke_command_with_operation_id(
                        &ctx,
                        "wid",
                        "alice",
                        OperationId::new("step/1")?,
                    )
                    .await
                    .into_result()?;
                    let conflict = bot
                        .invoke_command_with_operation_id(
                            &ctx,
                            "wid",
                            "other",
                            OperationId::new("step/1")?,
                        )
                        .await;
                    assert_eq!(conflict.status, InvocationStatus::Denied);
                    assert_eq!(
                        conflict.into_result().unwrap_err().to_remote().to_json()["operation_id"],
                        "step/1"
                    );
                } else if ctx.args == "denied" {
                    a.store(false, Ordering::SeqCst);
                    assert_eq!(
                        bot.invoke_prepared(plan).await.status,
                        InvocationStatus::Denied
                    );
                    a.store(true, Ordering::SeqCst);
                } else if ctx.args == "partial" {
                    let error = bot
                        .invoke_command_with_operation_id(
                            &ctx,
                            "wpartial",
                            "",
                            OperationId::new("partial/step")?,
                        )
                        .await
                        .into_result()
                        .unwrap_err();
                    assert_eq!(error.status_code(), Some(502));
                    assert_eq!(error.retry_after(), Some(Duration::from_millis(250)));
                    let remote = error.to_remote().to_json();
                    assert_eq!(
                        remote["source"]["completed_steps"]
                            .as_array()
                            .unwrap()
                            .len(),
                        1
                    );
                    assert_eq!(remote["source"]["outcome"], "unknown");
                    assert_eq!(remote["operation_id"], "partial/step");
                } else {
                    bot.invoke_prepared(plan).await.into_result()?;
                    assert_eq!(
                        bot.invoke_command_with_operation_id(
                            &ctx,
                            "wroll",
                            "101",
                            OperationId::new("invalid-step")?
                        )
                        .await
                        .status,
                        InvocationStatus::Denied
                    );
                    assert_eq!(
                        bot.invoke_command(&ctx, "wdeny", "").await.status,
                        InvocationStatus::Denied
                    );
                }
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "checked-invocation-worker").offline(true),
    )
    .await
    .unwrap();
    let pid = reload.process_id().await;
    for (id, args) in [(1, "denied"), (2, "valid"), (3, "partial"), (4, "identity")] {
        let event=serde_json::from_value(json!({"id":id,"type":"message","data":{"id":Uuid::from_u128(id),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::from_u128(42),"command":"outer","args":args}})).unwrap();
        bot.process_event(event).await.unwrap();
    }
    reload.unload().await.unwrap();
    std::fs::write(project.0.join("legacy"), "legacy").unwrap();
    let legacy = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "checked-invocation-worker").offline(true),
    )
    .await
    .unwrap();
    let legacy_event=serde_json::from_value(json!({"id":5,"type":"message","data":{"id":Uuid::from_u128(5),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::from_u128(42),"command":"outer","args":"legacy"}})).unwrap();
    bot.process_event(legacy_event).await.unwrap();
    legacy.unload().await.unwrap();
    http.await.unwrap();
    let mut count = 0;
    while let Ok(request) = requests.try_recv() {
        count += 1;
        assert!(request.starts_with("POST /channels/"));
    }
    assert_eq!(count, 6);
    assert!(pid.is_some());
}
