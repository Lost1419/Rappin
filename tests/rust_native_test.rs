mod support;
use rappin::*;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use support::*;
use uuid::Uuid;

#[derive(Debug, PartialEq)]
struct Roll {
    sides: BoundedU32<2, 100>,
    label: Option<String>,
}
impl FromArgs for Roll {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self {
            sides: args.required("sides")?,
            label: args.optional("label")?,
        })
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![
            ArgumentInfo::required("sides", "2 through 100"),
            ArgumentInfo::optional("label", "A quoted label"),
        ]
    }
}
fn raw(id: u64, content: &str) -> Event {
    let mut value = event(id, "unused");
    value["data"].as_object_mut().unwrap().remove("command");
    value["data"]["content"] = json!(content);
    serde_json::from_value(value).unwrap()
}

#[test]
fn typed_arguments_handle_quotes_unicode_bounds_and_errors() {
    let mut args = Arguments::new("20 '雪 day' ").unwrap();
    let roll = Roll::parse(&mut args).unwrap();
    args.finish().unwrap();
    assert_eq!(
        roll,
        Roll {
            sides: BoundedU32(20),
            label: Some("雪 day".into())
        }
    );
    let tokens = commands::tokenize(r#"one\ two "" 'a"b'"#).unwrap();
    assert_eq!(
        tokens.iter().map(|t| t.value.as_str()).collect::<Vec<_>>(),
        ["one two", "", "a\"b"]
    );
    assert!(Arguments::new("'unterminated").is_err());
    assert!(Arguments::new("trail\\").is_err());
    assert!(Roll::parse(&mut Arguments::new("101").unwrap()).is_err());
    assert!(Roll::parse(&mut Arguments::new("").unwrap()).is_err());
    let mut extra = Arguments::new("20 label extra").unwrap();
    Roll::parse(&mut extra).unwrap();
    assert_eq!(extra.finish(), Err(CommandError::Extra));
    assert_eq!(
        "1h30m".parse::<HumanDuration>().unwrap().0,
        Duration::from_secs(5400)
    );
    assert!("18446744073709551615w".parse::<HumanDuration>().is_err());
    assert!("0s".parse::<HumanDuration>().is_err());
    let id = Uuid::new_v4();
    assert_eq!(format!("<@{id}>").parse::<UserId>().unwrap().0, id);
    assert_eq!(format!("<#{id}>").parse::<ChannelId>().unwrap().0, id);
}

#[tokio::test]
async fn typed_routing_aliases_subcommands_prefixes_help_and_denials() {
    let (base, mut requests, task) = mock(vec![
        response(200, profile()),
        response(200, json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-10-27T00:00:00Z"}})),
        response(200, json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-10-27T00:00:00Z"}})),
        response(200, json!({"message":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":"2026-10-27T00:00:00Z"}})),
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .prefixes_with(|_| async { Ok(vec!["?".into()]) })
        .mention_prefix(true)
        .command_spec(
            CommandSpec::<Roll>::new("dice roll", "Roll dice").alias("roll"),
            move |ctx, roll| {
                let called = called.clone();
                async move {
                    assert_eq!(roll.sides.0, 20);
                    assert_eq!(roll.label.as_deref(), Some("two words"));
                    assert!(ctx.argv.len() <= 2);
                    called.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .command_spec(
            CommandSpec::<()>::new("private", "Private").check(Check::owner_only()),
            |_, _| async { panic!("denied handler ran") },
        )
        .help_command("help")
        .build();
    for content in [
        "?dice roll 20 'two words'",
        "?roll 20 'two words'",
        "<@11111111-1111-4111-8111-111111111111> dice roll 20 'two words'",
    ] {
        bot.process_event(raw(1, content)).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let help = bot.handle().help("dice").await;
    assert!(help.contains("/dice roll <sides> [label]"));
    assert!(help.contains("Aliases: roll"));
    bot.process_event(raw(2, "?roll 200")).await.unwrap();
    bot.process_event(raw(3, "?private")).await.unwrap();
    bot.process_event(raw(4, "?help dice")).await.unwrap();
    task.await.unwrap();
    let mut all = String::new();
    while let Ok(request) = requests.try_recv() {
        all.push_str(&request);
    }
    assert!(all.contains("invalid sides"));
    assert!(all.contains("only the bot owner"));
    assert!(all.contains("Usage:"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn invalid_and_conflicting_definitions_fail_before_network() {
    let bot = Bot::builder("secret")
        .base_url("http://127.0.0.1:1")
        .log(false)
        .command_spec(
            CommandSpec::<()>::new("ping", "Ping").alias("ping"),
            |_, _| async { Ok(()) },
        )
        .build();
    assert!(bot
        .process_event(raw(1, "!ping"))
        .await
        .unwrap_err()
        .to_string()
        .contains("already registered"));
}

#[tokio::test]
async fn collectors_filter_limit_cancel_overflow_and_timeout() {
    let hub = EventHub::default();
    let mut stream = hub
        .collect::<MessageEvent>()
        .channel(Uuid::nil())
        .author(Uuid::nil())
        .filter(|m| m.content.as_deref() == Some("answer"))
        .limit(1)
        .subscribe()
        .unwrap();
    hub.publish(raw(1, "wrong"));
    hub.publish(raw(2, "answer"));
    assert_eq!(
        stream.next().await.unwrap().unwrap().content.as_deref(),
        Some("answer")
    );
    assert!(stream.next().await.unwrap().is_none());
    assert_eq!(hub.subscriber_count(), 0);
    let mut lagged = hub.collect::<Event>().capacity(1).subscribe().unwrap();
    hub.publish(raw(3, "a"));
    hub.publish(raw(4, "b"));
    assert_eq!(lagged.next().await, Err(WaitError::Lagged));
    assert!(lagged.next().await.unwrap().is_none());
    let mut cancelled = hub.collect::<Event>().subscribe().unwrap();
    cancelled.cancel();
    assert_eq!(cancelled.next().await, Err(WaitError::Closed));
    let timeout = hub
        .collect::<Event>()
        .timeout(Duration::from_millis(10))
        .wait()
        .await;
    assert_eq!(timeout, Err(WaitError::Timeout));
    let dropped = hub.collect::<Event>().subscribe().unwrap();
    drop(dropped);
    assert_eq!(hub.subscriber_count(), 0);
    assert!(hub.collect::<Event>().capacity(4097).subscribe().is_err());
}

#[tokio::test]
async fn collectors_never_drop_user_captures_under_registry_lock() {
    let hub = EventHub::default();
    let nested = std::sync::Mutex::new(Some(hub.collect::<Event>().subscribe().unwrap()));
    let mut parent = hub
        .collect::<Event>()
        .capacity(1)
        .filter(move |_| {
            let _ = &nested;
            true
        })
        .subscribe()
        .unwrap();
    hub.publish(raw(1, "a"));
    hub.publish(raw(2, "b"));
    assert_eq!(parent.next().await, Err(WaitError::Lagged));
    assert_eq!(hub.subscriber_count(), 0);
}

#[tokio::test]
async fn interaction_clones_share_ack_state_and_allow_followups() {
    let (base, mut requests, task) =
        mock(vec![response(200, json!({})), response(200, json!({}))]).await;
    let client = Client::builder("secret").base_url(base).build();
    let event: InteractionEvent =
        serde_json::from_value(json!({"id":"click","token":"callback-secret"})).unwrap();
    let a = InteractionContext::new(Arc::new(client.clone()), event.clone(), None);
    let b = a.clone();
    let (one, two) = tokio::join!(a.ack(), b.ack());
    one.unwrap();
    two.unwrap();
    assert_eq!(
        a.response_state().await.unwrap().phase,
        ResponsePhase::Acknowledged
    );
    b.reply("finished").await.unwrap();
    a.ack().await.unwrap();
    assert_eq!(client.interaction_state(&event).await.unwrap().responses, 2);
    task.await.unwrap();
    assert!(requests.recv().await.unwrap().contains("\"type\":\"ack\""));
    assert!(requests.recv().await.unwrap().contains("finished"));
    assert!(requests.try_recv().is_err());
    let metrics = client.last_request().unwrap();
    assert_eq!(metrics.route, "/interactions/:id/:id/callback");
    assert!(!format!("{metrics:?}").contains("callback-secret"));
}

#[tokio::test]
async fn cancelled_interaction_write_remains_uncertain() {
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let client = Client::builder("secret")
        .base_url(format!("http://{address}"))
        .build();
    let event: InteractionEvent =
        serde_json::from_value(json!({"id":"click","token":"secret-click"})).unwrap();
    let c = client.clone();
    let e = event.clone();
    let write = tokio::spawn(async move {
        c.respond_interaction(&e.id, &e.token, &InteractionCallback::reply("one"))
            .await
    });
    ready.await.unwrap();
    write.abort();
    let _ = write.await;
    assert_eq!(
        client.interaction_state(&event).await.unwrap().phase,
        ResponsePhase::Uncertain
    );
    assert!(client
        .respond_interaction(&event.id, &event.token, &InteractionCallback::ack())
        .await
        .unwrap_err()
        .to_string()
        .contains("uncertain"));
    server.abort();
}

#[tokio::test]
async fn lookup_cache_is_opt_in_bounded_and_invalidated_by_writes() {
    let server = Uuid::nil();
    let user = Uuid::new_v4();
    let other = Uuid::new_v4();
    let member = |id| json!({"member":{"id":id,"username":"member","roles":[]}});
    let (base, mut requests, task) = mock(vec![
        response(200, member(user)),
        response(200, member(other)),
        response(200, member(user)),
        response(200, json!({"ok":true})),
        response(200, member(user)),
        response(200, member(user)),
    ])
    .await;
    let client = Client::builder("secret")
        .base_url(base)
        .lookup_cache(CachePolicy::new(1, Duration::from_secs(60)))
        .build();
    client
        .fetch_member(server, user, Freshness::PreferCache)
        .await
        .unwrap();
    client
        .fetch_member(server, user, Freshness::PreferCache)
        .await
        .unwrap();
    client
        .fetch_member(server, other, Freshness::PreferCache)
        .await
        .unwrap();
    assert!(client.cache().get_member(server, user).await.is_none());
    client
        .fetch_member(server, user, Freshness::PreferCache)
        .await
        .unwrap();
    client.timeout_member(server, user, 1, None).await.unwrap();
    assert!(client.cache().get_member(server, user).await.is_none());
    client
        .fetch_member(server, user, Freshness::PreferCache)
        .await
        .unwrap();
    client
        .fetch_member(server, user, Freshness::Refresh)
        .await
        .unwrap();
    task.await.unwrap();
    let mut count = 0;
    while requests.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 6);
}

#[tokio::test]
async fn checks_refresh_membership_and_fail_closed_on_unassigned_roles() {
    let server = Uuid::nil();
    let user = Uuid::nil();
    let role = Uuid::new_v4();
    let (base, _requests, task) = mock(vec![
        response(
            200,
            json!({"server":{"id":server,"name":"server","owner_id":Uuid::new_v4()}}),
        ),
        response(200, json!({"member":{"id":user,"username":"member","roles":[]}})),
        response(
            200,
            json!({"roles":[{"id":role,"name":"unassigned","auto":"everyone","manage_messages":true}]}),
        ),
        response(200, json!({"member":{"id":user,"username":"member","roles":[role]}})),
        response(200, json!({"member":{"id":user,"username":"member","roles":[]}})),
    ])
    .await;
    let client = Arc::new(
        Client::builder("secret")
            .base_url(base)
            .lookup_cache(CachePolicy::new(4, Duration::from_secs(60)))
            .build(),
    );
    let mut message: MessageEvent = serde_json::from_value(raw(1, "hello").data).unwrap();
    message.server_id = Some(server);
    let ctx = MessageContext::new(client, message, None, String::new(), None);
    assert!(Check::permissions([Permission::manage_messages()])
        .check(&ctx)
        .await
        .is_err());
    Check::roles([role]).check(&ctx).await.unwrap();
    Check::roles([role])
        .freshness(Freshness::PreferCache)
        .check(&ctx)
        .await
        .unwrap();
    assert!(Check::roles([role]).check(&ctx).await.is_err());
    task.await.unwrap();
}

#[tokio::test]
async fn pipeline_keeps_receiving_for_waiting_handler_and_commits_in_order() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let path = std::env::temp_dir().join(format!("rappin-pipeline-{}.json", Uuid::new_v4()));
    EventCursor::new(&path).save(10).await.unwrap();
    let registered = Arc::new(tokio::sync::Notify::new());
    let gate = registered.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for (index, body) in [
            profile(),
            json!({"events":[event(11,"question")],"last":11}),
            json!({"events":[serde_json::to_value(raw(19,"answer")).unwrap()],"last":19}),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut buf = [0; 4096];
            while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
            }
            if index == 2 {
                gate.notified().await;
                assert!(String::from_utf8_lossy(&request).contains("after=11"));
            }
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        std::future::pending::<()>().await;
    });
    let p = path.clone();
    let bot = Bot::builder("secret")
        .base_url(format!("http://{address}"))
        .sync_commands(false)
        .log(false)
        .cursor_file(&path)
        .handler_concurrency(1)
        .pending_batches(2)
        .command("question", "Wait", move |ctx| {
            let registered = registered.clone();
            let path = p.clone();
            async move {
                let reply = ctx.replies().timeout(Duration::from_secs(2)).subscribe()?;
                registered.notify_one();
                let answer = reply.wait().await?;
                assert_eq!(answer.content.as_deref(), Some("answer"));
                assert_eq!(EventCursor::new(path).load().await?, Some(10));
                ctx.bot().unwrap().stop();
                Ok(())
            }
        })
        .build();
    let handle = bot.handle();
    tokio::time::timeout(Duration::from_secs(3), bot.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(19));
    assert_eq!(handle.events().subscriber_count(), 0);
    assert_eq!(handle.transport_status().active, None);
    assert_eq!(handle.transport_status().committed_cursor, 19);
    server.abort();
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn auto_ack_defers_slow_handlers_but_skips_an_already_answered_interaction() {
    for (slow, aged) in [(true, false), (false, false), (true, true)] {
        let mut responses = vec![response(200, profile()), response(200, json!({}))];
        if slow {
            responses.push(response(200, json!({})));
        }
        let (base, mut requests, server) = mock(responses).await;
        let bot = Bot::builder("secret")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .auto_ack_after(Duration::from_millis(if aged { 100 } else { 10 }))
            .button("test", move |ctx| async move {
                if slow {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
                ctx.reply("done").await
            })
            .build();
        let mut event = json!({"id":1,"type":"interaction","data":{"id":"click","token":"callback-secret","custom_id":"test"}});
        if aged {
            event["created_at"] = json!(chrono::Utc::now() - chrono::Duration::seconds(2));
        }
        bot.process_event(serde_json::from_value(event).unwrap())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        let mut callbacks = vec![];
        while let Ok(request) = requests.try_recv() {
            if request.starts_with("POST") {
                callbacks.push(request);
            }
        }
        assert_eq!(callbacks.len(), if slow { 2 } else { 1 });
        if slow {
            assert!(callbacks[0].contains("\"type\":\"ack\""));
        }
        assert!(callbacks.last().unwrap().contains("done"));
    }
}

#[tokio::test]
async fn cache_expires_and_membership_events_invalidate_before_dispatch() {
    let user = Uuid::nil();
    let server = Uuid::nil();
    let (base, _requests, task) = mock(vec![
        response(200, json!({"member":{"id":user,"username":"member"}})),
        response(200, profile()),
    ])
    .await;
    let bot = Bot::builder("secret")
        .base_url(base)
        .log(false)
        .sync_commands(false)
        .lookup_cache(CachePolicy::new(2, Duration::from_secs(60)))
        .build();
    bot.client()
        .fetch_member(server, user, Freshness::PreferCache)
        .await
        .unwrap();
    assert!(bot.cache().get_member(server, user).await.is_some());
    bot.process_event(
        serde_json::from_value(
            json!({"type":"member_leave","data":{"user_id":user,"server_id":server}}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(bot.cache().get_member(server, user).await.is_none());
    task.await.unwrap();
    let (base, _requests, task) = mock(vec![response(
        200,
        json!({"member":{"id":user,"username":"member"}}),
    )])
    .await;
    let client = Client::builder("secret")
        .base_url(base)
        .lookup_cache(CachePolicy::new(2, Duration::from_millis(10)))
        .build();
    client.get_member(server, user).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(client.cache().get_member(server, user).await.is_none());
    task.await.unwrap();
}

#[tokio::test]
async fn server_retry_advice_is_shared_between_client_clones_and_observed() {
    let (base, _requests, server) = mock(vec![
        (
            429,
            vec![("Retry-After", "0.08")],
            json!({"error":"slow down"}).to_string(),
        ),
        response(200, json!({"ok":true})),
        response(200, json!({"ok":true})),
    ])
    .await;
    let client = Client::builder("secret")
        .base_url(base)
        .max_retries(0)
        .build();
    let result: Result<serde_json::Value> = client
        .request_json::<_, ()>(reqwest::Method::GET, "/channels/one/messages", None)
        .await;
    assert_eq!(result.unwrap_err().status_code(), Some(429));
    // An unrelated channel can proceed while the first route waits.
    let clone = client.clone();
    let _: serde_json::Value = tokio::time::timeout(
        Duration::from_millis(50),
        clone.request_json::<_, ()>(reqwest::Method::GET, "/channels/two/messages", None),
    )
    .await
    .unwrap()
    .unwrap();
    let _: serde_json::Value = client
        .request_json::<_, ()>(reqwest::Method::GET, "/channels/one/messages", None)
        .await
        .unwrap();
    let observation = clone.last_request().unwrap();
    assert_eq!(observation.status, Some(200));
    assert_eq!(observation.attempts, 1);
    assert!(observation.rate_limit_wait >= Duration::from_millis(20));
    assert_eq!(observation.route, "/channels/:id/messages");
    server.await.unwrap();
}

#[tokio::test]
async fn collector_idle_timeout_tracks_arrival_rather_than_consumer_speed() {
    let hub = EventHub::default();
    let mut collector = hub
        .collect::<Event>()
        .idle_timeout(Duration::from_millis(100))
        .timeout(Duration::from_secs(2))
        .subscribe()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    hub.publish(raw(1, "one"));
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(collector.next().await.unwrap().unwrap().id, 1);
    assert_eq!(collector.next().await, Err(WaitError::Timeout));
    assert_eq!(hub.subscriber_count(), 0);
}

#[tokio::test]
async fn later_success_never_commits_past_an_earlier_failed_batch() {
    let path = std::env::temp_dir().join(format!("rappin-order-{}.json", Uuid::new_v4()));
    EventCursor::new(&path).save(10).await.unwrap();
    let (base, _requests, server) = mock(vec![
        response(200, profile()),
        response(200, json!({"events":[event(11,"fail")],"last":11})),
        response(200, json!({"events":[event(20,"later")],"last":20})),
    ])
    .await;
    let done = Arc::new(tokio::sync::Notify::new());
    let wait = done.clone();
    let checked = path.clone();
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&path)
        .handler_concurrency(2)
        .pending_batches(2)
        .command("fail", "Fail after later event", move |_| {
            let wait = wait.clone();
            async move {
                wait.notified().await;
                Err(YappinError::Config("fixture failure".into()))
            }
        })
        .command("later", "Complete first", move |_| {
            let done = done.clone();
            let path = checked.clone();
            async move {
                assert_eq!(EventCursor::new(path).load().await?, Some(10));
                done.notify_one();
                Ok(())
            }
        })
        .build();
    assert!(tokio::time::timeout(Duration::from_secs(2), bot.run())
        .await
        .unwrap()
        .is_err());
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(10));
    server.await.unwrap();
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn concurrency_one_orders_handlers_across_batch_boundaries() {
    let mut fixtures: Vec<_> = (2..=4).map(|id| event(id, "work")).collect();
    for (index, fixture) in fixtures.iter_mut().enumerate() {
        fixture["data"]["id"] = json!(Uuid::from_u128(index as u128 + 2));
    }
    let (base, _requests, server) = mock(vec![
        response(200, profile()),
        response(200, json!({"last":1})),
        response(200, json!({"events":fixtures[..2],"last":3})),
        response(200, json!({"events":fixtures[2..],"last":4})),
    ])
    .await;
    let ids = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let seen = ids.clone();
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .handler_concurrency(1)
        .pending_batches(3)
        .command("work", "Order", move |ctx| {
            let seen = seen.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                let mut ids = seen.lock().await;
                ids.push(ctx.message_id());
                if ids.len() == 3 {
                    ctx.bot().unwrap().stop();
                }
                Ok(())
            }
        })
        .build();
    tokio::time::timeout(Duration::from_secs(2), bot.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *ids.lock().await,
        vec![Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4)]
    );
    server.await.unwrap();
}

#[tokio::test]
async fn aborting_run_closes_collectors_and_clears_transport_status() {
    let (base, _requests, server) = mock(vec![
        response(200, profile()),
        response(200, json!({"last":1})),
    ])
    .await;
    let bot = Bot::builder("secret")
        .base_url(base)
        .log(false)
        .sync_commands(false)
        .build();
    let handle = bot.handle();
    let mut status = handle.subscribe_transport();
    let waiter = handle.events().collect::<Event>().subscribe().unwrap();
    let runner = tokio::spawn(bot.run());
    tokio::time::timeout(Duration::from_secs(2), async {
        while status.borrow_and_update().active.is_none() {
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    runner.abort();
    let _ = runner.await;
    assert_eq!(waiter.wait().await, Err(WaitError::Closed));
    assert_eq!(handle.events().subscriber_count(), 0);
    assert_eq!(handle.transport_status().active, None);
    assert!(matches!(
        handle.events().collect::<Event>().subscribe(),
        Err(WaitError::Closed)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn native_cog_replacement_updates_aliases_checks_and_live_help_together() {
    let bot = Bot::builder("secret")
        .base_url("http://127.0.0.1:1")
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let checked = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let check = checked.clone();
    bot.add_cog(
        CogBuilder::new("typed")
            .command_spec(
                CommandSpec::<()>::new("work", "Original")
                    .alias("fast")
                    .check(check_fn(move |_| {
                        let check = check.clone();
                        async move {
                            check.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    })),
                move |_, ()| {
                    let c = c.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .build(),
    )
    .await
    .unwrap();
    bot.process_event(raw(1, "!fast")).await.unwrap();
    assert_eq!(checked.load(Ordering::SeqCst), 1);
    let c = calls.clone();
    bot.reload_cog(
        CogBuilder::new("typed")
            .command_spec(
                CommandSpec::<()>::new("work", "Replacement").alias("quick"),
                move |_, ()| {
                    let c = c.clone();
                    async move {
                        c.fetch_add(10, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .build(),
    )
    .await
    .unwrap();
    assert!(bot.handle().help("fast").await.is_empty());
    assert!(bot.handle().help("quick").await.contains("Replacement"));
    bot.process_event(raw(2, "!fast")).await.unwrap();
    bot.process_event(raw(3, "!quick")).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 11);
    assert_eq!(checked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn slash_sync_advertises_canonical_roots_without_local_aliases() {
    let (base, mut requests, server) = mock(vec![response(200, json!({"commands":[]}))]).await;
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .command_spec(
            CommandSpec::<Roll>::new("dice roll", "Child").alias("roll"),
            |_, _| async { Ok(()) },
        )
        .command("dice", "Root description", |_| async { Ok(()) })
        .build();
    bot.handle().sync_commands().await.unwrap();
    let request = requests.recv().await.unwrap();
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body["commands"],
        json!([{"name":"dice","description":"Root description"}])
    );
    server.await.unwrap();
}

#[cfg(not(feature = "gateway"))]
#[tokio::test]
async fn unsupported_gateway_mode_fails_before_authentication() {
    let bot = Bot::builder("secret")
        .base_url("http://127.0.0.1:1")
        .log(false)
        .transport_mode(TransportMode::GatewayOnly)
        .build();
    assert!(bot
        .run()
        .await
        .unwrap_err()
        .to_string()
        .contains("enable the gateway Cargo feature"));
}

#[tokio::test]
async fn unpolled_collector_cannot_revive_after_its_idle_deadline() {
    let hub = EventHub::default();
    let mut collector = hub
        .collect::<Event>()
        .idle_timeout(Duration::from_millis(10))
        .subscribe()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    hub.publish(raw(1, "too late"));
    assert_eq!(collector.next().await, Err(WaitError::Timeout));
    assert_eq!(hub.subscriber_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_collectors_isolate_routes_and_release_every_subscription() {
    let hub = EventHub::default();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let hub = hub.clone();
        let channel = Uuid::new_v4();
        tasks.spawn(async move {
            for sequence in 0..100 {
                let waiter = hub
                    .collect::<MessageEvent>()
                    .channel(channel)
                    .capacity(2)
                    .timeout(Duration::from_secs(3))
                    .subscribe()
                    .unwrap();
                let mut message = raw(sequence, "scoped");
                message.data["channel_id"] = json!(channel);
                hub.publish(message);
                let received = waiter.wait().await.unwrap();
                assert_eq!(received.channel_id, channel);
                tokio::task::yield_now().await;
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert_eq!(hub.subscriber_count(), 0);
}

#[tokio::test]
async fn collector_subscription_limit_recovers_after_drop() {
    let hub = EventHub::default();
    let mut subscriptions = Vec::with_capacity(1024);
    for _ in 0..1024 {
        subscriptions.push(hub.collect::<Event>().capacity(1).subscribe().unwrap());
    }
    assert!(matches!(
        hub.collect::<Event>().subscribe(),
        Err(WaitError::Limit)
    ));
    drop(subscriptions.pop());
    let replacement = hub.collect::<Event>().subscribe().unwrap();
    assert_eq!(hub.subscriber_count(), 1024);
    drop(replacement);
    drop(subscriptions);
    assert_eq!(hub.subscriber_count(), 0);
}

#[tokio::test]
async fn component_collectors_isolate_overlapping_prompts_and_ignore_other_users() {
    let client = Arc::new(Client::new("fixture-secret"));
    let message: MessageEvent = serde_json::from_value(raw(1, "prompt").data).unwrap();
    let ctx = MessageContext::new(client.clone(), message, None, String::new(), None);
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let first_wait = ctx
        .component_events(first)
        .custom_id_prefix("shared:")
        .capacity(1)
        .limit(1)
        .subscribe()
        .unwrap();
    let second_wait = ctx
        .component_events(second)
        .custom_id_prefix("shared:")
        .capacity(1)
        .limit(1)
        .subscribe()
        .unwrap();
    let click = |id, message, user| {
        serde_json::from_value(json!({
        "id":id,"type":"interaction","data":{"id":format!("click-{id}"),"token":"fixture-callback",
        "channel_id":Uuid::nil(),"user_id":user,"message_id":message,"custom_id":"shared:confirm"}
    })).unwrap()
    };
    client.events().publish(click(2, first, Uuid::new_v4()));
    client.events().publish(click(3, second, Uuid::nil()));
    client.events().publish(click(4, first, Uuid::nil()));
    let (one, two) = tokio::join!(first_wait.wait(), second_wait.wait());
    assert_eq!(one.unwrap().message_id, Some(first));
    assert_eq!(two.unwrap().message_id, Some(second));
    assert_eq!(client.events().subscriber_count(), 0);
}
