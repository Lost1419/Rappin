mod support;
use rappin::*;
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use support::{mock, response};
use uuid::Uuid;

fn policy(attempts: u32) -> AvailabilityPolicy {
    AvailabilityPolicy::new(attempts, Duration::from_secs(3))
        .unwrap()
        .backoff(Duration::from_millis(2), Duration::from_millis(8))
        .unwrap()
        .jitter_percent(0)
        .unwrap()
}
fn advised() -> support::Response {
    (
        503,
        vec![("Retry-After", "0.001")],
        json!({"error":"busy"}).to_string(),
    )
}
async fn request(client: &Client, method: Method) -> Result<Value> {
    client
        .request_json::<Value, ()>(method, "/fixture", None)
        .await
}

#[tokio::test]
async fn independent_budgets_preserve_read_advice_without_replaying_writes() {
    let (base, mut requests, http) =
        mock(vec![advised(), response(200, json!({})), advised()]).await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .max_write_retries(0)
        .max_read_retries(1)
        .build();
    request(&client, Method::GET).await.unwrap();
    assert_eq!(
        request(&client, Method::POST)
            .await
            .unwrap_err()
            .status_code(),
        Some(503)
    );
    http.await.unwrap();
    assert!(requests.recv().await.unwrap().starts_with("GET "));
    assert!(requests.recv().await.unwrap().starts_with("GET "));
    assert!(requests.recv().await.unwrap().starts_with("POST "));
    assert_eq!(client.request_metrics().attempts, 3);
}

#[tokio::test]
async fn legacy_retry_setter_resets_both_budgets_and_last_setting_wins() {
    let (base, _requests, http) = mock(vec![advised(), advised()]).await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .max_read_retries(10)
        .max_write_retries(10)
        .max_retries(0)
        .build();
    assert!(request(&client, Method::GET).await.is_err());
    assert!(request(&client, Method::POST).await.is_err());
    http.await.unwrap();
    assert_eq!(client.request_metrics().attempts, 2);
}

#[tokio::test]
async fn read_episode_owns_attempts_and_observations_and_never_recovers_a_write() {
    let (base, _requests, http) = mock(vec![
        advised(),
        response(522, json!({"error":"edge"})),
        response(200, json!({"ok":true})),
        response(502, json!({"error":"uncertain"})),
    ])
    .await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .max_retries(100)
        .read_availability_policy(policy(3))
        .build();
    assert_eq!(request(&client, Method::GET).await.unwrap()["ok"], true);
    let metrics = client.request_metrics();
    assert_eq!(metrics.started, 1);
    assert_eq!(metrics.attempts, 3);
    assert_eq!(metrics.retries, 2);
    assert_eq!(client.last_request().unwrap().attempts, 3);
    let status = client.read_availability_status();
    assert_eq!(status.phase, Some(AvailabilityPhase::Request));
    assert_eq!(status.recoveries, 1);
    assert_eq!(status.total_attempts, 3);
    let error = request(&client, Method::POST).await.unwrap_err();
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(client.read_availability_status().episodes, 1);
    assert_eq!(client.request_metrics().attempts, 4);
    http.await.unwrap();
}

#[tokio::test]
async fn bounded_recovery_retains_structured_cause_and_once_bypasses_policy() {
    let (base, _requests, http) = mock(vec![advised(), advised(), advised()]).await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .max_retries(100)
        .read_availability_policy(policy(2))
        .build();
    let error = request(&client, Method::GET).await.unwrap_err();
    let failure = error.availability_failure().unwrap();
    assert_eq!(failure.phase, AvailabilityPhase::Request);
    assert_eq!(failure.attempts, 2);
    assert_eq!(failure.last_error.as_ref().unwrap().status, Some(503));
    assert_eq!(error.retry_after(), Some(Duration::from_millis(1)));
    let error = client
        .request_once(Method::GET, "/fixture", None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(503));
    assert!(error.availability_failure().is_none());
    assert_eq!(client.read_availability_status().episodes, 1);
    assert_eq!(client.request_metrics().attempts, 3);
    http.await.unwrap();
}

#[tokio::test]
async fn terminal_auth_scope_input_and_json_errors_do_not_recover() {
    let (base, _requests, http) = mock(vec![
        response(401, json!({})),
        response(403, json!({"error":"denied"})),
        response(400, json!({"error":"invalid"})),
        (200, vec![], "invalid-json".into()),
    ])
    .await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .read_availability_policy(policy(5))
        .build();
    for status in [401, 403, 400] {
        assert_eq!(
            request(&client, Method::GET)
                .await
                .unwrap_err()
                .status_code(),
            Some(status)
        );
    }
    assert!(matches!(
        request(&client, Method::GET).await,
        Err(Error::Json(_))
    ));
    assert_eq!(client.request_metrics().attempts, 4);
    assert_eq!(client.read_availability_status().total_retries, 0);
    http.await.unwrap();
}

#[tokio::test]
async fn read_recovery_honors_the_distinct_server_wait_budget() {
    let (base, _requests, http) = mock(vec![(
        503,
        vec![("Retry-After", "1")],
        json!({"error":"busy"}).to_string(),
    )])
    .await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .max_retry_wait(Duration::from_millis(1))
        .read_availability_policy(policy(5))
        .build();
    let error = request(&client, Method::GET).await.unwrap_err();
    assert!(matches!(error, Error::RetryWaitExceeded { .. }));
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(client.request_metrics().attempts, 1);
    http.await.unwrap();
}

#[tokio::test]
async fn dropping_a_read_future_cancels_recovery_without_later_attempts() {
    let (base, _requests, http) = mock(vec![response(520, json!({"error":"edge"}))]).await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .read_availability_policy(
            AvailabilityPolicy::new(5, Duration::from_secs(30))
                .unwrap()
                .backoff(Duration::from_secs(10), Duration::from_secs(10))
                .unwrap()
                .jitter_percent(0)
                .unwrap(),
        )
        .build();
    let mut updates = client.subscribe_read_availability();
    let copy = client.clone();
    let task = tokio::spawn(async move { request(&copy, Method::GET).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while updates.borrow().state != AvailabilityState::Waiting {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(client.read_availability_status().cancellations, 1);
    assert_eq!(client.request_metrics().attempts, 1);
    http.await.unwrap();
}

fn message_event(id: u64, args: &str) -> Event {
    let mut value = support::event(id, "shared");
    value["data"]["args"] = json!(args);
    value["data"]["server_id"] = json!(Uuid::from_u128(3));
    value["data"]["author_id"] = json!(Uuid::from_u128(4));
    serde_json::from_value(value).unwrap()
}
fn interaction_event(id: u64, args: &str) -> Event {
    serde_json::from_value(json!({"id":id,"type":"command","data":{
        "id":Uuid::from_u128(id as u128),"token":"fixture_interaction_token",
        "command":"shared","args":args,"channel_id":Uuid::nil(),
        "server_id":Uuid::from_u128(3),"user_id":Uuid::from_u128(4),
        "expires_at":chrono::Utc::now()+chrono::Duration::minutes(15)
    }}))
    .unwrap()
}
fn message_receipt() -> Value {
    json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":chrono::Utc::now()}})
}
struct Count(BoundedU32<1, 10>);
impl FromArgs for Count {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self(args.required("count")?))
    }
}

#[tokio::test]
async fn shared_commands_preserve_scope_parsing_reply_ownership_and_generic_fallback() {
    let (base, mut requests, http) = mock(vec![
        response(200, message_receipt()),
        response(200, json!({})),
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .on_command_interaction(|_| async { panic!("generic handler duplicated shared command") })
        .command("shared", "Shared handler", move |ctx| {
            let c = c.clone();
            async move {
                assert_eq!(ctx.author_id(), Some(Uuid::from_u128(4)));
                assert_eq!(ctx.server_id(), Some(Uuid::from_u128(3)));
                assert_eq!(ctx.channel_id(), Some(Uuid::nil()));
                assert_eq!(ctx.parse::<Count>()?.0 .0, 7);
                assert!(ctx.message().is_some() || ctx.interaction().is_some());
                assert_eq!(ctx.operation_id(), None);
                c.fetch_add(1, Ordering::SeqCst);
                ctx.reply("shared reply").await
            }
        })
        .build()
        .unwrap();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let event = message_event(1, "7");
    let message = event.data["id"].clone();
    bot.process_event(event).await.unwrap();
    bot.process_event(interaction_event(2, "7")).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let first = requests.recv().await.unwrap();
    assert!(first.contains(&format!("\"reply_to\":{message}")));
    let second = requests.recv().await.unwrap();
    assert!(second.starts_with(&format!(
        "POST /interactions/{}/fixture_interaction_token/callback",
        Uuid::from_u128(2)
    )));
    assert!(second.contains("\"type\":\"reply\""));
    http.await.unwrap();
}

#[tokio::test]
async fn shared_gate_denial_executes_no_handler_and_interaction_rejection_replies_once() {
    let (base, mut requests, http) = mock(vec![response(200, json!({}))]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .message_gate(|_| async { Ok(MessageDisposition::Handled) })
        .command("shared", "Shared", move |ctx| {
            let c = c.clone();
            async move {
                ctx.parse::<Count>()?;
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .unwrap();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    bot.process_event(message_event(1, "7")).await.unwrap();
    bot.process_event(interaction_event(2, "101"))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"type\":\"ephemeral\""));
    http.await.unwrap();
}

#[tokio::test]
async fn shared_failed_replies_and_partial_uncertain_causes_remain_incomplete() {
    for is_interaction in [false, true] {
        let (base, _requests, http) =
            mock(vec![response(502, json!({"error":"uncertain reply"}))]).await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .configure(|settings| {
                settings.transport.read_availability = Some(policy(5));
                settings.transport.max_write_retries = 100;
            })
            .command(
                "shared",
                "Shared",
                |ctx| async move { ctx.reply("reply").await },
            )
            .build()
            .unwrap();
        bot.cache()
            .set_me(serde_json::from_value(support::profile()).unwrap())
            .await;
        let event = if is_interaction {
            interaction_event(2, "")
        } else {
            message_event(1, "")
        };
        assert_eq!(
            bot.process_event(event).await.unwrap_err().status_code(),
            Some(502)
        );
        assert_eq!(bot.client().request_metrics().attempts, 1);
        http.await.unwrap();
    }
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .command("shared", "Shared", |_| async {
            Err(Error::PartialWrite {
                completed: vec![json!({"receipt":"confirmed"})],
                failed_step: 2,
                source: Box::new(Error::Api {
                    status: 502,
                    message: "uncertain".into(),
                    retry_after: None,
                }),
            })
        })
        .build()
        .unwrap();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    for event in [message_event(1, ""), interaction_event(2, "")] {
        let error = bot.process_event(event).await.unwrap_err();
        assert!(matches!(error, Error::PartialWrite { failed_step: 2, .. }));
        assert_eq!(
            error.to_json()["completed_steps"][0]["receipt"],
            "confirmed"
        );
    }
}

#[tokio::test]
async fn shared_registration_conflicts_fail_before_io() {
    let result = Bot::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .command("shared", "Existing", |_| async { Ok(()) })
        .command("shared", "Duplicate", |_| async { Ok(()) })
        .build();
    assert!(matches!(
        result,
        Err(Error::Command(CommandError::Definition(_)))
    ));
}

#[tokio::test]
async fn concurrent_read_recovery_keeps_exact_totals_and_newest_episode_details() {
    let (base, _requests, http) = support::routed_mock(vec![
        (
            "GET /slow".into(),
            vec![
                response(520, json!({"error":"edge"})),
                response(200, json!({})),
            ],
        ),
        ("GET /fast".into(), vec![response(200, json!({}))]),
    ])
    .await;
    let client = Client::builder("fixture")
        .without_read_recovery()
        .base_url(base)
        .read_availability_policy(
            AvailabilityPolicy::new(2, Duration::from_secs(3))
                .unwrap()
                .backoff(Duration::from_millis(150), Duration::from_millis(150))
                .unwrap()
                .jitter_percent(0)
                .unwrap(),
        )
        .build();
    let mut updates = client.subscribe_read_availability();
    let copy = client.clone();
    let slow = tokio::spawn(async move { copy.request(Method::GET, "/slow", None).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while updates.borrow().state != AvailabilityState::Waiting {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    client.request(Method::GET, "/fast", None).await.unwrap();
    slow.await.unwrap().unwrap();
    let status = client.read_availability_status();
    assert_eq!(status.episodes, 2);
    assert_eq!(status.total_attempts, 3);
    assert_eq!(status.total_retries, 1);
    assert_eq!(status.recoveries, 1);
    assert_eq!(status.attempts, 1);
    assert_eq!(status.state, AvailabilityState::Recovered);
    assert!(status.last_error.is_none());
    assert_eq!(client.request_metrics().retries, 1);
    http.await.unwrap();
}

#[tokio::test]
async fn shared_failures_retain_checkpoint_and_supervision_never_reruns_handler() {
    let root = std::env::temp_dir().join(format!("rappin-shared-cursor-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let cursor = EventCursor::new(root.join("cursor.json"));
    cursor.save(1).await.unwrap();
    let (base, _requests, http) = support::routed_mock(vec![
        ("GET /me".into(), vec![response(200, support::profile())]),
        (
            "GET /events".into(),
            vec![response(
                200,
                json!({"events":[interaction_event(2,"")],"last":2}),
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
        .configure(|settings| {
            settings.persistence.cursor = Some(EventCursor::new(root.join("cursor.json")))
        })
        .command("shared", "Shared", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(Error::PartialWrite {
                    completed: vec![json!({"receipt":"confirmed"})],
                    failed_step: 2,
                    source: Box::new(Error::Api {
                        status: 502,
                        message: "uncertain".into(),
                        retry_after: None,
                    }),
                })
            }
        })
        .build()
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), bot.run())
        .await
        .unwrap()
        .unwrap_err();
    let Error::EventProcessing { report } = error else {
        panic!("missing checkpoint report")
    };
    assert_eq!(report.committed_cursor, 1);
    assert_eq!(report.failed_events[0].event_id, 2);
    assert_eq!(
        report.failed_events[0].error.details["completed_steps"][0]["receipt"],
        "confirmed"
    );
    assert_eq!(cursor.load().await.unwrap(), Some(1));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    http.await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
