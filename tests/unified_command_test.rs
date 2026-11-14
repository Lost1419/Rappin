mod support;
use rappin::*;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use uuid::Uuid;

fn message(command: &str, args: &str, id: u64) -> Event {
    let mut value = support::event(id, command);
    value["data"]["args"] = json!(args);
    value["data"]["server_id"] = json!(Uuid::from_u128(3));
    value["data"]["author_id"] = json!(Uuid::from_u128(4));
    serde_json::from_value(value).unwrap()
}
fn interaction(command: &str, args: &str, id: u64) -> Event {
    serde_json::from_value(json!({"id":id,"type":"command","data":{
        "id":Uuid::from_u128(id as u128),"token":"interaction_fixture","command":command,"args":args,
        "server_id":Uuid::from_u128(3),"channel_id":Uuid::nil(),"user_id":Uuid::from_u128(4),
        "expires_at":chrono::Utc::now()+chrono::Duration::minutes(15)
    }})).unwrap()
}
fn receipt() -> serde_json::Value {
    json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":chrono::Utc::now()}})
}
async fn seed(bot: &Bot) {
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
}
struct Trap;
#[async_trait::async_trait]
impl EventHandler for Trap {
    async fn on_interaction(&self, _: &InteractionContext) -> Result<()> {
        panic!("registered command reached interaction listener")
    }
}
struct Count(BoundedU32<1, 10>);
impl FromArgs for Count {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self(args.required("count")?))
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![ArgumentInfo::required("count", "1 through 10")]
    }
    fn schema() -> Option<Vec<ArgumentSchema>> {
        Some(vec![ArgumentSchema {
            name: "count".into(),
            description: "1 through 10".into(),
            kind: ArgumentType::Integer {
                min: Some(1),
                max: Some(10),
            },
            required: true,
            rest: false,
        }])
    }
}
fn spec() -> CommandSpec<Count> {
    CommandSpec::new("tools count", "Count")
        .alias("short")
        .check(Check::server_only())
        .check(check_fn(|ctx| async move {
            if !ctx.state::<AtomicBool>()?.load(Ordering::SeqCst) {
                return Err(CommandError::Denied("access revoked".into()).into());
            }
            Ok(())
        }))
}

#[tokio::test]
async fn defaults_enable_recovery_and_interaction_publication_without_switches() {
    let config = bot::TransportConfig::default();
    assert!(config.availability.is_some());
    assert!(config.read_availability.is_some());
    assert_eq!(config.max_read_retries, 3);
    assert_eq!(config.max_write_retries, 3);
    let (base, mut requests, http) = support::mock(vec![
        support::response(520, json!({"error":"edge"})),
        support::response(200, support::profile()),
        support::response(
            200,
            json!({"commands":[{"name":"ping","description":"Ping"}]}),
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .command("ping", "Ping", |ctx| async move { ctx.reply("pong").await })
        .build()
        .unwrap();
    bot.client().get_me().await.unwrap();
    assert_eq!(bot.client().read_availability_status().recoveries, 1);
    bot.handle().sync_commands().await.unwrap();
    assert!(requests.recv().await.unwrap().starts_with("GET /me "));
    assert!(requests.recv().await.unwrap().starts_with("GET /me "));
    let publication = requests.recv().await.unwrap();
    assert!(publication.contains("\"interactions\":true"));
    http.await.unwrap();
}

#[tokio::test]
async fn typed_commands_share_aliases_groups_checks_state_and_reply_ownership() {
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, receipt()),
        support::response(200, json!({})),
        support::response(200, json!({})),
        support::response(200, json!({})),
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let allowed = Arc::new(AtomicBool::new(true));
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .state(calls.clone())
        .state(allowed.clone())
        .on_command_interaction(|_| async { panic!("registered command reached generic fallback") })
        .event_handler(Trap)
        .command_spec(spec(), |ctx, count| async move {
            assert_eq!(ctx.command_name(), Some("tools count"));
            assert_eq!(ctx.args(), "7");
            assert_eq!(count.0 .0, 7);
            assert_eq!(ctx.author_id(), Some(Uuid::from_u128(4)));
            assert_eq!(ctx.server_id(), Some(Uuid::from_u128(3)));
            assert_eq!(ctx.channel_id(), Some(Uuid::nil()));
            if let Some(original) = ctx.interaction() {
                assert_eq!(original.command.as_deref(), Some("tools"));
                assert_eq!(original.args.as_deref(), Some("count 7"));
            }
            ctx.state::<AtomicUsize>()?.fetch_add(1, Ordering::SeqCst);
            ctx.reply("counted").await
        })
        .build()
        .unwrap();
    seed(&bot).await;
    assert!(bot.handle().help("short").await.contains("<count>"));
    bot.process_event(message("short", "7", 1)).await.unwrap();
    bot.process_event(interaction("tools", "count 7", 2))
        .await
        .unwrap();
    bot.process_event(interaction("short", "101", 3))
        .await
        .unwrap();
    allowed.store(false, Ordering::SeqCst);
    bot.process_event(interaction("short", "7", 4))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let first = requests.recv().await.unwrap();
    assert!(first.contains("\"reply_to\":"));
    let second = requests.recv().await.unwrap();
    assert!(second.contains("\"type\":\"reply\""));
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"type\":\"ephemeral\""));
    assert!(requests.recv().await.unwrap().contains("access revoked"));
    http.await.unwrap();
}

#[tokio::test]
async fn one_cooldown_and_failure_callback_cover_both_delivery_forms() {
    let (base, _requests, http) = support::mock(vec![
        support::response(200, receipt()),
        support::response(200, json!({})),
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let failures = Arc::new(AtomicUsize::new(0));
    let f = failures.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .command_with_cooldown(
            "limited",
            "Limited",
            Cooldown::user(Duration::from_secs(60)),
            move |ctx| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    ctx.reply("allowed").await
                }
            },
        )
        .on_command_failure(move |ctx, failure| {
            let f = f.clone();
            async move {
                assert!(ctx.interaction().is_some());
                assert_eq!(failure.kind, HandlerKind::Command);
                assert_eq!(failure.class, FailureClass::Rejected);
                f.fetch_add(1, Ordering::SeqCst);
                Ok(FailureDisposition::Default)
            }
        })
        .build()
        .unwrap();
    seed(&bot).await;
    bot.process_event(message("limited", "", 1)).await.unwrap();
    bot.process_event(interaction("limited", "", 2))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(failures.load(Ordering::SeqCst), 1);
    http.await.unwrap();
}

#[tokio::test]
async fn a_cog_uses_one_fallible_check_for_both_commands() {
    struct Guard(Arc<AtomicBool>, Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl Cog for Guard {
        fn name(&self) -> &str {
            "guard"
        }
        async fn check(&self, ctx: &CommandContext) -> Result<bool> {
            assert_eq!(ctx.author_id(), Some(Uuid::from_u128(4)));
            Ok(self.0.load(Ordering::SeqCst))
        }
        fn commands(&self) -> Vec<CogCommand> {
            let calls = self.1.clone();
            vec![CogCommand::new("guarded", "Guarded", move |_| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })]
        }
    }
    let (base, _requests, http) = support::mock(vec![support::response(200, json!({}))]).await;
    let allowed = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(AtomicUsize::new(0));
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build()
        .unwrap();
    seed(&bot).await;
    bot.handle()
        .add_cog(Guard(allowed.clone(), calls.clone()))
        .await
        .unwrap();
    bot.process_event(message("guarded", "", 1)).await.unwrap();
    bot.process_event(interaction("guarded", "", 2))
        .await
        .unwrap();
    allowed.store(false, Ordering::SeqCst);
    bot.process_event(message("guarded", "", 3)).await.unwrap();
    bot.process_event(interaction("guarded", "", 4))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    http.await.unwrap();
}

#[cfg(feature = "hot-reload")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_specs_share_checks_scope_recovery_partial_errors_and_reload() {
    struct Project(std::path::PathBuf);
    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let project =
        Project(std::env::temp_dir().join(format!("rappin-unified-worker-{}", Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    let manifest=format!("[package]\nname=\"unified-command-worker\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin={{package=\"rappin-rs\",path={},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\nserde_json=\"1\"\n",json!(env!("CARGO_MANIFEST_DIR")));
    std::fs::write(project.0.join("Cargo.toml"), manifest).unwrap();
    #[cfg(unix)]
    {
        std::fs::create_dir_all(project.0.join("target")).unwrap();
        let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rappin-hot");
        std::fs::create_dir_all(&cache).unwrap();
        std::os::unix::fs::symlink(cache, project.0.join("target/rappin-hot")).unwrap();
    }
    std::fs::write(project.0.join("src/main.rs"),r#"
use rappin::*;
use std::sync::{Arc,atomic::{AtomicBool,AtomicUsize,Ordering}};
struct State{allowed:AtomicBool,calls:AtomicUsize}
struct Count(BoundedU32<1,10>);
impl FromArgs for Count{fn parse(a:&mut Arguments)->std::result::Result<Self,CommandError>{Ok(Self(a.required("count")?))}}
#[tokio::main]async fn main()->Result<()> {
 WorkerCog::new("unified")
 .state(Arc::new(State{allowed:AtomicBool::new(true),calls:AtomicUsize::new(0)}))
 .command_spec(CommandSpec::<Count>::new("tools count","Count").alias("short")
    .check(Check::server_only()).check(check_fn(|ctx|async move{
        if !ctx.state::<State>()?.allowed.load(Ordering::SeqCst){return Err(CommandError::Denied("revoked".into()).into())}Ok(())
    })),|ctx,count|async move{
    assert_eq!(count.0.0,7);assert_eq!(ctx.command_name(),Some("tools count"));assert_eq!(ctx.args(),"7");
    assert_eq!(ctx.author_id().unwrap().as_u128(),4);assert_eq!(ctx.server_id().unwrap().as_u128(),3);assert_eq!(ctx.channel_id().unwrap().as_u128(),0);
    if let Some(event)=ctx.worker().unwrap().ui_interaction(){assert_eq!(event.command.as_deref(),Some("tools"));assert_eq!(event.args.as_deref(),Some("count 7"));}
    let me=ctx.get_me().await?;assert_eq!(me.username,"bot");
    let count=ctx.state::<State>()?.calls.fetch_add(1,Ordering::SeqCst)+1;
    ctx.reply(format!("counted {count}")).await
 })
 .command("revoke","Revoke",|ctx|async move{ctx.state::<State>()?.allowed.store(false,Ordering::SeqCst);ctx.reply("revoked").await})
 .command("uncertain","Partial",|ctx|async move{
    let first=ctx.say("confirmed").await?;let cause=ctx.say("uncertain").await.unwrap_err();
    Err(Error::PartialWrite{completed:vec![serde_json::json!({"message":first})],failed_step:2,source:Box::new(cause)})
 })
 .on_interaction(|_|async{panic!("registered worker command reached generic UI listener")})
 .run().await
}
"#).unwrap();
    let callback = |id| {
        format!(
            "POST /interactions/{}/interaction_fixture/callback",
            Uuid::from_u128(id)
        )
    };
    let (base, mut requests, http) = support::routed_mock(vec![
        (
            "GET /me".into(),
            vec![
                support::response(520, json!({"error":"edge"})),
                support::response(200, support::profile()),
                support::response(200, support::profile()),
            ],
        ),
        (
            format!("POST /channels/{}/messages", Uuid::nil()),
            vec![
                support::response(200, receipt()),
                support::response(200, receipt()),
                support::response(200, receipt()),
                support::response(502, json!({"error":"uncertain"})),
            ],
        ),
        (callback(2), vec![support::response(200, json!({}))]),
        (callback(4), vec![support::response(200, json!({}))]),
        (callback(6), vec![support::response(200, json!({}))]),
    ])
    .await;
    let failures = Arc::new(AtomicUsize::new(0));
    let f = failures.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .on_command_failure(move |ctx, failure| {
            let f = f.clone();
            async move {
                assert!(ctx.interaction().is_some());
                assert_eq!(failure.kind, HandlerKind::Command);
                f.fetch_add(1, Ordering::SeqCst);
                if failure.class == FailureClass::Rejected {
                    Ok(FailureDisposition::Default)
                } else {
                    assert_eq!(failure.error.status, Some(502));
                    assert_eq!(
                        failure.error.details["completed_steps"]
                            .as_array()
                            .unwrap()
                            .len(),
                        1
                    );
                    Ok(FailureDisposition::Stop)
                }
            }
        })
        .build()
        .unwrap();
    seed(&bot).await;
    let reload = hot_reload::HotReload::load(
        bot.handle(),
        hot_reload::WorkerProject::new(project.0.join("Cargo.toml"), "unified-command-worker")
            .offline(true),
    )
    .await
    .unwrap();
    bot.process_event(message("short", "7", 1)).await.unwrap();
    bot.process_event(interaction("short", "101", 2))
        .await
        .unwrap();
    bot.process_event(message("revoke", "", 3)).await.unwrap();
    bot.process_event(interaction("short", "7", 4))
        .await
        .unwrap();
    let error = bot
        .process_event(interaction("uncertain", "", 5))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(
        error.to_json()["completed_steps"].as_array().unwrap().len(),
        1
    );
    reload.reload().await.unwrap();
    bot.process_event(interaction("tools", "count 7", 6))
        .await
        .unwrap();
    assert_eq!(failures.load(Ordering::SeqCst), 3);
    assert_eq!(bot.client().read_availability_status().recoveries, 1);
    http.await.unwrap();
    let mut writes = vec![];
    while let Ok(request) = requests.try_recv() {
        if request.starts_with("POST ") {
            writes.push(request);
        }
    }
    assert_eq!(
        writes
            .iter()
            .filter(|request| request.contains("\"content\":\"uncertain\""))
            .count(),
        1
    );
    assert!(writes.last().unwrap().contains("counted 1"));
}

#[tokio::test]
async fn disabling_startup_recovery_does_not_inherit_handler_read_retries() {
    let (base, _requests, http) =
        support::mock(vec![support::response(520, json!({"error":"edge"}))]).await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.transport.availability = None;
            settings.transport.max_read_retries = 0;
        })
        .command("must-not-run", "Must not run", |_| async {
            panic!("startup failure executed a handler")
        })
        .build()
        .unwrap();
    let client = bot.client().clone();
    let error = bot.run().await.unwrap_err();
    assert_eq!(error.status_code(), Some(520));
    assert!(error.availability_failure().is_none());
    assert_eq!(client.request_metrics().attempts, 1);
    assert_eq!(client.read_availability_status().episodes, 0);
    http.await.unwrap();
}
