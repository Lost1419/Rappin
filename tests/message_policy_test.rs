mod support;
use async_trait::async_trait;
use rappin::prelude::*;
use rappin::{BotProfile, EventCursor};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use support::{profile, response, routed_mock};
use uuid::Uuid;

type Trace = Arc<Mutex<Vec<String>>>;
fn record(trace: &Trace, name: &str) {
    trace.lock().unwrap().push(name.into());
}
fn event(id: u64, content: &str) -> Value {
    json!({"id":id,"type":"message","data":{"id":Uuid::from_u128(id as u128),"channel_id":Uuid::nil(),"server_id":Uuid::nil(),"author_id":Uuid::nil(),"content":content}})
}
fn message() -> Value {
    json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-11-03T00:00:00Z"}})
}
fn rejected() -> YappinError {
    CommandError::Denied("access rejected".into()).into()
}
fn uncertain() -> YappinError {
    YappinError::PartialWrite {
        completed: vec![json!({"receipt":"confirmed-step"})],
        failed_step: 2,
        source: Box::new(YappinError::Api {
            status: 503,
            message: "outcome unknown".into(),
            retry_after: Some(0.25),
        }),
    }
}
async fn seed(bot: &Bot) {
    bot.cache()
        .set_me(serde_json::from_value::<BotProfile>(profile()).unwrap())
        .await;
}
async fn process(bot: &Bot, id: u64, content: &str) -> Result<()> {
    bot.process_event(serde_json::from_value(event(id, content)).unwrap())
        .await
}
async fn run(bot: Bot) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), bot.run())
        .await
        .expect("intake stalled")
}
struct Cursor(PathBuf);
impl Cursor {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("rappin-message-policy-{}", Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let path = root.join("cursor.json");
        EventCursor::new(&path).save(1).await.unwrap();
        Self(path)
    }
    async fn last(&self) -> u64 {
        EventCursor::new(&self.0).load().await.unwrap().unwrap()
    }
}
impl Drop for Cursor {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
struct Legacy(Trace);
#[async_trait]
impl EventHandler for Legacy {
    async fn on_message(&self, _: &MessageContext) {
        record(&self.0, "legacy");
    }
}
struct Probe(Trace);
#[async_trait]
impl Cog for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn commands(&self) -> Vec<CogCommand> {
        let t = self.0.clone();
        vec![CogCommand::new("ping", "test", move |_| {
            let t = t.clone();
            async move {
                record(&t, "command");
                Ok(())
            }
        })]
    }
    async fn check(&self, _: &MessageContext) -> bool {
        record(&self.0, "check");
        true
    }
    async fn on_message(&self, _: &MessageContext) {
        record(&self.0, "cog");
    }
}
#[tokio::test]
async fn gate_precedes_prefix_checks_commands_and_legacy_observers() {
    let t = Trace::default();
    let g = t.clone();
    let p = t.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .message_gate(move |ctx| {
            let t = g.clone();
            async move {
                assert!(ctx.command.is_none());
                assert!(ctx.args.is_empty());
                record(&t, "gate");
                Ok(MessageDisposition::Continue)
            }
        })
        .prefixes_with(move |_| {
            let t = p.clone();
            async move {
                record(&t, "prefix");
                Ok(vec!["?".into()])
            }
        })
        .event_handler(Legacy(t.clone()))
        .build();
    seed(&bot).await;
    bot.handle().add_cog(Probe(t.clone())).await.unwrap();
    process(&bot, 2, "?ping").await.unwrap();
    assert_eq!(
        *t.lock().unwrap(),
        ["gate", "prefix", "check", "command", "legacy", "cog"]
    );
}
#[tokio::test]
async fn handled_rejected_and_stopped_gates_execute_zero_downstream_handlers() {
    for mode in 0..3 {
        let t = Trace::default();
        let p = t.clone();
        let (base, mut requests, http) = support::mock(if mode == 1 {
            vec![response(201, message())]
        } else {
            vec![]
        })
        .await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .message_gate(move |_| async move {
                match mode {
                    0 => Ok(MessageDisposition::Handled),
                    1 => Err(rejected()),
                    _ => Ok(MessageDisposition::Stop),
                }
            })
            .prefixes_with(move |_| {
                let t = p.clone();
                async move {
                    record(&t, "prefix");
                    Ok(vec!["?".into()])
                }
            })
            .event_handler(Legacy(t.clone()))
            .build();
        seed(&bot).await;
        bot.handle().add_cog(Probe(t.clone())).await.unwrap();
        let result = process(&bot, 2, "?ping").await;
        assert_eq!(result.is_ok(), mode != 2);
        assert!(t.lock().unwrap().is_empty());
        if mode == 2 {
            assert_eq!(result.unwrap_err().to_json()["kind"], "message_stopped");
        }
        if mode == 1 {
            assert!(requests.recv().await.unwrap().contains("access rejected"));
        }
        http.await.unwrap();
    }
}

struct Natural {
    trace: Trace,
    fail: bool,
    handled: bool,
}

#[tokio::test]
async fn completing_a_failed_gate_never_dispatches_denied_messages() {
    for mode in 0..3 {
        let t = Trace::default();
        let (base, _requests, http) = support::mock(vec![if mode == 2 {
            response(502, json!({"error":"reply uncertain"}))
        } else {
            response(201, message())
        }])
        .await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .message_gate(
                move |_| async move { Err(if mode == 0 { rejected() } else { uncertain() }) },
            )
            .on_message_failure(|ctx, failure| async move {
                assert_eq!(failure.stage, MessageStage::Gate);
                ctx.reply("application completed the rejection or reconciled its workflow")
                    .await?;
                Ok(FailureDisposition::Handled)
            })
            .event_handler(Legacy(t.clone()))
            .build();
        seed(&bot).await;
        bot.handle().add_cog(Probe(t.clone())).await.unwrap();
        let result = process(&bot, 2, "!ping").await;
        assert!(t.lock().unwrap().is_empty());
        if mode == 2 {
            let e = result.unwrap_err().to_json();
            assert_eq!(e["kind"], "failure_handling");
            assert_eq!(e["source"], uncertain().to_json());
            assert_eq!(e["handling_error"]["status"], 502);
        } else {
            result.unwrap();
        }
        http.await.unwrap();
    }
}
#[async_trait]
impl EventHandler for Natural {
    async fn on_message_result(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
        record(&self.trace, "natural");
        if self.fail {
            return Err(uncertain());
        }
        if ctx.content() == "question" {
            return Err(rejected());
        }
        Ok(if self.handled {
            MessageDisposition::Handled
        } else {
            MessageDisposition::Continue
        })
    }
}
#[tokio::test]
async fn natural_language_handled_skips_cogs_and_rejection_completes_only_its_listener() {
    for handled in [false, true] {
        let t = Trace::default();
        let (base, _requests, http) = support::mock(if handled {
            vec![]
        } else {
            vec![response(201, message())]
        })
        .await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .event_handler(Natural {
                trace: t.clone(),
                fail: false,
                handled,
            })
            .build();
        seed(&bot).await;
        bot.handle().add_cog(Probe(t.clone())).await.unwrap();
        process(&bot, 2, if handled { "ordinary text" } else { "question" })
            .await
            .unwrap();
        let expected = if handled {
            vec!["natural"]
        } else {
            vec!["natural", "cog"]
        };
        assert_eq!(*t.lock().unwrap(), expected);
        http.await.unwrap();
    }
}
#[tokio::test]
async fn message_failures_preserve_original_effects_callback_errors_and_dispositions() {
    for mode in 0..5 {
        let t = Trace::default();
        let seen = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let (base, _requests, http) = support::mock(if mode == 4 {
            vec![response(502, json!({"error":"reply uncertain"}))]
        } else {
            vec![]
        })
        .await;
        let mut builder = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .event_handler(Natural {
                trace: t.clone(),
                fail: true,
                handled: false,
            });
        if mode != 0 {
            builder = builder.on_message_failure(move |ctx, failure| {
                let s = s.clone();
                async move {
                    s.lock().unwrap().push(failure);
                    match mode {
                        1 => Ok(FailureDisposition::Handled),
                        2 => Ok(FailureDisposition::Default),
                        3 => Ok(FailureDisposition::Stop),
                        _ => {
                            ctx.reply("failed").await?;
                            Ok(FailureDisposition::Handled)
                        }
                    }
                }
            });
        }
        let bot = builder.build();
        seed(&bot).await;
        bot.handle().add_cog(Probe(t.clone())).await.unwrap();
        let result = process(&bot, 2, "natural language").await;
        if mode == 1 {
            result.unwrap();
            assert_eq!(*t.lock().unwrap(), ["natural", "cog"]);
        } else {
            let error = result.unwrap_err();
            assert_eq!(*t.lock().unwrap(), ["natural"]);
            assert_eq!(error.status_code(), Some(503));
            assert_eq!(error.retry_after(), Some(Duration::from_millis(250)));
            let data = error.to_json();
            let cause = if mode == 4 { &data["source"] } else { &data };
            assert_eq!(*cause, uncertain().to_json());
            if mode == 4 {
                assert_eq!(data["handling_error"]["status"], 502);
            }
        }
        if mode != 0 {
            let s = seen.lock().unwrap();
            assert_eq!(s[0].stage, MessageStage::Listener);
            assert_eq!(s[0].event_id, 2);
            assert_eq!(s[0].error.to_json(), uncertain().to_json());
        }
        http.await.unwrap();
    }
}
#[tokio::test]
async fn duplicate_intake_commits_handled_gate_once_then_runs_subsequent_command() {
    let cursor = Cursor::new().await;
    let t = Trace::default();
    let g = t.clone();
    let c = t.clone();
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"denied"),event(2,"denied"),event(3,"!next")],"last":3}),
            )],
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&cursor.0)
        .handler_concurrency(1)
        .message_gate(move |ctx| {
            let t = g.clone();
            async move {
                record(&t, ctx.content());
                Ok(if ctx.content() == "denied" {
                    MessageDisposition::Handled
                } else {
                    MessageDisposition::Continue
                })
            }
        })
        .command("next", "next", move |ctx| {
            let t = c.clone();
            async move {
                record(&t, "next");
                ctx.bot().unwrap().stop();
                Ok(())
            }
        })
        .build();
    run(bot).await.unwrap();
    assert_eq!(cursor.last().await, 3);
    assert_eq!(*t.lock().unwrap(), ["denied", "!next", "next"]);
    http.await.unwrap();
}
#[tokio::test]
async fn failed_gate_or_error_reply_retains_prefix_and_never_dispatches_later_messages() {
    for mode in 0..4 {
        let cursor = Cursor::new().await;
        let t = Trace::default();
        let c = t.clone();
        let mut routes = vec![
            ("GET /me".into(), vec![response(200, profile())]),
            (
                "GET /events".into(),
                vec![response(
                    200,
                    json!({"events":[event(2,"handled"),event(3,"bad"),event(4,"!later")],"last":4}),
                )],
            ),
        ];
        if mode == 2 {
            routes.push((
                format!("POST /channels/{}/messages", Uuid::nil()),
                vec![response(502, json!({"error":"reply uncertain"}))],
            ));
        }
        let (base, _requests, http) = routed_mock(routes).await;
        let mut builder = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .cursor_file(&cursor.0)
            .handler_concurrency(1)
            .message_gate(move |ctx| async move {
                if ctx.content() == "handled" {
                    return Ok(MessageDisposition::Handled);
                }
                match mode {
                    0 => Ok(MessageDisposition::Stop),
                    2 => Err(rejected()),
                    3 => panic!("gate panicked"),
                    _ => Err(uncertain()),
                }
            })
            .command("later", "must not run", move |_| {
                let t = c.clone();
                async move {
                    record(&t, "later");
                    Ok(())
                }
            });
        if mode == 1 {
            builder = builder.on_message_failure(|_, f| async move {
                assert_eq!(f.error.to_json(), uncertain().to_json());
                Ok(FailureDisposition::Default)
            });
        }
        let error = run(builder.build()).await.unwrap_err();
        let report = match error {
            YappinError::EventProcessing { report } => report,
            other => panic!("{other:?}"),
        };
        assert_eq!(cursor.last().await, 2);
        assert_eq!(report.failed_events[0].event_id, 3);
        assert_eq!(report.unprocessed_events, [4]);
        assert!(t.lock().unwrap().is_empty());
        if mode == 1 {
            assert_eq!(
                report.failed_events[0].error.to_json(),
                uncertain().to_json()
            );
        }
        if mode == 2 {
            let e = report.failed_events[0].error.to_json();
            assert_eq!(e["kind"], "failure_handling");
            assert_eq!(e["source"], rejected().to_json());
            assert_eq!(e["handling_error"]["status"], 502);
        }
        if mode == 3 {
            assert_eq!(report.failed_events[0].error.kind, "handler_interrupted");
        }
        http.await.unwrap();
    }
}

// Public compatibility: these old exhaustive matches must continue compiling.
#[test]
fn existing_handler_kind_still_has_exactly_the_original_variants() {
    fn name(kind: HandlerKind) -> &'static str {
        match kind {
            HandlerKind::Command => "command",
            HandlerKind::Button => "button",
        }
    }
    assert_eq!(name(HandlerKind::Command), "command");
}

struct FallibleCog(Trace);
#[async_trait]
impl Cog for FallibleCog {
    fn name(&self) -> &str {
        "natural_cog"
    }
    async fn on_message_result(&self, _: &MessageContext) -> Result<MessageDisposition> {
        record(&self.0, "fallible_cog");
        Err(uncertain())
    }
}
#[tokio::test]
async fn native_cog_listener_failure_has_its_own_identity_and_explicit_completion() {
    let t = Trace::default();
    let c = t.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .event_handler(Legacy(t.clone()))
        .on_message_failure(move |_, failure| {
            let t = c.clone();
            async move {
                assert_eq!(failure.handler, "natural_cog");
                assert_eq!(failure.stage, MessageStage::Listener);
                assert_eq!(failure.error.to_json(), uncertain().to_json());
                record(&t, "failure");
                Ok(FailureDisposition::Handled)
            }
        })
        .build();
    seed(&bot).await;
    bot.handle().add_cog(FallibleCog(t.clone())).await.unwrap();
    process(&bot, 2, "natural language").await.unwrap();
    assert_eq!(*t.lock().unwrap(), ["legacy", "fallible_cog", "failure"]);
}

#[tokio::test]
async fn concurrent_gate_failure_drains_started_work_without_crossing_the_cursor_barrier() {
    let cursor = Cursor::new().await;
    let t = Trace::default();
    let c = t.clone();
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"!slow"),event(3,"bad"),event(4,"!later")],"last":4}),
            )],
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&cursor.0)
        .handler_concurrency(2)
        .message_gate(move |ctx| {
            let entered = entered.clone();
            let resume = resume.clone();
            async move {
                if ctx.content() == "!slow" {
                    entered.notify_one();
                    resume.notified().await;
                    Ok(MessageDisposition::Continue)
                } else if ctx.content() == "bad" {
                    entered.notified().await;
                    resume.notify_one();
                    Err(uncertain())
                } else {
                    panic!("not-started event entered gate")
                }
            }
        })
        .command("slow", "drain", move |_| {
            let t = c.clone();
            async move {
                record(&t, "drained");
                Ok(())
            }
        })
        .command("later", "must not start", |_| async {
            panic!("barrier crossed")
        })
        .build();
    let error = run(bot).await.unwrap_err();
    let report = match error {
        YappinError::EventProcessing { report } => report,
        other => panic!("{other:?}"),
    };
    assert_eq!(cursor.last().await, 2);
    assert_eq!(report.failed_events[0].event_id, 3);
    assert_eq!(report.unprocessed_events, [4]);
    assert_eq!(*t.lock().unwrap(), ["drained"]);
    http.await.unwrap();
}
