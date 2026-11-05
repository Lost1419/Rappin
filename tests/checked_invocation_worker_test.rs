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
use rappin::{*,hot_reload::WorkerCog};
struct Roll(BoundedU32<2,100>);
impl FromArgs for Roll {
 fn parse(a:&mut Arguments)->std::result::Result<Self,CommandError>{Ok(Self(a.required("sides")?))}
 fn describe()->Vec<ArgumentInfo>{vec![ArgumentInfo::required("sides","bounded")]}
}
#[tokio::main] async fn main()->Result<()> {
 WorkerCog::new("checked-worker")
 .command_typed::<Roll,_,_>("wroll","roll",|ctx,roll|async move {
    assert_eq!(roll.0.0,20);assert_eq!(ctx.event.message().unwrap().author_id.to_string(),"00000000-0000-0000-0000-00000000002a");
    ctx.reply("worker reply").await?;Ok(())
 })
 .command("wdeny","deny",|_|async {Err(CommandError::Denied("worker policy denied".into()).into())})
 .command("wpartial","partial",|ctx|async move {
    let first=ctx.say("confirmed").await?;
    let source=ctx.say("uncertain").await.unwrap_err();
    Err(YappinError::PartialWrite{completed:vec![serde_json::json!({"message":first})],failed_step:2,source:Box::new(source)})
 }).run().await
}
"#).unwrap();
    let message = json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-11-05T00:00:00Z"}});
    let (base, mut requests, http) = support::mock(vec![
        support::response(201, message.clone()),
        support::response(201, message),
        (
            502,
            vec![("Retry-After", "0.25")],
            json!({"error":"unknown"}).to_string(),
        ),
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
                let plan = bot.prepare_invocation(&ctx, "wroll", "20").await?;
                assert!(plan.report().deferred.iter().any(|s| s.contains("worker")));
                if ctx.args == "denied" {
                    a.store(false, Ordering::SeqCst);
                    assert_eq!(
                        bot.invoke_prepared(plan).await.status,
                        InvocationStatus::Denied
                    );
                    a.store(true, Ordering::SeqCst);
                } else if ctx.args == "partial" {
                    let error = bot
                        .invoke_command(&ctx, "wpartial", "")
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
                } else {
                    bot.invoke_prepared(plan).await.into_result()?;
                    assert_eq!(
                        bot.invoke_command(&ctx, "wroll", "101").await.status,
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
        .build();
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
    for (id, args) in [(1, "denied"), (2, "valid"), (3, "partial")] {
        let event=serde_json::from_value(json!({"id":id,"type":"message","data":{"id":Uuid::from_u128(id),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::from_u128(42),"command":"outer","args":args}})).unwrap();
        bot.process_event(event).await.unwrap();
    }
    http.await.unwrap();
    let mut count = 0;
    while let Ok(request) = requests.try_recv() {
        count += 1;
        assert!(request.starts_with("POST /channels/"));
    }
    assert_eq!(count, 3);
    assert_eq!(reload.process_id().await, pid);
    bot.handle().remove_cog("checked-worker").await.unwrap();
}
