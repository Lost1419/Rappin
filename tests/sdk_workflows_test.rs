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
use support::*;
use uuid::Uuid;

fn schema() -> CommandSchema {
    serde_json::from_value(json!({"name":"feature run","description":"Run a configured feature","aliases":["frun"],"arguments":[
        {"name":"count","kind":{"type":"integer","min":1,"max":10}},
        {"name":"enabled","kind":{"type":"boolean"}},
        {"name":"target","kind":{"type":"user_id"}},
        {"name":"duration","kind":{"type":"duration","min_seconds":60,"max_seconds":3600}},
        {"name":"mode","kind":{"type":"choice","values":["fast","slow"]}},
        {"name":"label","kind":{"type":"string","min_length":1,"max_length":30},"rest":true,"required":false}
    ]})).unwrap()
}
fn snapshot() -> PermissionSnapshot {
    PermissionSnapshot {
        server:serde_json::from_value(json!({"id":Uuid::nil(),"name":"test","owner_id":Uuid::from_u128(9)})).unwrap(),
        actor:Uuid::nil(),member:Some(serde_json::from_value(json!({"id":Uuid::nil(),"username":"actor","roles":[Uuid::from_u128(1)]})).unwrap()),
        roles:serde_json::from_value(json!([
            {"id":Uuid::from_u128(1),"name":"assigned","perms":{"kick_members":true},"permissions":{"manage_messages":false},"manage_messages":true},
            {"id":Uuid::from_u128(2),"name":"everyone","auto":"everyone","permissions":{"view_channel":true}},
            {"id":Uuid::from_u128(3),"name":"unassigned","permissions":{"administrator":true}}
        ])).unwrap(),
    }
}

#[test]
fn permissions_are_shared_fail_closed_and_support_owner_admin_and_everyone() {
    let mut s = snapshot();
    let result = s.evaluate().unwrap();
    assert!(result.allows(&Permission::kick_members()));
    assert!(result.allows(&Permission::new("view_channel")));
    assert!(!result.allows(&Permission::manage_messages()));
    assert!(!result.administrator);
    s.member
        .as_mut()
        .unwrap()
        .member
        .roles
        .push(Uuid::from_u128(3));
    let result = s.evaluate().unwrap();
    assert!(result.administrator);
    assert!(result.allows(&Permission::new("future_permission")));
    s.member
        .as_mut()
        .unwrap()
        .member
        .roles
        .push(Uuid::from_u128(99));
    assert!(s.evaluate().is_err());
    s.actor = s.server.owner_id;
    s.member = None;
    s.roles.clear();
    assert!(s.evaluate().unwrap().allows(&Permission::new("anything")));
    s.actor = Uuid::nil();
    assert!(s.evaluate().is_err());
    s = snapshot();
    s.member.as_mut().unwrap().member.user.id = Uuid::new_v4();
    assert!(s.evaluate().is_err());
    s = snapshot();
    s.roles.push(s.roles[0].clone());
    assert!(s.evaluate().is_err());
}

async fn shared_call(api: &(impl Api + ?Sized)) -> Result<Value> {
    api.api_json::<_, ()>(Method::GET, "/fixture", None).await
}
#[tokio::test]
async fn common_api_works_with_clients_contexts_trait_objects_and_native_errors() {
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"value":1})),
        response(200, json!({"value":2})),
        response(403, json!({"error":"denied"})),
    ])
    .await;
    let client = Client::builder("fixture-secret").base_url(base).build();
    assert_eq!(shared_call(&client).await.unwrap()["value"], 1);
    let event = serde_json::from_value(
        json!({"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil()}),
    )
    .unwrap();
    let context = MessageContext::new(Arc::new(client), event, None, String::new(), None);
    let object: &dyn Api = &context;
    assert_eq!(shared_call(object).await.unwrap()["value"], 2);
    assert!(shared_call(object).await.unwrap_err().is_forbidden());
    for _ in 0..3 {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
}

#[test]
fn remote_errors_retain_status_retry_outcome_and_partial_receipts() {
    let error = YappinError::PartialWrite {
        completed: vec![json!({"id":Uuid::nil()})],
        failed_step: 2,
        source: Box::new(YappinError::Api {
            status: 429,
            message: "wait".into(),
            retry_after: Some(0.5),
        }),
    };
    let wire = serde_json::to_string(&error.to_remote()).unwrap();
    let remote = YappinError::Remote(serde_json::from_str(&wire).unwrap());
    assert_eq!(remote.status_code(), Some(429));
    assert!(remote.is_rate_limit());
    assert_eq!(remote.retry_after(), Some(Duration::from_millis(500)));
    assert_eq!(remote.to_json(), error.to_json());
    let mut conflicting = error.to_remote();
    conflicting
        .details
        .insert("status".into(), json!("invalid"));
    conflicting.details.insert("kind".into(), json!(false));
    let round_trip: RemoteError =
        serde_json::from_str(&serde_json::to_string(&conflicting).unwrap()).unwrap();
    assert_eq!(round_trip.status, Some(429));
    assert_eq!(round_trip.kind, "partial_write");
    assert_eq!(
        remote.to_json()["completed_steps"][0]["id"],
        Uuid::nil().to_string()
    );
    assert!(
        YappinError::Remote(YappinError::Command(CommandError::Extra).to_remote())
            .is_command_error()
    );
}

#[test]
fn runtime_schema_parses_quotes_types_bounds_and_rejects_bad_definitions() {
    let s = schema();
    let args = s
        .parse(&format!(
            "4 false <@{}> 2m fast 'two words' more",
            Uuid::nil()
        ))
        .unwrap();
    assert_eq!(args["count"], 4);
    assert_eq!(args["enabled"], false);
    assert_eq!(args["duration"], 120);
    assert_eq!(args["label"], "two words more");
    for bad in [
        "11 true x 2m fast",
        "1 maybe x 2m fast",
        "1 true x 2m fast",
        "1 true 00000000-0000-0000-0000-000000000000 2s fast",
        "1 true 00000000-0000-0000-0000-000000000000 2m unknown",
        "'broken",
    ] {
        assert!(s.parse(bad).is_err(), "{bad}");
    }
    let mut bad = s.clone();
    bad.arguments[0].required = false;
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.arguments[0].rest = true;
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.arguments[0].kind = ArgumentType::Integer {
        min: Some(5),
        max: Some(1),
    };
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.arguments[1].name = bad.arguments[0].name.clone();
    assert!(bad.validate().is_err());
    let number:CommandSchema=serde_json::from_value(json!({"name":"number","description":"Test","arguments":[{"name":"n","kind":{"type":"number","min":0.0,"max":10.0}}]})).unwrap();
    assert!(number.parse("NaN").is_err());
    assert!(number.parse("inf").is_err());
    assert!(number.parse("2.5").is_ok());
}

#[tokio::test]
async fn dynamic_commands_preflight_conflicts_before_actions_and_replace_transactionally() {
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let mut s = schema();
    s.arguments.clear();
    let command = DynamicCommandSpec::new(s.clone())
        .bind(move |_, _| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .unwrap();
    let bot = Bot::builder("fixture-secret")
        .log(false)
        .sync_commands(false)
        .prefix("?")
        .registered_command(command)
        .build();
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    let plan = bot
        .handle()
        .preflight_commands(
            &[CommandInfo {
                name: "new".into(),
                description: "New".into(),
                aliases: vec![],
                arguments: vec![],
            }],
            Some("dynamic"),
        )
        .await
        .unwrap();
    assert!(plan.routes.contains(&"feature run".into()));
    assert!(plan.routes.contains(&"new".into()));
    assert!(bot
        .handle()
        .preflight_commands(&[s.info()], None)
        .await
        .is_err());
    let mut event:Event=serde_json::from_value(json!({"type":"message","data":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"content":"?frun"}})).unwrap();
    bot.process_event(event.clone()).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    struct Collision(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl Cog for Collision {
        fn name(&self) -> &str {
            "collision"
        }
        fn commands(&self) -> Vec<CogCommand> {
            vec![CogCommand::new("frun", "Conflict", |_| async { Ok(()) })]
        }
        async fn on_load(&self, _: &Client) -> Result<()> {
            self.0.fetch_add(100, Ordering::SeqCst);
            Ok(())
        }
    }
    assert!(bot
        .handle()
        .add_cog(Collision(count.clone()))
        .await
        .is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let definition = CommandSchema {
        name: "new".into(),
        description: "New".into(),
        aliases: vec![],
        arguments: vec![],
    };
    let command = DynamicCommandSpec::new(definition.clone())
        .bind(|_, _| async { Ok(()) })
        .unwrap();
    bot.handle()
        .add_cog(
            CogBuilder::new("dynamic")
                .registered_command(command)
                .build(),
        )
        .await
        .unwrap();
    assert!(bot
        .handle()
        .preflight_commands(&[definition.info()], Some("dynamic"))
        .await
        .is_ok());
    assert!(bot
        .handle()
        .preflight_commands(&[definition.info()], None)
        .await
        .is_err());
    event.data["content"] = "?frun".into();
    bot.process_event(event).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_totals_remain_exact_when_watch_coalesces_and_event_stream_lags() {
    let count = 300;
    let (base, mut requests, task) =
        mock((0..count).map(|_| response(200, json!({}))).collect()).await;
    let client = Client::builder("fixture-secret").base_url(base).build();
    let mut events = client.subscribe_request_events();
    let mut latest = client.subscribe_requests();
    let mut jobs = tokio::task::JoinSet::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(16));
    for _ in 0..count {
        let client = client.clone();
        let slots = slots.clone();
        jobs.spawn(async move {
            let _permit = slots.acquire().await.unwrap();
            shared_call(&client).await.unwrap();
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    let totals = client.request_metrics();
    assert_eq!(totals.started, count);
    assert_eq!(totals.completed, count);
    assert_eq!(totals.succeeded, count);
    assert_eq!(totals.attempts, count);
    assert_eq!(totals.observed_attempts, count);
    assert_eq!(totals.in_flight, 0);
    assert_eq!(totals.cancelled, 0);
    assert!(latest.has_changed().unwrap());
    latest.borrow_and_update();
    assert!(!latest.has_changed().unwrap());
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ));
    let mut previous = 0;
    while let Ok(event) = events.try_recv() {
        assert!(event.sequence > previous);
        previous = event.sequence;
        assert!(event.request_id > 0);
        assert_eq!(event.observation.route, "/:unknown");
    }
    assert_eq!(previous, totals.last_sequence);
    for _ in 0..count {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
}

#[tokio::test]
async fn metrics_count_retries_failures_and_cancellation_during_advised_waits() {
    let (base, mut requests, task) = mock(vec![
        (429, vec![("Retry-After", "0")], "{}".into()),
        response(200, json!({})),
        response(403, json!({"error":"denied"})),
        (503, vec![("Retry-After", "30")], "{}".into()),
    ])
    .await;
    let client = Client::builder("fixture-secret").base_url(base).build();
    shared_call(&client).await.unwrap();
    assert!(shared_call(&client).await.unwrap_err().is_forbidden());
    let before = client.request_metrics();
    assert_eq!(before.started, 2);
    assert_eq!(before.attempts, 3);
    assert_eq!(before.retries, 1);
    assert_eq!(before.failed, 1);
    assert_eq!(before.rate_limited, 1);
    let mut events = client.subscribe_request_events();
    let job = tokio::spawn({
        let client = client.clone();
        async move { shared_call(&client).await }
    });
    let event = events.recv().await.unwrap();
    assert_eq!(event.observation.status, Some(503));
    tokio::time::sleep(Duration::from_millis(15)).await;
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    let after = client.request_metrics();
    assert_eq!(after.cancelled, 1);
    assert_eq!(after.completed, 2);
    assert_eq!(after.in_flight, 0);
    assert!(after.total_rate_limit_wait >= Duration::from_millis(10));
    for _ in 0..4 {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
}

fn members() -> Vec<Value> {
    (1..=500).map(|n|json!({"id":Uuid::from_u128(n),"username":if n==2 {"target".into()} else {format!("member{n}")}})).collect()
}
#[tokio::test]
async fn bounded_search_never_confuses_one_candidate_with_proven_uniqueness() {
    let page = members();
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"members":page})),
        response(200, json!({"members":page})),
        response(200, json!({"members":page})),
        response(
            200,
            json!({"members":[{"id":Uuid::from_u128(501),"username":"target"}]}),
        ),
        response(200, json!({"members":page})),
        response(200, json!({"members":[]})),
    ])
    .await;
    let client = Client::builder("fixture-secret").base_url(base).build();
    let result = client
        .search_members(Uuid::nil(), "target", 1)
        .await
        .unwrap();
    assert_eq!(result.status(), LookupStatus::Incomplete);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.scanned, 500);
    assert_eq!(result.next_cursor, Some(Uuid::from_u128(500)));
    let error = client
        .resolve_member(Uuid::nil(), "target", 1)
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "incomplete_search");
    let result = client
        .search_members(Uuid::nil(), "target", 2)
        .await
        .unwrap();
    assert_eq!(result.status(), LookupStatus::Ambiguous);
    assert!(result.complete);
    assert_eq!(result.matches.len(), 2);
    let member = client
        .resolve_member(Uuid::nil(), "target", 2)
        .await
        .unwrap();
    assert_eq!(member.user.id, Uuid::from_u128(2));
    for _ in 0..6 {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
}

#[tokio::test]
async fn repeated_pages_remain_incomplete_and_exact_ids_do_not_scan() {
    let page = members();
    let id = Uuid::from_u128(9);
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"members":page})),
        response(200, json!({"members":page})),
        response(200, json!({"member":{"id":id,"username":"nine"}})),
    ])
    .await;
    let client = Client::builder("fixture-secret").base_url(base).build();
    let result = client.search_members(Uuid::nil(), "none", 2).await.unwrap();
    assert_eq!(result.status(), LookupStatus::Incomplete);
    assert_eq!(result.stop, SearchStop::NoProgress);
    assert_eq!(result.scanned, 500);
    let result = client
        .search_members(Uuid::nil(), &id.to_string(), 1)
        .await
        .unwrap();
    assert_eq!(result.status(), LookupStatus::Unique);
    assert_eq!(result.stop, SearchStop::ExactId);
    assert!(client
        .search_members(Uuid::nil(), &id.to_string(), 0)
        .await
        .is_err());
    for _ in 0..3 {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
}
