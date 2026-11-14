mod support;
use rappin::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use uuid::Uuid;
struct Recipient(String);
impl FromArgs for Recipient {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self(args.required("recipient")?))
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![ArgumentInfo::required("recipient", "recipient")]
    }
}
fn id(s: &str) -> OperationId {
    OperationId::new(s).unwrap()
}
fn event() -> Event {
    serde_json::from_value(json!({"id":1,"type":"message","data":{"id":Uuid::from_u128(1),"channel_id":Uuid::from_u128(2),"server_id":Uuid::from_u128(3),"author_id":Uuid::from_u128(4),"command":"outer","content":"authenticated"}})).unwrap()
}
async fn seed(bot: &Bot) {
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
}
fn scope(command: &CommandContext) {
    let ctx = command.message().expect("authenticated message");
    assert_eq!(ctx.message_id(), Uuid::from_u128(1));
    assert_eq!(ctx.channel_id(), Uuid::from_u128(2));
    assert_eq!(ctx.server_id(), Some(Uuid::from_u128(3)));
    assert_eq!(ctx.author_id(), Uuid::from_u128(4));
    assert_eq!(ctx.content(), "authenticated");
}
#[test]
fn bounded_opaque_ids_validate_on_construction_and_deserialization() {
    for invalid in [
        String::new(),
        "x".repeat(129),
        "line\nbreak".into(),
        "\0".into(),
        "é".repeat(65),
    ] {
        assert!(OperationId::new(&invalid).is_err());
        assert!(serde_json::from_value::<OperationId>(json!(invalid)).is_err());
    }
    for valid in ["opaque / : λ".to_owned(), "é".repeat(64)] {
        assert_eq!(serde_json::to_value(id(&valid)).unwrap(), json!(valid));
        assert_eq!(
            serde_json::from_value::<OperationId>(json!(valid)).unwrap(),
            id(&valid)
        );
    }
}
#[tokio::test]
async fn complete_plan_keys_reach_preparation_checks_execution_and_consumer_journal() {
    struct Audit(Arc<Mutex<Vec<(String, bool)>>>);
    impl CommandCheck for Audit {
        fn check<'a>(&'a self, ctx: &'a rappin::CommandContext) -> commands::CheckFuture<'a> {
            Box::pin(async move {
                scope(ctx);
                self.0
                    .lock()
                    .unwrap()
                    .push((ctx.operation_id().unwrap().as_str().into(), false));
                Ok(())
            })
        }
        fn prepare<'a>(
            &'a self,
            ctx: &'a rappin::CommandContext,
        ) -> Option<commands::CheckFuture<'a>> {
            Some(Box::pin(async move {
                scope(ctx);
                self.0
                    .lock()
                    .unwrap()
                    .push((ctx.operation_id().unwrap().as_str().into(), true));
                Ok(())
            }))
        }
    }
    let audit = Arc::new(Mutex::new(vec![]));
    let journal = Arc::new(Mutex::new(HashMap::<OperationId, String>::new()));
    let effects = Arc::new(AtomicUsize::new(0));
    let j = journal.clone();
    let e = effects.clone();
    let count = effects.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command_spec(
            CommandSpec::<Recipient>::new("record", "record")
                .alias("rec")
                .check(Audit(audit.clone())),
            move |ctx, Recipient(recipient)| {
                let j = j.clone();
                let e = e.clone();
                async move {
                    scope(&ctx);
                    let key = ctx.operation_id().unwrap();
                    let mut rows = j.lock().unwrap();
                    if let Some(previous) = rows.get(&key) {
                        if previous != &recipient {
                            return Err(CommandError::Denied(
                                "operation key reused for different arguments".into(),
                            )
                            .into());
                        }
                        return Ok(());
                    }
                    rows.insert(key, recipient);
                    e.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .command("outer", "plan", move |ctx| {
            let count = count.clone();
            async move {
                assert_eq!(ctx.operation_id(), None);
                let bot = ctx.bot().unwrap();
                let first = bot
                    .prepare_invocation_with_operation_id(
                        ctx.message().expect("message-origin invocation"),
                        "rec",
                        "alice",
                        id("step/1"),
                    )
                    .await?;
                let second = bot
                    .prepare_invocation_with_operation_id(
                        ctx.message().expect("message-origin invocation"),
                        "record",
                        "bob",
                        id("step/2"),
                    )
                    .await?;
                assert_eq!(first.operation_id(), Some(&id("step/1")));
                assert_eq!(second.operation_id(), Some(&id("step/2")));
                assert_eq!(count.load(Ordering::SeqCst), 0);
                assert_eq!(ctx.operation_id(), None);
                for plan in [first, second] {
                    let key = plan.operation_id().unwrap().clone();
                    let result = bot.invoke_prepared(plan).await;
                    assert_eq!(result.operation_id(), Some(&key));
                    result.into_result()?;
                }
                // Stable replay/conflict behavior belongs to the consumer journal, not the SDK.
                bot.invoke_command_with_operation_id(
                    ctx.message().expect("message-origin invocation"),
                    "record",
                    "alice",
                    id("step/1"),
                )
                .await
                .into_result()?;
                let conflict = bot
                    .invoke_command_with_operation_id(
                        ctx.message().expect("message-origin invocation"),
                        "record",
                        "charlie",
                        id("step/1"),
                    )
                    .await;
                assert_eq!(conflict.status, InvocationStatus::Denied);
                assert_eq!(count.load(Ordering::SeqCst), 2);
                assert_eq!(
                    conflict.into_result().unwrap_err().to_remote().to_json()["operation_id"],
                    "step/1"
                );
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event()).await.unwrap();
    assert_eq!(journal.lock().unwrap().len(), 2);
    assert_eq!(
        &audit.lock().unwrap()[..2],
        &[("step/1".into(), true), ("step/2".into(), true)]
    );
}
#[tokio::test]
async fn nesting_inherits_or_overrides_and_restores_scope_without_deduplicating() {
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("leaf", "leaf", move |ctx| {
            let s = s.clone();
            async move {
                scope(&ctx);
                s.lock().unwrap().push(ctx.operation_id());
                Ok(())
            }
        })
        .command("middle", "middle", |ctx| async move {
            let bot = ctx.bot().unwrap();
            assert_eq!(ctx.operation_id(), Some(id("parent")));
            let inherited = bot
                .prepare_invocation(
                    ctx.message().expect("message-origin invocation"),
                    "leaf",
                    "",
                )
                .await?;
            assert_eq!(inherited.operation_id(), Some(&id("parent")));
            bot.invoke_prepared(inherited).await.into_result()?;
            bot.invoke_command_with_operation_id(
                ctx.message().expect("message-origin invocation"),
                "leaf",
                "",
                id("child"),
            )
            .await
            .into_result()?;
            assert_eq!(ctx.operation_id(), Some(id("parent")));
            bot.invoke_command(
                ctx.message().expect("message-origin invocation"),
                "leaf",
                "",
            )
            .await
            .into_result()
        })
        .command("outer", "outer", |ctx| async move {
            let bot = ctx.bot().unwrap();
            bot.invoke_command(
                ctx.message().expect("message-origin invocation"),
                "leaf",
                "",
            )
            .await
            .into_result()?;
            bot.invoke_command_with_operation_id(
                ctx.message().expect("message-origin invocation"),
                "middle",
                "",
                id("parent"),
            )
            .await
            .into_result()?;
            assert_eq!(ctx.operation_id(), None);
            // A stable key by itself does not suppress a handler.
            for _ in 0..2 {
                bot.invoke_command_with_operation_id(
                    ctx.message().expect("message-origin invocation"),
                    "leaf",
                    "",
                    id("same"),
                )
                .await
                .into_result()?;
            }
            let detached = ctx.clone();
            assert_eq!(
                tokio::spawn(async move { detached.operation_id() })
                    .await
                    .unwrap(),
                None
            );
            Ok(())
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event()).await.unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            None,
            Some(id("parent")),
            Some(id("child")),
            Some(id("parent")),
            Some(id("same")),
            Some(id("same"))
        ]
    );
}
#[tokio::test]
async fn operation_metadata_does_not_bypass_gate_cog_checks_or_revocation() {
    let allowed = Arc::new(AtomicBool::new(true));
    let a = allowed.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .message_gate(move |ctx| {
            let a = a.clone();
            async move {
                scope(&ctx.clone().into());
                Ok(if a.load(Ordering::SeqCst) {
                    MessageDisposition::Continue
                } else {
                    MessageDisposition::Handled
                })
            }
        })
        .command("target", "target", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .command("outer", "outer", move |ctx| {
            let allowed = allowed.clone();
            async move {
                let bot = ctx.bot().unwrap();
                let plan = bot
                    .prepare_invocation_with_operation_id(
                        ctx.message().expect("message-origin invocation"),
                        "target",
                        "",
                        id("key"),
                    )
                    .await?;
                allowed.store(false, Ordering::SeqCst);
                let out = bot.invoke_prepared(plan).await;
                assert_eq!(out.status, InvocationStatus::Denied);
                assert_eq!(out.operation_id(), Some(&id("key")));
                allowed.store(true, Ordering::SeqCst);
                assert_eq!(
                    bot.invoke_command_with_operation_id(
                        ctx.message().expect("message-origin invocation"),
                        "private",
                        "",
                        id("key")
                    )
                    .await
                    .status,
                    InvocationStatus::Denied
                );
                let mut forged = ctx.message().unwrap().clone();
                forged.event.author_id = Uuid::nil();
                let denied = bot
                    .invoke_command_with_operation_id(&forged, "target", "", id("key"))
                    .await;
                assert!(!denied.effects_possible);
                assert_eq!(denied.status, InvocationStatus::Failed);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    struct Private;
    #[async_trait::async_trait]
    impl Cog for Private {
        fn name(&self) -> &str {
            "private"
        }
        fn commands(&self) -> Vec<CogCommand> {
            vec![CogCommand::new("private", "private", |_| async {
                panic!("denied handler")
            })]
        }
        async fn check(&self, ctx: &rappin::CommandContext) -> rappin::Result<bool> {
            scope(ctx);
            assert_eq!(ctx.operation_id(), Some(id("key")));
            Ok(false)
        }
    }
    bot.handle().add_cog(Private).await.unwrap();
    bot.process_event(event()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn partial_uncertain_causes_keep_operation_identity_through_failure_callbacks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let callback = Arc::new(AtomicUsize::new(0));
    let cb = callback.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("effect", "effect", move |ctx| {
            let c = c.clone();
            async move {
                assert_eq!(ctx.operation_id(), Some(id("uncertain-step")));
                c.fetch_add(1, Ordering::SeqCst);
                Err(Error::PartialWrite {
                    completed: vec![json!({"operation_id":"uncertain-step","receipt":"confirmed"})],
                    failed_step: 2,
                    source: Box::new(Error::Api {
                        status: 502,
                        message: "unknown".into(),
                        retry_after: Some(0.25),
                    }),
                })
            }
        })
        .command("outer", "outer", |ctx| async move {
            ctx.bot()
                .unwrap()
                .invoke_command_with_operation_id(
                    ctx.message().expect("message-origin invocation"),
                    "effect",
                    "",
                    id("uncertain-step"),
                )
                .await
                .into_result()
        })
        .on_command_failure(move |_, failure| {
            let cb = cb.clone();
            async move {
                assert_eq!(failure.error.kind, "command_invocation");
                assert_eq!(failure.error.status, Some(502));
                assert_eq!(failure.error.retry_after, Some(0.25));
                let error = failure.error.to_json();
                assert_eq!(error["operation_id"], "uncertain-step");
                assert_eq!(
                    error["source"]["completed_steps"][0]["receipt"],
                    "confirmed"
                );
                assert_eq!(error["source"]["outcome"], "unknown");
                cb.fetch_add(1, Ordering::SeqCst);
                Ok(FailureDisposition::Stop)
            }
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    assert!(bot.process_event(event()).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(callback.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn concurrent_identified_calls_keep_independent_task_local_metadata() {
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("leaf", "leaf", move |ctx| {
            let s = s.clone();
            async move {
                let before = ctx.operation_id().unwrap();
                tokio::task::yield_now().await;
                assert_eq!(ctx.operation_id(), Some(before.clone()));
                scope(&ctx);
                s.lock().unwrap().push(before);
                Ok(())
            }
        })
        .command("outer", "outer", |ctx| async move {
            let bot = ctx.bot().unwrap();
            let (left, right) = tokio::join!(
                bot.invoke_command_with_operation_id(
                    ctx.message().expect("message-origin invocation"),
                    "leaf",
                    "",
                    id("left")
                ),
                bot.invoke_command_with_operation_id(
                    ctx.message().expect("message-origin invocation"),
                    "leaf",
                    "",
                    id("right")
                ),
            );
            assert_eq!(left.operation_id(), Some(&id("left")));
            assert_eq!(right.operation_id(), Some(&id("right")));
            left.into_result()?;
            right.into_result()?;
            assert_eq!(ctx.operation_id(), None);
            Ok(())
        })
        .build()
        .expect("valid bot configuration");
    seed(&bot).await;
    bot.process_event(event()).await.unwrap();
    let mut keys = seen
        .lock()
        .unwrap()
        .iter()
        .map(|key| key.as_str().to_owned())
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys, ["left", "right"]);
}
