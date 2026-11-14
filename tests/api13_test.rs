mod support;
use rappin::{capabilities::*, *};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use uuid::Uuid;
fn menu() -> SelectMenu {
    SelectMenu::new("choice").option(SelectOption::new("Rust", "rust").emoji(EmojiRef::custom(
        Uuid::nil(),
        "crab",
        false,
    )))
}
fn body() -> CreateMessage {
    CreateMessage::new("Choose").component(menu().row())
}
fn interaction(kind: &str) -> Event {
    let mut data = json!({"id":Uuid::new_v4(),"token":"private_interaction_token","channel_id":Uuid::nil(),"server_id":Uuid::nil(),"user_id":Uuid::nil(),"custom_id":"choice","type":kind,"expires_at":chrono::Utc::now()+chrono::Duration::minutes(15)});
    if kind == "select" {
        data["values"] = json!(["rust"]);
    }
    if kind == "modal_submit" {
        data["values"] = json!({"reason":"Rust","choice":["rust"]});
    }
    if kind == "command" {
        data["command"] = json!("apply");
        data["args"] = json!("abc");
        data.as_object_mut().unwrap().remove("type");
    }
    serde_json::from_value(
        json!({"id":1,"type":if kind=="command" {"command"} else {"interaction"},"data":data}),
    )
    .unwrap()
}
fn message() -> Value {
    json!({"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":chrono::Utc::now(),"content":"Choose","components":body().components.unwrap(),"future_metadata":{"a":1}})
}
#[test]
fn select_bounds_rows_ids_and_structured_emojis_validate_before_io() {
    body().validate().unwrap();
    let definition = capability("send_message").unwrap();
    definition
        .validate(&json!({"channel":Uuid::nil(),"body":body()}))
        .unwrap();
    let mut numeric = json!(body());
    numeric["components"][0]["type"] = 1.into();
    numeric["components"][0]["components"][0]["type"] = 3.into();
    validation::validate_message(&numeric, true).unwrap();
    definition
        .validate(&json!({"channel":Uuid::nil(),"body":numeric}))
        .unwrap();
    let layout = CreateMessage::layout([Component::container([menu().row()])]);
    layout.validate().unwrap();
    definition
        .validate(&json!({"channel":Uuid::nil(),"body":layout}))
        .unwrap();
    for mutate in 0..7 {
        let mut value = json!(body());
        let select = &mut value["components"][0]["components"][0];
        match mutate {
            0 => select["max_values"] = 2.into(),
            1 => select["min_values"] = (-1).into(),
            2 => select["options"] = json!([]),
            3 => select["options"][0]["emoji"]["id"] = "invalid".into(),
            4 => select["options"][0]["default"] = "yes".into(),
            5 => select["options"][0]["description"] = json!({}),
            _ => select["options"] = json!(vec![json!({"label":"x","value":"x"}); 26]),
        }
        assert!(
            validation::validate_message(&value, true).is_err(),
            "mutation {mutate}"
        );
    }
    let duplicate = CreateMessage::new("Choose")
        .component(menu().row())
        .component(Component::row([Button::primary("x", "choice")]));
    assert!(duplicate.validate().is_err());
    let mixed = CreateMessage::new("Choose").component(Component::row([
        Component::from(menu()),
        Component::from(Button::primary("x", "x")),
    ]));
    assert!(mixed.validate().is_err());
    let mut duplicate = menu();
    duplicate.options.push(duplicate.options[0].clone());
    assert!(CreateMessage::new("Choose")
        .component(duplicate.row())
        .validate()
        .is_err());
}
#[test]
fn modal_text_select_limits_patch_nulls_and_wrappers_are_validated() {
    let modal = Modal::new("form", "Apply")
        .input(TextInput::paragraph("why", "Why?"))
        .input(ModalSelect::new("Choice", menu()));
    modal.validate().unwrap();
    let wrapped = json!({"custom_id":"form","title":"Apply","components":[{"type":18,"label":"Why?","component":{"type":4,"custom_id":"why","style":2}}]});
    validation::validate_modal(&wrapped).unwrap();
    for field in ["min_length", "max_length"] {
        let mut bad = json!(modal);
        bad["components"][0][field] = (-1).into();
        assert!(validation::validate_modal(&bad).is_err());
    }
    let mut duplicate = modal.clone();
    duplicate.components.push(duplicate.components[0].clone());
    assert!(duplicate.validate().is_err());
    let mut too_many = modal.clone();
    too_many.components = (0..6)
        .map(|n| ModalInput::from(TextInput::short(format!("v{n}"), "x")))
        .collect();
    assert!(too_many.validate().is_err());
    let patch: EditMessage =
        serde_json::from_value(json!({"components":null,"content":null})).unwrap();
    assert_eq!(json!(patch), json!({"components":null,"content":null}));
    assert!(!format!(
        "{:?}",
        UiInteraction::from_event(&interaction("select")).unwrap()
    )
    .contains("private_interaction_token"));
}
#[tokio::test]
async fn shared_native_api_keeps_ui_responses_and_emoji_metadata_lossless() {
    let found = json!({"id":Uuid::nil(),"name":"crab","animated":false,"url":"https://files.yapper.website/emojis/picture.webp","in_server":false,"text":"<:crab:00000000-0000-0000-0000-000000000000>"});
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({"emojis":[found]})),
        support::response(200, json!({"message":message()})),
        support::response(200, json!({"messages":[message()],"authors":[]})),
    ])
    .await;
    let client = Client::builder("fixture_secret").base_url(base).build();
    let emojis = client
        .get_message_emojis(Uuid::nil(), Uuid::nil())
        .await
        .unwrap();
    assert!(!emojis.emojis[0].in_server);
    let response = client.send_message(Uuid::nil(), &body()).await.unwrap();
    assert_eq!(response.extra["future_metadata"]["a"], 1);
    assert_eq!(
        json!(response)["components"][0]["components"][0]["options"][0]["emoji"]["name"],
        "crab"
    );
    let messages = client
        .get_messages(Uuid::nil(), Some(1), None, None)
        .await
        .unwrap();
    assert_eq!(messages.messages.len(), 1);
    let first = requests.recv().await.unwrap();
    assert!(first.starts_with(&format!(
        "GET /channels/{}/messages/{}/emojis ",
        Uuid::nil(),
        Uuid::nil()
    )));
    for _ in 0..2 {
        requests.recv().await.unwrap();
    }
    http.await.unwrap();
}
#[tokio::test]
async fn modal_is_initial_only_and_unknown_responses_cannot_be_replayed() {
    for status in [200, 403, 502] {
        let mut responses = vec![support::response(status, json!({"error":"fixture"}))];
        if status == 403 {
            responses.push(support::response(200, json!({"ok":true})));
        }
        let (base, mut requests, http) = support::mock(responses).await;
        let client = Arc::new(Client::builder("fixture").base_url(base).build());
        let event = UiInteraction::from_event(&interaction("button"))
            .unwrap()
            .interaction;
        let ctx = InteractionContext::new(client.clone(), event, None);
        let modal = Modal::new("apply", "Apply").input(TextInput::short("name", "Name"));
        let result = ctx.show_modal(&modal).await;
        if status == 200 {
            result.unwrap();
            assert!(ctx.show_modal(&modal).await.is_err());
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.status_code(), Some(status));
            if status == 403 {
                ctx.show_modal(&modal).await.unwrap();
            } else {
                assert_eq!(
                    ctx.response_state().await.unwrap().phase,
                    ResponsePhase::Uncertain
                );
                assert!(ctx.show_modal(&modal).await.is_err());
            }
        }
        let count = if status == 403 { 2 } else { 1 };
        for _ in 0..count {
            requests.recv().await.unwrap();
        }
        assert!(requests.try_recv().is_err());
        http.await.unwrap();
    }
    let client = Client::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .build();
    let event = UiInteraction::from_event(&interaction("modal_submit")).unwrap();
    assert!(client
        .show_modal(
            &event.interaction,
            &Modal::new("f", "F").input(TextInput::short("n", "N"))
        )
        .await
        .is_err());
    assert!(client
        .respond(
            &event.interaction,
            InteractionCallbackType::Update,
            &CreateMessage::new("no source")
        )
        .await
        .is_err());
    assert_eq!(client.request_metrics().attempts, 0);
    let mut reply = CreateMessage::new("reply");
    reply.reply_to = Some(Uuid::nil());
    assert!(client
        .respond(&event.interaction, InteractionCallbackType::Reply, &reply)
        .await
        .is_err());
    assert_eq!(client.request_metrics().attempts, 0);
}
#[tokio::test]
async fn opt_in_ui_routing_keeps_values_separate_from_legacy_buttons_and_failures_explicit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let d = calls.clone();
    let e = calls.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .button("choice", |_| async {
            panic!("new UI event reached legacy button")
        })
        .on_select(move |ctx| {
            let c = c.clone();
            async move {
                assert!(matches!(ctx.values, Some(InteractionValues::Selected(_))));
                c.fetch_add(1, Ordering::SeqCst);
                Err(Error::Config("expected failure".into()))
            }
        })
        .on_modal(move |ctx| {
            let d = d.clone();
            async move {
                assert!(matches!(ctx.values, Some(InteractionValues::Submitted(_))));
                d.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .on_command_interaction(move |ctx| {
            let e = e.clone();
            async move {
                assert_eq!(ctx.command.as_deref(), Some("apply"));
                e.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .on_button_failure(|_, failure| async move {
            assert_eq!(failure.handler, "ui:select");
            Ok(FailureDisposition::Handled)
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    for kind in ["select", "modal_submit", "command"] {
        bot.process_event(interaction(kind)).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let legacy = Arc::new(AtomicUsize::new(0));
    let c = legacy.clone();
    let bot = Bot::builder("fixture")
        .interactive_events(false)
        .sync_commands(false)
        .log(false)
        .button("choice", move |_| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    bot.process_event(interaction("select")).await.unwrap();
    assert_eq!(legacy.load(Ordering::SeqCst), 1);
}
#[test]
fn emoji_lookup_catalogue_scope_and_rollout_are_explicit() {
    let entry = capability("get_message_emojis").unwrap();
    assert!(entry.availability.test_servers_only);
    assert!(entry
        .permissions
        .requester_channel
        .contains(&"read_history"));
    assert!(entry.permissions.bot_channel.contains(&"read_history"));
    let call = entry
        .validate(&json!({"channel":Uuid::nil(),"message":Uuid::nil()}))
        .unwrap();
    let request = call.api_request().unwrap();
    assert_eq!(request.method, reqwest::Method::GET);
    assert!(request.path.ends_with("/emojis"));
}

#[tokio::test]
async fn ui_batches_deduplicate_and_checkpoint_only_completed_work_on_both_transports() {
    for gateway in [false, true] {
        #[cfg(not(feature = "gateway"))]
        if gateway {
            continue;
        }
        for handled in [false, true] {
            let root = std::env::temp_dir().join(format!("rappin-ui-cursor-{}", Uuid::new_v4()));
            tokio::fs::create_dir(&root).await.unwrap();
            let path = root.join("cursor.json");
            EventCursor::new(&path).save(7).await.unwrap();
            let mut events = Vec::new();
            for (id, kind) in [
                (8, "select"),
                (8, "select"),
                (9, "modal_submit"),
                (10, "command"),
            ] {
                let mut e = interaction(kind);
                e.id = id;
                events.push(json!(e));
            }
            let mut routes = vec![(
                "GET /me".into(),
                vec![support::response(200, support::profile())],
            )];
            if !gateway {
                routes.push((
                    "GET /events".into(),
                    vec![support::response(200, json!({"events":events,"last":10}))],
                ));
            }
            let (base, _requests, http) = support::routed_mock(routes).await;
            let select_calls = Arc::new(AtomicUsize::new(0));
            let a = select_calls.clone();
            let command_calls = Arc::new(AtomicUsize::new(0));
            let b = command_calls.clone();
            #[allow(unused_mut)]
            let mut builder = Bot::builder("fixture")
                .base_url(base)
                .sync_commands(false)
                .log(false)
                .configure(|settings| {
                    settings.persistence.cursor = Some(rappin::EventCursor::new(&path))
                })
                .configure(|settings| settings.execution.concurrency = 1)
                .on_select(move |_| {
                    let a = a.clone();
                    async move {
                        a.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .on_modal(|_| async { Err(Error::Config("unprocessed modal failure".into())) })
                .on_command_interaction(move |ctx| {
                    let b = b.clone();
                    async move {
                        b.fetch_add(1, Ordering::SeqCst);
                        ctx.bot().unwrap().stop();
                        Ok(())
                    }
                })
                .on_button_failure(move |_, _| async move {
                    Ok(if handled {
                        FailureDisposition::Handled
                    } else {
                        FailureDisposition::Stop
                    })
                });
            #[allow(unused_mut)]
            let mut ws: Option<tokio::task::JoinHandle<()>> = None;
            #[cfg(feature = "gateway")]
            if gateway {
                use futures_util::{SinkExt, StreamExt};
                use tokio_tungstenite::{accept_async, tungstenite::Message};
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                ws = Some(tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut socket = accept_async(stream).await.unwrap();
                    socket
                        .send(Message::Text(
                            json!({"op":"hello","heartbeat_interval":1000})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    let identify = socket.next().await.unwrap().unwrap();
                    let value: Value = serde_json::from_str(identify.to_text().unwrap()).unwrap();
                    assert_eq!(value["seq"], 7);
                    for event in events {
                        socket
                            .send(Message::Text(
                                json!({"op":"event","s":event["id"],"d":event})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    while let Some(Ok(message)) = socket.next().await {
                        if message.is_close() {
                            break;
                        }
                        if message
                            .to_text()
                            .is_ok_and(|text| text.contains("heartbeat"))
                        {
                            let _ = socket
                                .send(Message::Text(
                                    json!({"op":"heartbeat_ack"}).to_string().into(),
                                ))
                                .await;
                        }
                    }
                }));
                builder = builder.configure(|settings| {
                    settings.transport.gateway_url = Some(format!("ws://{address}"));
                    settings.transport.mode = rappin::TransportMode::GatewayOnly;
                });
            }
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                builder.build().expect("valid bot configuration").run(),
            )
            .await
            .unwrap();
            assert_eq!(result.is_ok(), handled);
            assert_eq!(select_calls.load(Ordering::SeqCst), 1);
            assert_eq!(command_calls.load(Ordering::SeqCst), usize::from(handled));
            assert_eq!(
                EventCursor::new(&path).load().await.unwrap(),
                Some(if handled { 10 } else { 8 })
            );
            http.abort();
            if let Some(ws) = ws {
                tokio::time::timeout(std::time::Duration::from_secs(2), ws)
                    .await
                    .unwrap()
                    .unwrap();
            }
            tokio::fs::remove_dir_all(root).await.unwrap();
        }
    }
}

#[tokio::test]
async fn command_publication_mode_is_opt_in_and_modal_deadlines_are_authoritative() {
    for enabled in [false, true] {
        let commands = json!([{"name":"apply","description":"Apply"}]);
        let (base, mut requests, http) =
            support::mock(vec![support::response(200, json!({"commands":commands}))]).await;
        let bot = Bot::builder("fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .interaction_commands(enabled)
            .command("apply", "Apply", |_| async { Ok(()) })
            .build()
            .expect("valid bot configuration");
        bot.handle().sync_commands().await.unwrap();
        let request = requests.recv().await.unwrap();
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body.get("interactions"),
            if enabled {
                Some(&Value::Bool(true))
            } else {
                None
            }
        );
        http.await.unwrap();
    }
    let client = Client::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .build();
    let mut event = UiInteraction::from_event(&interaction("button"))
        .unwrap()
        .interaction;
    event.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    let error = client
        .show_modal(
            &event,
            &Modal::new("x", "X").input(TextInput::short("n", "N")),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("expired"));
    assert_eq!(client.request_metrics().attempts, 0);
}

#[tokio::test]
async fn extended_context_forwards_public_reads_and_upload_transport_without_auth_leaks() {
    let (base,mut requests,http)=support::mock(vec![support::response(200,json!({"api":"1.3.0","build":"fixture","sdk":{"js":"1.6.0","python":"1.6.0"},"packages":{"npm":"yapin","pypi":"yapin"}})),support::response(200,json!({"messages":[]}))]).await;
    let client = Arc::new(
        Client::builder("private_parent_token")
            .base_url(base)
            .build(),
    );
    let ctx = InteractionContext::new(
        client,
        UiInteraction::from_event(&interaction("select")).unwrap(),
        None,
    );
    let sanitized =
        ctx.sanitize(json!({"token":"interaction_secret","text":"private_parent_token"}));
    assert_eq!(sanitized["token"], "[REDACTED]");
    assert!(!sanitized.to_string().contains("private_parent_token"));
    assert_eq!(ctx.get_api_version().await.unwrap().api, "1.3.0");
    ctx.upload(
        &format!("/channels/{}/messages", Uuid::nil()),
        json!({"content":"file"}),
        &[UploadFile::new("note.txt", b"safe".to_vec())],
    )
    .await
    .unwrap();
    let public = requests.recv().await.unwrap();
    assert!(!public.to_lowercase().contains("authorization:"));
    let upload = requests.recv().await.unwrap();
    assert!(upload
        .to_lowercase()
        .contains("authorization: bot private_parent_token"));
    assert!(upload.contains("note.txt"));
    http.await.unwrap();
}

#[tokio::test]
async fn unified_interaction_updates_preserve_explicit_nulls_and_response_ownership() {
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({})),
        support::response(200, json!({})),
    ])
    .await;
    let client = std::sync::Arc::new(Client::builder("fixture").base_url(base).build());
    let data = UiInteraction::from_event(&interaction("select")).unwrap();
    let ctx = InteractionContext::new(client, data, None);
    ctx.update_with(&EditMessage::new().clear_content().clear_components())
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert!(request.contains("\"content\":null"));
    assert!(request.contains("\"components\":null"));
    assert_eq!(ctx.response_state().await.unwrap().responses, 1);
    let other = ctx.clone();
    other
        .update_with(&EditMessage::new().content("follow-up"))
        .await
        .unwrap();
    assert!(requests.recv().await.unwrap().contains("follow-up"));
    assert_eq!(ctx.response_state().await.unwrap().responses, 2);
    assert!(requests.try_recv().is_err());
    http.await.unwrap();
}
