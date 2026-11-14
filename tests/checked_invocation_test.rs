mod support;
use async_trait::async_trait;
use rappin::*;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use uuid::Uuid;

fn event(id: u64, command: &str) -> Event {
    serde_json::from_value(json!({"id":id,"type":"message","data":{"id":Uuid::from_u128(id as u128),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::from_u128(42),"command":command,"content":"original content"}})).unwrap()
}
async fn seed(bot: &Bot) {
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
}
struct Roll(BoundedU32<2, 100>);
impl FromArgs for Roll {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self(args.required("sides")?))
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![ArgumentInfo::required("sides", "2 to 100")]
    }
}
fn deny() -> Error {
    CommandError::Denied("revoked".into()).into()
}

#[tokio::test]
async fn aliases_groups_typed_checks_and_original_identity_share_dispatch() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command_spec(
            CommandSpec::<Roll>::new("dice roll", "roll").alias("roll"),
            move |ctx, value| {
                let c = c.clone();
                async move {
                    assert_eq!(value.0 .0, 20);
                    assert_eq!(ctx.author_id(), Uuid::from_u128(42));
                    assert_eq!(ctx.channel_id(), Uuid::nil());
                    assert_eq!(ctx.server_id(), Some(Uuid::nil()));
                    assert_eq!(ctx.event.id, Uuid::from_u128(7));
                    assert_eq!(ctx.content(), "original content");
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .command("outer", "invoke", |ctx| async move {
            let bot = ctx.bot.as_ref().unwrap();
            for (name, args) in [("dice", "roll 20"), ("ROLL", "20")] {
                let plan = bot.prepare_invocation(&ctx, name, args).await?;
                assert!(plan.report().deferred.iter().any(|s| s.contains("typed")));
                bot.invoke_prepared(plan).await.into_result()?;
            }
            assert_eq!(
                bot.invoke_command(&ctx, "roll", "101").await.status,
                InvocationStatus::Denied
            );
            assert_eq!(
                bot.invoke_command(&ctx, "absent", "").await.status,
                InvocationStatus::NotFound
            );
            Ok(())
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event(7, "outer")).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

struct Access(Arc<AtomicBool>);
impl CommandCheck for Access {
    fn check<'a>(&'a self, _: &'a MessageContext) -> commands::CheckFuture<'a> {
        Box::pin(async {
            if self.0.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(deny())
            }
        })
    }
    fn prepare<'a>(&'a self, ctx: &'a MessageContext) -> Option<commands::CheckFuture<'a>> {
        Some(self.check(ctx))
    }
}
#[tokio::test]
async fn preparation_never_grants_access_and_revocation_runs_zero_handlers() {
    let access = Arc::new(AtomicBool::new(true));
    let a = access.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command_spec(
            CommandSpec::<()>::new("private", "private").check(Access(access)),
            |_, _| async { panic!("denied handler") },
        )
        .command("outer", "invoke", move |ctx| {
            let a = a.clone();
            async move {
                let bot = ctx.bot.as_ref().unwrap();
                let plan = bot
                    .prepare_invocation_with_operation_id(
                        &ctx,
                        "private",
                        "",
                        OperationId::new("assessed-step")?,
                    )
                    .await?;
                assert!(plan.report().assessed.iter().any(|s| s.contains("check")));
                a.store(false, Ordering::SeqCst);
                assert_eq!(
                    bot.invoke_prepared(plan).await.status,
                    InvocationStatus::Denied
                );
                assert!(bot
                    .prepare_invocation_with_operation_id(
                        &ctx,
                        "private",
                        "",
                        OperationId::new("assessed-step")?
                    )
                    .await
                    .unwrap_err()
                    .is_command_rejection());
                assert_eq!(
                    bot.invoke_command_with_operation_id(
                        &ctx,
                        "private",
                        "",
                        OperationId::new("denied-step")?
                    )
                    .await
                    .status,
                    InvocationStatus::Denied
                );
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event(2, "outer")).await.unwrap();
}

static PARSED: AtomicUsize = AtomicUsize::new(0);
struct Opaque;
impl FromArgs for Opaque {
    fn parse(_: &mut Arguments) -> std::result::Result<Self, CommandError> {
        PARSED.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![]
    }
}
#[tokio::test]
async fn preparation_defers_arbitrary_parsers_checks_and_cooldowns() {
    let checks = Arc::new(AtomicUsize::new(0));
    let checked = checks.clone();
    let cd = cooldown::Cooldown::user(Duration::from_secs(60));
    let cool = cd.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command_spec(
            CommandSpec::<Opaque>::new("opaque", "opaque").check(check_fn(move |ctx| {
                let checks = checks.clone();
                let cd = cd.clone();
                async move {
                    checks.fetch_add(1, Ordering::SeqCst);
                    if cd.check(&ctx).await.is_some() {
                        Err(deny())
                    } else {
                        Ok(())
                    }
                }
            })),
            |_, _| async { Ok(()) },
        )
        .command("outer", "invoke", move |ctx| {
            let checked = checked.clone();
            let cool = cool.clone();
            async move {
                let bot = ctx.bot.as_ref().unwrap();
                for _ in 0..2 {
                    let plan = bot.prepare_invocation(&ctx, "opaque", "").await?;
                    assert!(plan.report().deferred.iter().any(|s| s.contains("check")));
                }
                assert_eq!(PARSED.load(Ordering::SeqCst), 0);
                assert_eq!(checked.load(Ordering::SeqCst), 0);
                bot.invoke_command_with_operation_id(
                    &ctx,
                    "opaque",
                    "",
                    OperationId::new("distinct-step")?,
                )
                .await
                .into_result()?;
                assert_eq!(
                    bot.invoke_command_with_operation_id(
                        &ctx,
                        "opaque",
                        "",
                        OperationId::new("another-step")?
                    )
                    .await
                    .status,
                    InvocationStatus::Denied
                );
                assert!(cool.check(&ctx).await.is_some());
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event(3, "outer")).await.unwrap();
    assert_eq!(PARSED.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn plans_reject_replacement_and_unload_without_registry_locking_handlers() {
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("outer", "invoke", |ctx| async move {
            let bot = ctx.bot.as_ref().unwrap();
            let plan = bot
                .prepare_invocation_with_operation_id(
                    &ctx,
                    "target",
                    "",
                    OperationId::new("planned-step")?,
                )
                .await?;
            bot.reload_cog(
                CogBuilder::new("target-cog")
                    .command("target", "new", |_| async { Err(deny()) })
                    .build(),
            )
            .await?;
            let stale = bot.invoke_prepared(plan).await;
            assert_eq!(stale.operation_id().unwrap().as_str(), "planned-step");
            assert_eq!(stale.status, InvocationStatus::Failed);
            assert!(!stale.effects_possible);
            assert_eq!(
                bot.invoke_command(&ctx, "target", "").await.status,
                InvocationStatus::Denied
            );
            let plan = bot
                .prepare_invocation_with_operation_id(
                    &ctx,
                    "target",
                    "",
                    OperationId::new("planned-step")?,
                )
                .await?;
            bot.remove_cog("target-cog").await?;
            assert_eq!(
                bot.invoke_prepared(plan).await.status,
                InvocationStatus::Failed
            );
            Ok(())
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.handle()
        .add_cog(
            CogBuilder::new("target-cog")
                .command("target", "old", |_| async { panic!("stale handler") })
                .build(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), bot.process_event(event(4, "outer")))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn identity_replacement_detached_tasks_and_depth_are_rejected() {
    let seen = Arc::new(AtomicUsize::new(0));
    let s = seen.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("loop", "recursive", move |ctx| {
            let s = s.clone();
            async move {
                s.fetch_add(1, Ordering::SeqCst);
                ctx.bot
                    .as_ref()
                    .unwrap()
                    .invoke_command(&ctx, "loop", "")
                    .await
                    .into_result()
            }
        })
        .command("outer", "invoke", |ctx| async move {
            let bot = ctx.bot.as_ref().unwrap();
            let mut forged = ctx.clone();
            forged.event.author_id = Uuid::nil();
            assert!(
                !bot.invoke_command(&forged, "loop", "")
                    .await
                    .effects_possible
            );
            forged = ctx.clone();
            forged.event.channel_id = Uuid::from_u128(99);
            assert!(
                !bot.invoke_command(&forged, "loop", "")
                    .await
                    .effects_possible
            );
            let detached = ctx.clone();
            let outcome = tokio::spawn(async move {
                detached
                    .bot
                    .as_ref()
                    .unwrap()
                    .invoke_command(&detached, "loop", "")
                    .await
            })
            .await
            .unwrap();
            assert_eq!(outcome.status, InvocationStatus::Failed);
            assert!(!outcome.effects_possible);
            let nested = bot.invoke_command(&ctx, "loop", "").await;
            assert_eq!(nested.status, InvocationStatus::Failed);
            let error = nested.into_result().unwrap_err();
            assert!(!error.is_command_rejection());
            assert_eq!(
                error.to_remote().details["retry_safety"],
                "do_not_replay_workflow"
            );
            Ok(())
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event(5, "outer")).await.unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 7);
}

#[tokio::test]
async fn gate_rechecks_and_cog_checks_deny_without_target_execution() {
    let allowed = Arc::new(AtomicBool::new(true));
    let a = allowed.clone();
    let gatecalls = Arc::new(AtomicUsize::new(0));
    let gc = gatecalls.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .message_gate(move |_| {
            let a = a.clone();
            let gc = gc.clone();
            async move {
                gc.fetch_add(1, Ordering::SeqCst);
                Ok(if a.load(Ordering::SeqCst) {
                    MessageDisposition::Continue
                } else {
                    MessageDisposition::Handled
                })
            }
        })
        .command("target", "target", |_| async { panic!("gate denied") })
        .command("outer", "invoke", move |ctx| {
            let allowed = allowed.clone();
            async move {
                let bot = ctx.bot.as_ref().unwrap();
                let plan = bot.prepare_invocation(&ctx, "target", "").await?;
                allowed.store(false, Ordering::SeqCst);
                assert_eq!(
                    bot.invoke_prepared(plan).await.status,
                    InvocationStatus::Denied
                );
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event(6, "outer")).await.unwrap();
    assert_eq!(gatecalls.load(Ordering::SeqCst), 2);
}

struct Listener(Arc<AtomicUsize>);
#[async_trait]
impl EventHandler for Listener {
    async fn on_message(&self, _: &MessageContext) -> rappin::Result<rappin::MessageDisposition> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(rappin::MessageDisposition::Continue)
    }
}
#[tokio::test]
async fn partial_and_failed_reply_causes_reach_one_outer_callback_without_automatic_replies() {
    let (base, mut requests, http) = support::mock(vec![(
        502,
        vec![("Retry-After", "0.25")],
        json!({"error":"uncertain reply"}).to_string(),
    )])
    .await;
    let observed = Arc::new(Mutex::new(vec![]));
    let o = observed.clone();
    let listeners = Arc::new(AtomicUsize::new(0));
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .event_handler(Listener(listeners.clone()))
        .command("reply", "reply", |ctx| async move {
            ctx.reply("attempt once").await?;
            Ok(())
        })
        .command("partial", "partial", |_| async {
            Err(Error::PartialWrite {
                completed: vec![json!({"receipt":"step-one"})],
                failed_step: 2,
                source: Box::new(Error::Api {
                    status: 502,
                    message: "uncertain".into(),
                    retry_after: Some(0.5),
                }),
            })
        })
        .command("outer", "invoke", |ctx| async move {
            let name = if ctx.event.id == Uuid::from_u128(8) {
                "reply"
            } else {
                "partial"
            };
            ctx.bot
                .as_ref()
                .unwrap()
                .invoke_command(&ctx, name, "")
                .await
                .into_result()
        })
        .on_command_failure(move |_, failure| {
            let o = o.clone();
            async move {
                o.lock().unwrap().push(failure);
                Ok(FailureDisposition::Stop)
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    for id in [8, 9] {
        let err = bot.process_event(event(id, "outer")).await.unwrap_err();
        assert_eq!(err.to_json()["kind"], "command_invocation");
        assert!(!err.is_command_rejection());
    }
    http.await.unwrap();
    assert!(requests.recv().await.unwrap().contains("attempt once"));
    assert!(requests.try_recv().is_err());
    assert_eq!(listeners.load(Ordering::SeqCst), 0);
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert!(observed
        .iter()
        .all(|f| f.handler == "outer" && f.class == FailureClass::Unexpected));
    assert_eq!(observed[0].error.status, Some(502));
    assert_eq!(observed[0].error.retry_after, Some(0.25));
    let partial = observed[1].error.to_json();
    assert_eq!(
        partial["source"]["completed_steps"][0]["receipt"],
        "step-one"
    );
    assert_eq!(partial["source"]["outcome"], "unknown");
}

#[tokio::test]
async fn dynamic_schema_preparation_validates_bounds_and_builtin_scope_without_effects() {
    let schema = CommandSchema {
        name: "dynamic".into(),
        description: "validated".into(),
        aliases: vec!["dyn".into()],
        arguments: vec![ArgumentSchema {
            name: "count".into(),
            description: "bounded".into(),
            kind: ArgumentType::Integer {
                min: Some(2),
                max: Some(10),
            },
            required: true,
            rest: false,
        }],
    };
    let command = DynamicCommandSpec::new(schema)
        .check(Check::server_only())
        .bind(|_, _| async { panic!("preparation executed handler") })
        .unwrap();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .registered_command(command)
        .command("outer", "prepare", |ctx| async move {
            let bot = ctx.bot.as_ref().unwrap();
            let plan = bot.prepare_invocation(&ctx, "dyn", "4").await?;
            assert_eq!(
                plan.report().assessed,
                ["dynamic argument schema", "registered check 0"]
            );
            assert!(bot.prepare_invocation(&ctx, "dynamic", "11").await.is_err());
            assert!(bot
                .prepare_invocation(&ctx, "dynamic", "'unterminated")
                .await
                .is_err());
            Ok(())
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event(10, "outer")).await.unwrap();
}

#[tokio::test]
async fn nested_uncertain_effect_stops_polling_with_duplicate_and_cursor_barrier_preserved() {
    let root = std::env::temp_dir().join(format!("rappin-invocation-cursor-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("cursor.json");
    events::EventCursor::new(&path).save(1).await.unwrap();
    let (base, _requests, http) = support::routed_mock(vec![
        (
            "GET /me".into(),
            vec![support::response(200, support::profile())],
        ),
        (
            "GET /events".into(),
            vec![support::response(
                200,
                json!({"events":[event(2,"outer"),event(2,"outer"),event(3,"later")],"last":3}),
            )],
        ),
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| settings.persistence.cursor = Some(rappin::EventCursor::new(&path)))
        .configure(|settings| settings.execution.concurrency = 1)
        .command("effect", "one attempt", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(Error::Api {
                    status: 502,
                    message: "unknown write".into(),
                    retry_after: None,
                })
            }
        })
        .command("outer", "invoke", |ctx| async move {
            ctx.bot
                .as_ref()
                .unwrap()
                .invoke_command(&ctx, "effect", "")
                .await
                .into_result()
        })
        .command("later", "barrier", |_| async {
            panic!("crossed failure barrier")
        })
        .build()
        .expect("valid bot configuration");
    let err = tokio::time::timeout(Duration::from_secs(3), bot.run())
        .await
        .unwrap()
        .unwrap_err();
    http.await.unwrap();
    let report = match err {
        Error::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.committed_cursor, 1);
    assert_eq!(report.unprocessed_events, [3]);
    assert_eq!(report.failed_events[0].error.kind, "command_invocation");
    assert_eq!(
        events::EventCursor::new(&path).load().await.unwrap(),
        Some(1)
    );
    std::fs::remove_dir_all(root).unwrap();
}
