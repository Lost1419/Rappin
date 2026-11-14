mod support;
use rappin::*;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use support::{event, profile, response, routed_mock};

struct Cursor(PathBuf);
impl Cursor {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("rappin-failures-{}", uuid::Uuid::new_v4()));
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
fn button(id: u64, prefix: &str) -> Value {
    json!({"id":id,"type":"interaction","data":{"id":uuid::Uuid::from_u128(id as u128),"token":format!("interaction_{id}"),"custom_id":prefix,"channel_id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil()}})
}
fn message() -> Value {
    json!({"message":{"id":uuid::Uuid::nil(),"channel_id":uuid::Uuid::nil(),"author_id":uuid::Uuid::nil(),"created_at":"2026-11-01T00:00:00Z"}})
}
fn rejected() -> Error {
    CommandError::Missing("target (use an ID to clarify)".into()).into()
}
fn uncertain() -> Error {
    Error::PartialWrite {
        completed: vec![json!({"receipt":"confirmed-first-step"})],
        failed_step: 2,
        source: Box::new(Error::Api {
            status: 502,
            message: "uncertain mutation".into(),
            retry_after: None,
        }),
    }
}
fn report(error: Error) -> EventProcessingFailure {
    match error {
        Error::EventProcessing { report } => *report,
        other => panic!("expected event processing report: {other:?}"),
    }
}
async fn run(bot: Bot) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(8), bot.run())
        .await
        .expect("bot stalled")
}

#[tokio::test]
async fn native_handled_command_and_button_failures_commit_then_run_next_on_same_intake() {
    for is_button in [false, true] {
        for mode in [0, 1, 2, 3] {
            // default rejection, callback rejection, reconciled unexpected failure
            let cursor = Cursor::new().await;
            let bad = if is_button {
                button(2, "bad:")
            } else {
                event(2, "bad")
            };
            let reply = if is_button { json!({}) } else { message() };
            let route = if is_button {
                format!(
                    "POST /interactions/{}/interaction_2/callback",
                    uuid::Uuid::from_u128(2)
                )
            } else {
                format!("POST /channels/{}/messages", uuid::Uuid::nil())
            };
            let (base, _requests, http) = routed_mock(vec![
                ("GET /me".into(), vec![response(200, profile())]),
                (
                    "GET /events".into(),
                    vec![response(
                        200,
                        json!({"events":[bad,event(3,"next")],"last":3}),
                    )],
                ),
                (route, vec![response(200, reply)]),
            ])
            .await;
            let attempts = Arc::new(AtomicUsize::new(0));
            let a = attempts.clone();
            let b = attempts.clone();
            let next = Arc::new(AtomicUsize::new(0));
            let n = next.clone();
            let observed = Arc::new(Mutex::new(vec![]));
            let command_seen = observed.clone();
            let button_seen = observed.clone();
            let mut builder = Bot::builder("fixture")
                .base_url(base)
                .sync_commands(false)
                .log(false)
                .configure(|settings| {
                    settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
                })
                .configure(|settings| settings.execution.concurrency = 1)
                .command("bad", "Fail", move |_| {
                    let a = a.clone();
                    async move {
                        a.fetch_add(1, Ordering::SeqCst);
                        Err(if mode == 2 { uncertain() } else { rejected() })
                    }
                })
                .button("bad:", move |_| {
                    let b = b.clone();
                    async move {
                        b.fetch_add(1, Ordering::SeqCst);
                        Err(if mode == 2 { uncertain() } else { rejected() })
                    }
                })
                .command("next", "Continue", move |ctx| {
                    let n = n.clone();
                    async move {
                        n.fetch_add(1, Ordering::SeqCst);
                        ctx.bot().unwrap().stop();
                        Ok(())
                    }
                });
            if mode != 0 {
                builder = builder
                    .on_command_failure(move |ctx, failure| {
                        let seen = command_seen.clone();
                        async move {
                            seen.lock().unwrap().push(failure);
                            if mode == 3 {
                                return Ok(FailureDisposition::Default);
                            }
                            ctx.reply("Completed rejection/reconciliation").await?;
                            Ok(FailureDisposition::Handled)
                        }
                    })
                    .on_button_failure(move |ctx, failure| {
                        let seen = button_seen.clone();
                        async move {
                            seen.lock().unwrap().push(failure);
                            if mode == 3 {
                                return Ok(FailureDisposition::Default);
                            }
                            ctx.ephemeral("Completed rejection/reconciliation").await?;
                            Ok(FailureDisposition::Handled)
                        }
                    });
            }
            run(builder.build().expect("valid bot configuration"))
                .await
                .unwrap();
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert_eq!(next.load(Ordering::SeqCst), 1);
            assert_eq!(cursor.last().await, 3);
            if mode != 0 {
                let failures = observed.lock().unwrap();
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].event_id, 2);
                assert_eq!(
                    failures[0].class,
                    if mode == 2 {
                        FailureClass::Unexpected
                    } else {
                        FailureClass::Rejected
                    }
                );
                assert_eq!(
                    failures[0].error.to_json(),
                    if mode == 2 {
                        uncertain().to_json()
                    } else {
                        rejected().to_json()
                    }
                );
            }
            http.await.unwrap();
        }
    }
}

#[tokio::test]
async fn unexpected_and_legacy_observed_errors_stop_without_starting_the_next_command() {
    for legacy in [false, true] {
        let cursor = Cursor::new().await;
        let (base, _requests, http) = routed_mock(vec![
            ("GET /me".into(), vec![response(200, profile())]),
            (
                "GET /events".into(),
                vec![response(
                    200,
                    json!({"events":[event(2,"bad"),event(3,"next")],"last":3}),
                )],
            ),
        ])
        .await;
        let later = Arc::new(AtomicUsize::new(0));
        let l = later.clone();
        let observed = Arc::new(AtomicUsize::new(0));
        let o = observed.clone();
        let mut builder = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.execution.concurrency = 1)
            .command("bad", "Unexpected", |_| async { Err(uncertain()) })
            .command("next", "Must not start", move |_| {
                let l = l.clone();
                async move {
                    l.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            });
        if legacy {
            builder = builder.on_command_failure(move |_, error| {
                let o = o.clone();
                async move {
                    assert_eq!(error.error.to_json(), uncertain().to_json());
                    o.fetch_add(1, Ordering::SeqCst);
                    Ok(FailureDisposition::Default)
                }
            });
        }
        let failure = report(
            run(builder.build().expect("valid bot configuration"))
                .await
                .unwrap_err(),
        );
        assert_eq!(
            failure.failed_events[0].error.to_json(),
            uncertain().to_json()
        );
        assert_eq!(failure.unprocessed_events, vec![3]);
        assert_eq!(failure.committed_cursor, 1);
        assert_eq!(cursor.last().await, 1);
        assert_eq!(observed.load(Ordering::SeqCst), usize::from(legacy));
        assert_eq!(later.load(Ordering::SeqCst), 0);
        http.await.unwrap();
    }
}

#[tokio::test]
async fn handled_prefix_commits_before_uncertain_failure_and_restarts_after_that_prefix_only() {
    let cursor = Cursor::new().await;
    let path = format!("POST /channels/{}/messages", uuid::Uuid::nil());
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"reject"),event(3,"fail"),event(4,"later")],"last":9}),
            )],
        ),
        (path, vec![response(200, message())]),
    ])
    .await;
    let failure = report(
        run(Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.execution.concurrency = 1)
            .command("reject", "Reject", |_| async { Err(rejected()) })
            .command("fail", "Uncertain", |_| async { Err(uncertain()) })
            .command("later", "Must not run", |_| async {
                panic!("unstarted handler executed")
            })
            .build()
            .expect("valid bot configuration"))
        .await
        .unwrap_err(),
    );
    assert_eq!(cursor.last().await, 2);
    assert_eq!(failure.committed_cursor, 2);
    assert_eq!(failure.failed_events[0].event_id, 3);
    assert_eq!(failure.unprocessed_events, vec![4]);
    assert_eq!(
        failure.failed_events[0].error.to_json()["retry_safety"],
        "do_not_replay_workflow"
    );
    http.await.unwrap();
    // Explicit restart is simulated only with read-only consumption: the SDK
    // never automatically invokes the uncertain handler again.
    let (base, mut requests, http) = routed_mock(vec![(
        "GET /events".into(),
        vec![response(200, json!({"events":[event(3,"fail")],"last":3}))],
    )])
    .await;
    EventConsumer::new(Client::builder("fixture").base_url(base).build())
        .cursor_file(&cursor.0)
        .poll(0)
        .await
        .unwrap();
    assert!(requests.recv().await.unwrap().contains("after=2"));
    http.await.unwrap();
}

#[tokio::test]
async fn failed_default_and_custom_error_replies_preserve_both_causes_and_do_not_commit() {
    for is_button in [false, true] {
        for custom in [false, true] {
            let cursor = Cursor::new().await;
            let route = if is_button {
                format!(
                    "POST /interactions/{}/interaction_2/callback",
                    uuid::Uuid::from_u128(2)
                )
            } else {
                format!("POST /channels/{}/messages", uuid::Uuid::nil())
            };
            let bad = if is_button {
                button(2, "bad:")
            } else {
                event(2, "bad")
            };
            let (base, _requests, http) = routed_mock(vec![
                ("GET /me".into(), vec![response(200, profile())]),
                (
                    "GET /events".into(),
                    vec![response(200, json!({"events":[bad],"last":2}))],
                ),
                (
                    route,
                    vec![response(503, json!({"error":"reply outcome unknown"}))],
                ),
            ])
            .await;
            let mut builder = Bot::builder("fixture")
                .base_url(base)
                .sync_commands(false)
                .log(false)
                .configure(|settings| {
                    settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
                })
                .command("bad", "Reject", |_| async { Err(rejected()) })
                .button("bad:", |_| async { Err(rejected()) });
            if custom {
                builder = builder
                    .on_command_failure(|ctx, _| async move {
                        ctx.reply("Reject").await?;
                        Ok(FailureDisposition::Handled)
                    })
                    .on_button_failure(|ctx, _| async move {
                        ctx.ephemeral("Reject").await?;
                        Ok(FailureDisposition::Handled)
                    });
            }
            let failure = report(
                run(builder.build().expect("valid bot configuration"))
                    .await
                    .unwrap_err(),
            );
            let error = failure.failed_events[0].error.to_json();
            assert_eq!(error["kind"], "failure_handling");
            assert_eq!(error["source"], rejected().to_json());
            assert_eq!(error["handling_error"]["status"], 503);
            assert_eq!(error["handling_error"]["outcome"], "unknown");
            assert_eq!(cursor.last().await, 1);
            http.await.unwrap();
        }
    }
}

#[tokio::test]
async fn handled_button_failure_still_finishes_other_matching_handlers() {
    let (base, _requests, http) = routed_mock(vec![]).await;
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .button("bad:", |_| async { Err(rejected()) })
        .button("bad:", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .on_button_failure(|_, _| async { Ok(FailureDisposition::Handled) })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    bot.process_event(serde_json::from_value(button(2, "bad:")).unwrap())
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    http.abort();
}

#[test]
fn rejection_classification_survives_structured_roundtrips_and_is_conservative_for_legacy_errors() {
    for error in [
        rejected(),
        CommandError::Denied("role required".into()).into(),
        CommandError::Invalid {
            name: "target".into(),
            reason: "clarification required".into(),
        }
        .into(),
    ] {
        assert!(error.is_command_rejection());
        assert!(Error::Remote(error.to_remote()).is_command_rejection());
    }
    assert!(
        !Error::from(CommandError::Definition("invalid registration".into()))
            .is_command_rejection()
    );
    let legacy:RemoteError=serde_json::from_value(json!({"kind":"command","message":"unclassified legacy command error","status":null,"retry_after":null})).unwrap();
    assert!(!Error::Remote(legacy).is_command_rejection());
}

async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn concurrent_success_after_failure_is_drained_but_cannot_cross_the_cursor_barrier() {
    let cursor = Cursor::new().await;
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"first"),event(3,"fail"),event(4,"late")],"last":9}),
            )],
        ),
    ])
    .await;
    let all = Arc::new(tokio::sync::Barrier::new(3));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let a = all.clone();
    let b = all.clone();
    let c = all.clone();
    let r = release.clone();
    let r2 = release.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.execution.concurrency = 3)
        .command("first", "Slow earlier success", move |_| {
            let a = a.clone();
            let r = r.clone();
            async move {
                a.wait().await;
                r.acquire().await.unwrap().forget();
                Ok(())
            }
        })
        .command("fail", "Failure after all started", move |_| {
            let b = b.clone();
            let r2 = r2.clone();
            async move {
                b.wait().await;
                r2.add_permits(2);
                Err(uncertain())
            }
        })
        .command("late", "Later completed work", move |_| {
            let c = c.clone();
            async move {
                c.wait().await;
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let failure = report(run(bot).await.unwrap_err());
    assert_eq!(cursor.last().await, 2);
    assert_eq!(failure.committed_cursor, 2);
    assert_eq!(failure.completed_uncommitted_events, vec![4]);
    assert_eq!(failure.failed_events[0].event_id, 3);
    assert!(failure.unprocessed_events.is_empty());
    http.await.unwrap();
}

#[tokio::test]
async fn errors_from_concurrent_batches_keep_all_causes_and_never_commit_past_earlier_failure() {
    let cursor = Cursor::new().await;
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![
                response(200, json!({"events":[event(2,"early")],"last":2})),
                response(200, json!({"events":[event(8,"late")],"last":8})),
            ],
        ),
    ])
    .await;
    let gate = Arc::new(tokio::sync::Barrier::new(2));
    let a = gate.clone();
    let b = gate.clone();
    let failure = report(
        run(Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.execution.concurrency = 2)
            .configure(|settings| settings.execution.pending_batches = 2)
            .command("early", "Earlier failure", move |_| {
                let a = a.clone();
                async move {
                    a.wait().await;
                    Err(uncertain())
                }
            })
            .command("late", "Later failure", move |_| {
                let b = b.clone();
                async move {
                    b.wait().await;
                    Err(Error::Api {
                        status: 429,
                        message: "advised".into(),
                        retry_after: Some(0.125),
                    })
                }
            })
            .build()
            .expect("valid bot configuration"))
        .await
        .unwrap_err(),
    );
    assert_eq!(cursor.last().await, 1);
    assert_eq!(
        failure
            .failed_events
            .iter()
            .map(|e| e.event_id)
            .collect::<Vec<_>>(),
        vec![2, 8]
    );
    assert_eq!(failure.failed_events[1].error.retry_after, Some(0.125));
    assert_eq!(
        failure.failed_events[0].error.to_json()["completed_steps"],
        json!([{"receipt":"confirmed-first-step"}])
    );
    http.await.unwrap();
}

#[tokio::test]
async fn graceful_stop_drains_accepted_work_then_commits() {
    let cursor = Cursor::new().await;
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"hold"),event(3,"hold")],"last":3}),
            )],
        ),
    ])
    .await;
    let started = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let s = started.clone();
    let d = done.clone();
    let r = release.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.execution.concurrency = 2)
        .command("hold", "Drain", move |_| {
            let s = s.clone();
            let d = d.clone();
            let r = r.clone();
            async move {
                s.fetch_add(1, Ordering::SeqCst);
                r.acquire().await.unwrap().forget();
                d.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let running = tokio::spawn(bot.run());
    until(|| started.load(Ordering::SeqCst) == 2).await;
    handle.stop();
    assert_eq!(cursor.last().await, 1);
    release.add_permits(2);
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(done.load(Ordering::SeqCst), 2);
    assert_eq!(cursor.last().await, 3);
    http.await.unwrap();
}

#[tokio::test]
async fn cancellation_never_commits_an_unfinished_handler_or_automatically_runs_it_again() {
    let cursor = Cursor::new().await;
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(200, json!({"events":[event(2,"hold")],"last":2}))],
        ),
    ])
    .await;
    let writes = Arc::new(AtomicUsize::new(0));
    let w = writes.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .command("hold", "Uncertain external side effect", move |_| {
            let w = w.clone();
            async move {
                w.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<Result<()>>().await
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let running = tokio::spawn(bot.run());
    until(|| writes.load(Ordering::SeqCst) == 1).await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert_eq!(cursor.last().await, 1);
    assert!(handle.transport_status().active.is_none());
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(handle
        .client()
        .events()
        .collect::<Event>()
        .subscribe()
        .is_err());
    http.await.unwrap();
}

#[tokio::test]
async fn cursor_persistence_failure_reports_completed_but_uncommitted_work() {
    let cursor = Cursor::new().await;
    let saved = cursor.0.with_extension("saved");
    let path = cursor.0.clone();
    let backup = saved.clone();
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(200, json!({"events":[event(2,"done")],"last":2}))],
        ),
    ])
    .await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let a = attempts.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .command("done", "Complete but fail storage", move |_| {
            let path = path.clone();
            let backup = backup.clone();
            let a = a.clone();
            async move {
                a.fetch_add(1, Ordering::SeqCst);
                tokio::fs::rename(&path, &backup).await?;
                tokio::fs::create_dir(&path).await?;
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    let handle = bot.handle();
    let failure = report(run(bot).await.unwrap_err());
    assert!(failure.cursor_error.is_some());
    assert!(failure.failed_events.is_empty());
    assert_eq!(failure.completed_uncommitted_events, vec![2]);
    assert_eq!(handle.transport_status().committed_cursor, 1);
    assert_eq!(failure.committed_cursor, 1);
    assert_eq!(EventCursor::new(saved).load().await.unwrap(), Some(1));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    http.await.unwrap();
}

#[tokio::test]
async fn intake_auth_permanent_errors_and_corrupt_cursor_do_not_enter_handler_failure_callbacks() {
    for status in [401, 403, 404] {
        let cursor = Cursor::new().await;
        let (base, _requests, http) = routed_mock(vec![
            ("GET /me".into(), vec![response(200, profile())]),
            (
                "GET /events".into(),
                vec![response(status, json!({"error":"intake failed"}))],
            ),
        ])
        .await;
        let failures = Arc::new(AtomicUsize::new(0));
        let f = failures.clone();
        let error = run(Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .on_command_failure(move |_, _| {
                let f = f.clone();
                async move {
                    f.fetch_add(1, Ordering::SeqCst);
                    Ok(FailureDisposition::Handled)
                }
            })
            .build()
            .expect("valid bot configuration"))
        .await
        .unwrap_err();
        assert_eq!(error.status_code(), Some(status));
        let failure = report(error);
        assert_eq!(failure.intake_error.unwrap().status, Some(status));
        assert_eq!(failures.load(Ordering::SeqCst), 0);
        assert_eq!(cursor.last().await, 1);
        http.await.unwrap();
    }
    let cursor = Cursor::new().await;
    tokio::fs::write(&cursor.0, "corrupt").await.unwrap();
    let (base, _requests, http) =
        routed_mock(vec![("GET /me".into(), vec![response(200, profile())])]).await;
    assert!(matches!(
        run(Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(
                |settings| settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            )
            .build()
            .expect("valid bot configuration"))
        .await
        .unwrap_err(),
        Error::Config(_)
    ));
    http.await.unwrap();
}

#[tokio::test]
async fn long_failed_rejection_reply_keeps_confirmed_reply_receipts() {
    let cursor = Cursor::new().await;
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"reject")],"last":2}),
            )],
        ),
        (
            format!("POST /channels/{}/messages", uuid::Uuid::nil()),
            vec![
                response(200, message()),
                response(502, json!({"error":"second reply uncertain"})),
            ],
        ),
    ])
    .await;
    let failure = report(
        run(Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .command("reject", "Long", |_| async {
                Err(CommandError::Denied("x".repeat(2500)).into())
            })
            .build()
            .expect("valid bot configuration"))
        .await
        .unwrap_err(),
    );
    let handling = failure.failed_events[0].error.to_json()["handling_error"].clone();
    assert_eq!(handling["kind"], "partial_write");
    assert_eq!(handling["completed_steps"].as_array().unwrap().len(), 1);
    assert_eq!(handling["failed_step"], 2);
    assert_eq!(cursor.last().await, 1);
    http.await.unwrap();
}

#[tokio::test]
async fn callback_keeps_structured_authorization_checks_while_scrubbing_credentials() {
    let seen = Arc::new(Mutex::new(None));
    let s = seen.clone();
    let bot = Bot::builder("yb_private_failure_test")
        .sync_commands(false)
        .log(false)
        .command("bad", "Auth report", |_| async {
            Err(Error::OperationAuthorization {
                report: Box::new(rappin::capabilities::AuthorizationReport {
                    operation: "edit_channel".into(),
                    server_id: uuid::Uuid::nil(),
                    requester_id: uuid::Uuid::nil(),
                    bot_id: None,
                    checks: vec![rappin::capabilities::AuthorizationCheck {
                        check: "scope".into(),
                        principal: rappin::capabilities::Principal::Requester,
                        decision: rappin::capabilities::AccessDecision::Unknown,
                        reason: "yb_private_failure_test".into(),
                        error: None,
                    }],
                }),
            })
        })
        .on_command_failure(move |_, failure| {
            let s = s.clone();
            async move {
                *s.lock().unwrap() = Some(failure.error.to_json());
                Ok(FailureDisposition::Stop)
            }
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    assert!(bot
        .process_event(serde_json::from_value(event(2, "bad")).unwrap())
        .await
        .is_err());
    let value = seen.lock().unwrap().clone().unwrap();
    assert!(value["authorization"]["checks"].is_array());
    assert!(!value.to_string().contains("yb_private_failure_test"));
}

#[cfg(feature = "gateway")]
#[path = "support/failure_gateway.rs"]
mod gateway;
#[cfg(feature = "hot-reload")]
#[path = "support/failure_worker.rs"]
mod worker;

#[tokio::test]
async fn callback_failure_retains_original_partial_receipts_and_independent_retry_advice() {
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("bad", "Partial", |_| async { Err(uncertain()) })
        .on_command_failure(|_, _| async {
            Err(Error::Api {
                status: 429,
                message: "callback failed".into(),
                retry_after: Some(0.125),
            })
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    let error = bot
        .process_event(serde_json::from_value(event(2, "bad")).unwrap())
        .await
        .unwrap_err();
    let value = error.to_json();
    assert_eq!(value["kind"], "failure_handling");
    assert_eq!(value["source"], uncertain().to_json());
    assert_eq!(value["handling_error"]["status"], 429);
    assert_eq!(value["handling_error"]["retry_after"], 0.125);
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(error.retry_after(), Some(Duration::from_secs_f64(0.125)));
}
#[tokio::test]
async fn panic_and_panicking_failure_callback_are_uncertain_and_never_completed_by_default() {
    for callback_panics in [false, true] {
        let cursor = Cursor::new().await;
        let (base, _requests, http) = routed_mock(vec![
            ("GET /me".into(), vec![response(200, profile())]),
            (
                "GET /events".into(),
                vec![response(200, json!({"events":[event(2,"bad")],"last":2}))],
            ),
        ])
        .await;
        let mut builder = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .command("bad", "Panic", move |_| async move {
                if callback_panics {
                    Err(uncertain())
                } else {
                    panic!("fixture panic after possible effects")
                }
            });
        if callback_panics {
            builder = builder.on_command_failure(|_, _| async { panic!("failure callback panic") });
        }
        let failure = report(
            run(builder.build().expect("valid bot configuration"))
                .await
                .unwrap_err(),
        );
        let error = failure.failed_events[0].error.to_json();
        if callback_panics {
            assert_eq!(error["source"], uncertain().to_json());
            assert_eq!(error["handling_error"]["kind"], "handler_interrupted");
        } else {
            assert_eq!(error["kind"], "handler_interrupted");
            assert_eq!(error["outcome"], "unknown");
        }
        assert_eq!(cursor.last().await, 1);
        http.await.unwrap();
    }
}
#[tokio::test]
async fn cooldown_rejection_never_constructs_the_user_handler_and_reply_failure_is_not_completed() {
    for cog in [false, true] {
        let cursor = Cursor::new().await;
        let (base, _requests, http) = routed_mock(vec![
            ("GET /me".into(), vec![response(200, profile())]),
            (
                "GET /events".into(),
                vec![response(
                    200,
                    json!({"events":[event(2,"cool"),event(3,"cool")],"last":3}),
                )],
            ),
            (
                format!("POST /channels/{}/messages", uuid::Uuid::nil()),
                vec![response(503, json!({"error":"reply uncertain"}))],
            ),
        ])
        .await;
        let constructed = Arc::new(AtomicUsize::new(0));
        let c = constructed.clone();
        let cd = Cooldown::user(Duration::from_secs(100));
        let mut builder = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
            })
            .configure(|settings| settings.execution.concurrency = 1);
        if cog {
            builder = builder.cog(
                CogBuilder::new("cooldown")
                    .command_with_cooldown("cool", "Cooldown", cd, move |_| {
                        c.fetch_add(1, Ordering::SeqCst);
                        async { Ok(()) }
                    })
                    .build(),
            );
        } else {
            builder = builder.command_with_cooldown("cool", "Cooldown", cd, move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            });
        }
        let failure = report(
            run(builder.build().expect("valid bot configuration"))
                .await
                .unwrap_err(),
        );
        assert_eq!(cursor.last().await, 2);
        assert_eq!(constructed.load(Ordering::SeqCst), 1);
        let error = failure.failed_events[0].error.to_json();
        assert_eq!(error["source"]["command_kind"], "denied");
        assert_eq!(error["handling_error"]["status"], 503);
        http.await.unwrap();
    }
}

#[tokio::test]
async fn handled_rejection_in_a_later_batch_waits_for_earlier_work_before_committing() {
    let cursor = Cursor::new().await;
    let path = cursor.0.clone();
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![
                response(200, json!({"events":[event(2,"slow")],"last":2})),
                response(
                    200,
                    json!({"events":[event(8,"reject"),event(9,"next")],"last":9}),
                ),
            ],
        ),
    ])
    .await;
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let r = release.clone();
    let r2 = release.clone();
    let handled = Arc::new(AtomicUsize::new(0));
    let h = handled.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .configure(|settings| settings.execution.concurrency = 2)
        .configure(|settings| settings.execution.pending_batches = 2)
        .command("slow", "Earlier work", move |_| {
            let r = r.clone();
            async move {
                r.acquire().await.unwrap().forget();
                Ok(())
            }
        })
        .command("reject", "Expected", |_| async { Err(rejected()) })
        .on_command_failure(move |_, failure| {
            let h = h.clone();
            async move {
                assert_eq!(failure.class, FailureClass::Rejected);
                h.fetch_add(1, Ordering::SeqCst);
                Ok(FailureDisposition::Handled)
            }
        })
        .command("next", "Continue before earlier finishes", move |ctx| {
            let r2 = r2.clone();
            let path = path.clone();
            async move {
                assert_eq!(EventCursor::new(path).load().await?, Some(1));
                r2.add_permits(1);
                ctx.bot().unwrap().stop();
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    run(bot).await.unwrap();
    assert_eq!(cursor.last().await, 9);
    assert_eq!(handled.load(Ordering::SeqCst), 1);
    http.await.unwrap();
}
#[tokio::test]
async fn definition_errors_are_not_user_rejections_and_default_policy_never_replies_to_them() {
    let bot = Bot::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .sync_commands(false)
        .log(false)
        .command("bad", "Definition", |_| async {
            Err(CommandError::Definition("invalid handler schema".into()).into())
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    let error = bot
        .process_event(serde_json::from_value(event(2, "bad")).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Command(CommandError::Definition(_))));
}

#[tokio::test]
async fn preparation_failure_reports_every_undispatched_event_and_received_watermark() {
    let cursor = Cursor::new().await;
    let expiry = chrono::Utc::now() + chrono::Duration::minutes(15);
    let mut bad = button(3, "capacity:");
    bad["data"]["expires_at"] = json!(expiry);
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"never"),bad],"last":3}),
            )],
        ),
    ])
    .await;
    let called = Arc::new(AtomicUsize::new(0));
    let c = called.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .configure(|settings| {
            settings.persistence.cursor = Some(rappin::EventCursor::new(&cursor.0))
        })
        .command("never", "Not prepared", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    for id in 10000..14096 {
        let mut raw = button(id, "existing:")["data"].clone();
        raw["expires_at"] = json!(expiry);
        bot.client()
            .track_interaction(&serde_json::from_value(raw).unwrap())
            .await
            .unwrap();
    }
    let failure = report(run(bot).await.unwrap_err());
    assert_eq!(failure.received_cursor, 3);
    assert_eq!(failure.committed_cursor, 1);
    assert_eq!(failure.unprocessed_events, vec![2, 3]);
    assert!(failure.intake_error.is_some());
    assert!(failure.failed_events.is_empty());
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert_eq!(cursor.last().await, 1);
    http.await.unwrap();
}

#[tokio::test]
async fn startup_auth_failure_closes_waiters_without_entering_completion_callbacks() {
    let (base, _requests, http) = routed_mock(vec![(
        "GET /me".into(),
        vec![response(401, json!({"error":"invalid credential"}))],
    )])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build()
        .expect("valid bot configuration");
    let waiter = bot
        .client()
        .events()
        .collect::<Event>()
        .subscribe()
        .unwrap();
    let handle = bot.handle();
    assert!(run(bot).await.unwrap_err().is_unauthorized());
    assert!(matches!(waiter.wait().await, Err(WaitError::Closed)));
    assert!(handle.transport_status().active.is_none());
    http.await.unwrap();
}
