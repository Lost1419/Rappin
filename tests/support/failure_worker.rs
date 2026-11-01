use super::*;
use rappin::hot_reload::{HotReload, WorkerProject};

struct Project(PathBuf);
impl Project {
    async fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("rappin-failure-worker-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(root.join("src")).await.unwrap();
        let manifest=format!("[package]\nname=\"failure-worker\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin-rs={{path={},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\nserde_json=\"1\"\n",json!(env!("CARGO_MANIFEST_DIR")));
        tokio::fs::write(root.join("Cargo.toml"), manifest)
            .await
            .unwrap();
        tokio::fs::write(root.join("src/main.rs"),r#"
use rappin::*;
use serde_json::json;
fn reject()->YappinError {CommandError::Missing("target (use an ID to clarify)".into()).into()}
fn uncertain()->YappinError {YappinError::PartialWrite{completed:vec![json!({"receipt":"confirmed-first-step"})],failed_step:2,source:Box::new(YappinError::Api{status:502,message:"uncertain mutation".into(),retry_after:None})}}
#[tokio::main]
async fn main()->Result<()> {
    WorkerCog::new("failure_worker")
        .command("reject","Reject",|_|async{Err(reject())})
        .command_typed::<(),_,_>("typed","Typed parse",|_,_|async{panic!("invalid typed invocation ran")})
        .dynamic_command(serde_json::from_value(json!({"name":"dynamic","description":"Dynamic parse","arguments":[{"name":"count","kind":{"type":"integer","min":1,"max":10},"required":true}]}))?,|_,_|async{panic!("invalid dynamic invocation ran")})?
        .command("uncertain","Partial failure",|_|async{Err(uncertain())})
        .command("advised","Retain advice",|_|async{Err(YappinError::Api{status:503,message:"advised".into(),retry_after:Some(0.125)})})
        .command("after","Still alive",|ctx|async move{ctx.reply("same worker continued").await})
        .command("slow","Interrupted",|ctx|async move{ctx.reply("confirmed before interruption").await?;tokio::time::sleep(std::time::Duration::from_secs(5)).await;Ok(())})
        .button("reject:",|_|async{Err(reject())}).button("uncertain:",|_|async{Err(uncertain())})
        .run().await
}
"#).await.unwrap();
        Self(root)
    }
    fn spec(&self) -> WorkerProject {
        WorkerProject::new(self.0.join("Cargo.toml"), "failure-worker").offline(true)
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_failures_preserve_class_causes_and_keep_the_worker_alive_when_handled() {
    let project = Project::new().await;
    for is_button in [false, true] {
        for mode in [0, 1, 2, 3, 4] {
            // expected default/callback, unexpected handled/unhandled
            let cursor = Cursor::new().await;
            let expected = mode < 2;
            let handled = mode < 3;
            let name = if expected {
                if is_button {
                    "reject"
                } else {
                    "typed"
                }
            } else {
                "uncertain"
            };
            let mut bad = if is_button {
                button(2, &format!("{name}:"))
            } else {
                event(2, name)
            };
            if expected && !is_button {
                bad["data"]["args"] = json!("extra");
            }
            let expected_error = if expected && !is_button {
                YappinError::from(CommandError::Extra).to_json()
            } else if expected {
                rejected().to_json()
            } else {
                uncertain().to_json()
            };
            let mut routes = vec![
                ("GET /me".into(), vec![response(200, profile())]),
                (
                    "GET /events".into(),
                    vec![response(
                        200,
                        json!({"events":[bad,event(3,"after"),event(4,"stop")],"last":4}),
                    )],
                ),
            ];
            let mut messages = vec![];
            if handled {
                if is_button {
                    routes.push((
                        format!(
                            "POST /interactions/{}/interaction_2/callback",
                            uuid::Uuid::from_u128(2)
                        ),
                        vec![response(200, json!({}))],
                    ));
                } else {
                    messages.push(response(200, message()));
                }
                messages.push(response(200, message()));
            }
            if !messages.is_empty() {
                routes.push((
                    format!("POST /channels/{}/messages", uuid::Uuid::nil()),
                    messages,
                ));
            }
            let (base, mut requests, http) = routed_mock(routes).await;
            let observed = Arc::new(Mutex::new(vec![]));
            let a = observed.clone();
            let b = observed.clone();
            let mut builder = Bot::builder("fixture")
                .base_url(base)
                .sync_commands(false)
                .log(false)
                .cursor_file(&cursor.0)
                .handler_concurrency(1)
                .command("stop", "Stop", |ctx| async move {
                    ctx.bot().unwrap().stop();
                    Ok(())
                });
            if mode != 0 && mode != 4 {
                builder = builder
                    .on_command_failure(move |ctx, failure| {
                        let a = a.clone();
                        async move {
                            a.lock().unwrap().push(failure);
                            if mode == 3 {
                                return Ok(FailureDisposition::Stop);
                            }
                            ctx.reply("handled").await?;
                            Ok(FailureDisposition::Handled)
                        }
                    })
                    .on_button_failure(move |ctx, failure| {
                        let b = b.clone();
                        async move {
                            b.lock().unwrap().push(failure);
                            if mode == 3 {
                                return Ok(FailureDisposition::Stop);
                            }
                            ctx.ephemeral("handled").await?;
                            Ok(FailureDisposition::Handled)
                        }
                    });
            }
            let legacy = Arc::new(Mutex::new(vec![]));
            if mode == 4 {
                let c = legacy.clone();
                let b = legacy.clone();
                builder = builder
                    .on_command_error(move |_, error| {
                        let c = c.clone();
                        async move {
                            c.lock().unwrap().push(error.to_json());
                        }
                    })
                    .on_button_error(move |_, error| {
                        let b = b.clone();
                        async move {
                            b.lock().unwrap().push(error.to_json());
                        }
                    });
            }
            let bot = builder.build();
            let handle = bot.handle();
            let reload = HotReload::load(handle.clone(), project.spec())
                .await
                .unwrap();
            let pid = reload.process_id().await;
            let result = run(bot).await;
            if handled {
                result.unwrap();
                assert_eq!(cursor.last().await, 4);
            } else {
                let failure = report(result.unwrap_err());
                assert_eq!(
                    failure.failed_events[0].error.to_json(),
                    uncertain().to_json()
                );
                assert_eq!(cursor.last().await, 1);
            }
            assert_eq!(reload.process_id().await, pid);
            assert_eq!(reload.generation(), 1);
            if mode != 0 && mode != 4 {
                let observed = observed.lock().unwrap();
                assert_eq!(
                    observed[0].class,
                    if expected {
                        FailureClass::Rejected
                    } else {
                        FailureClass::Unexpected
                    }
                );
                assert_eq!(observed[0].error.to_json(), expected_error);
            }
            if mode == 4 {
                assert_eq!(legacy.lock().unwrap().as_slice(), &[uncertain().to_json()]);
            }
            http.await.unwrap();
            let mut continued = 0;
            while let Ok(request) = requests.try_recv() {
                if request.contains("same worker continued") {
                    continued += 1;
                }
            }
            assert_eq!(continued, usize::from(handled));
            reload.unload().await.unwrap();
        }
    }
    let cursor = Cursor::new().await;
    let mut invalid = event(2, "dynamic");
    invalid["data"]["args"] = json!("invalid");
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[invalid,event(3,"stop")],"last":3}),
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
        .command("stop", "Stop", |ctx| async move {
            ctx.bot().unwrap().stop();
            Ok(())
        })
        .on_command_failure(|_, failure| async move {
            assert_eq!(failure.class, FailureClass::Rejected);
            assert_eq!(failure.error.to_json()["command_kind"], "invalid");
            Ok(FailureDisposition::Handled)
        })
        .build();
    let handle = bot.handle();
    let reload = HotReload::load(handle.clone(), project.spec())
        .await
        .unwrap();
    run(bot).await.unwrap();
    assert_eq!(cursor.last().await, 3);
    http.await.unwrap();
    reload.unload().await.unwrap();
    // Error-reply failure retains the worker cause and the native reply cause.
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
            vec![response(503, json!({"error":"reply uncertain"}))],
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&cursor.0)
        .on_command_failure(|ctx, _| async move {
            ctx.reply("rejected").await?;
            Ok(FailureDisposition::Handled)
        })
        .build();
    let handle = bot.handle();
    let reload = HotReload::load(handle.clone(), project.spec())
        .await
        .unwrap();
    let failure = report(run(bot).await.unwrap_err());
    let error = failure.failed_events[0].error.to_json();
    assert_eq!(error["source"], rejected().to_json());
    assert_eq!(error["handling_error"]["status"], 503);
    assert_eq!(cursor.last().await, 1);
    http.await.unwrap();
    reload.unload().await.unwrap();
    // Retry advice is preserved, but the handler is not scheduled for retry.
    let (base, _requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"advised")],"last":2}),
            )],
        ),
    ])
    .await;
    let cursor = Cursor::new().await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&cursor.0)
        .build();
    let handle = bot.handle();
    let reload = HotReload::load(handle.clone(), project.spec())
        .await
        .unwrap();
    let error = run(bot).await.unwrap_err();
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(error.retry_after(), Some(Duration::from_secs_f64(0.125)));
    assert_eq!(
        report(error).failed_events[0].error.to_json()["retry_safety"],
        "server_advised"
    );
    assert_eq!(cursor.last().await, 1);
    http.await.unwrap();
    reload.unload().await.unwrap();
    // Completion loss after a confirmed API request is explicitly uncertain.
    let cursor = Cursor::new().await;
    let (base, mut requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[event(2,"slow"),event(3,"after")],"last":3}),
            )],
        ),
        (
            format!("POST /channels/{}/messages", uuid::Uuid::nil()),
            vec![response(200, message())],
        ),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&cursor.0)
        .handler_concurrency(1)
        .build();
    let handle = bot.handle();
    let mut spec = project.spec();
    spec.handler_timeout = Duration::from_secs(1);
    let reload = HotReload::load(handle.clone(), spec).await.unwrap();
    let failure = report(run(bot).await.unwrap_err());
    assert_eq!(failure.failed_events[0].error.kind, "handler_interrupted");
    assert_eq!(
        failure.failed_events[0].error.to_json()["outcome"],
        "unknown"
    );
    assert_eq!(failure.unprocessed_events, vec![3]);
    assert_eq!(cursor.last().await, 1);
    http.await.unwrap();
    let mut writes = 0;
    while let Ok(request) = requests.try_recv() {
        if request.starts_with("POST ") {
            writes += 1;
        }
    }
    assert_eq!(writes, 1);
    reload.unload().await.unwrap();
}
