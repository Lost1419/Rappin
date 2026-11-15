use rappin::{capabilities::capability, *};
use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;

const NONCE: &str = "send-test";

struct NonceFixture;

#[async_trait::async_trait]
impl Api for NonceFixture {
    async fn request(&self, _: Method, _: &str, _: Option<Value>) -> Result<Value> {
        panic!("nonce sends must use the single-attempt API");
    }

    async fn request_once(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        assert_eq!(method, Method::POST);
        assert_eq!(path, format!("/channels/{}/messages", Uuid::nil()));
        assert_eq!(body.unwrap()["nonce"], NONCE);
        Ok(json!({"nonce":NONCE,"duplicate":true,"message":{
            "id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),
            "created_at":"2026-11-13T00:00:00Z","content":"hello"
        }}))
    }
}

#[tokio::test]
async fn nonce_send_uses_one_attempt_and_retains_duplicate_receipt() {
    let body = CreateMessage::new("hello");
    assert!(NonceFixture
        .send_message(Uuid::nil(), &body.clone().nonce("bad"))
        .await
        .is_err());
    let result = NonceFixture
        .send_message(Uuid::nil(), &body.nonce(NONCE))
        .await
        .unwrap();
    assert!(result.duplicate);
    assert_eq!(result.nonce.as_deref(), Some(NONCE));
}

#[test]
fn capability_keeps_private_audience_and_blocks_public_fallback() {
    let id = Uuid::nil();
    let create = capability("create_channel").unwrap();
    let operation = create
        .validate(&json!({"server":id,"body":{
            "name":"private","private":true,"role_ids":[],"user_ids":[id]
        }}))
        .unwrap();
    assert_eq!(
        operation.preview().data["arguments"]["body"]["user_ids"],
        json!([id])
    );
    assert!(create
        .validate(&json!({"server":id,"body":{
            "name":"private","user_ids":[id]
        }}))
        .is_err());
}

#[test]
fn slash_options_keep_positional_and_named_values() {
    let mut user = SlashOption::new("user", SlashOptionKind::User);
    user.required = true;
    let command = Command::new("ban", "")
        .options([user, SlashOption::new("reason", SlashOptionKind::String)]);
    let id = Uuid::nil();
    let named = command
        .parse_options(&format!(r#"reason="spam and ads" user=<@{id}>"#))
        .unwrap();
    assert_eq!(
        named,
        command
            .parse_options(&format!("<@{id}> spam and ads"))
            .unwrap()
    );
    assert!(command.parse_options("reason=missing-user").is_err());
    assert!(command
        .parse_options(&format!("user={id} user={id}"))
        .is_err());
}

#[test]
fn metadata_and_new_events_decode_through_normal_event_dispatch() {
    let packages: SdkPackages = serde_json::from_value(json!({"npm":null,"pypi":null})).unwrap();
    assert!(packages.npm.is_none());
    let event: Event = serde_json::from_value(json!({
        "id":1, "type":"message_delete_bulk",
        "data":{"ids":[Uuid::nil()],"channel_id":Uuid::nil(),"server_id":Uuid::nil()}
    }))
    .unwrap();
    assert!(matches!(
        event.parse_change().unwrap(),
        Some(ChangeEvent::MessageDeleteBulk(_))
    ));
    assert!(matches!(
        event.parse_data().unwrap(),
        EventData::Change(ChangeEvent::MessageDeleteBulk(_))
    ));
}

#[test]
fn new_capability_routes_and_request_validation_match_the_contract() {
    let create = capability("create_thread").unwrap();
    assert!(create
        .permissions
        .requester_channel
        .contains(&"send_in_threads"));
    let channel = Uuid::nil();
    let route = capability("list_threads")
        .unwrap()
        .validate(&json!({"channel":channel,"limit":50}))
        .unwrap()
        .api_request()
        .unwrap();
    assert_eq!(route.path, format!("/channels/{channel}/threads?limit=50"));
    let moderate = capability("moderate_reactions").unwrap();
    let route = moderate
        .validate(&json!({"channel":channel,"message":channel,"emoji":"👍","user":channel}))
        .unwrap()
        .api_request()
        .unwrap();
    assert!(route
        .path
        .ends_with(&format!("/%F0%9F%91%8D?user_id={channel}")));
    assert!(moderate
        .validate(&json!({"channel":channel,"message":channel,"user":channel}))
        .is_err());
    let publish = capability("publish_slash_commands").unwrap().validate(&json!({
        "body":{"commands":[{"name":"find","description":"Find items","options":[{"name":"query"}]}]}
    })).unwrap().api_request().unwrap();
    assert_eq!(publish.path, "/commands");
    assert_eq!(publish.method, Method::PUT);
}

mod support;

#[tokio::test]
async fn structured_errors_preserve_the_platform_code_by_default() {
    let error = support::response(
        403,
        json!({
            "error":"Feature unavailable",
            "code":"feature_off",
            "permission":"manage_messages"
        }),
    );
    let (url, _requests, server) = support::mock(vec![error]).await;
    let client = Client::builder("example").base_url(url).build();
    let error = client.list_threads(Uuid::nil(), None).await.unwrap_err();
    assert_eq!(error.status_code(), Some(403));
    assert_eq!(error.to_json()["code"], "feature_off");
    assert_eq!(error.to_json()["permission"], "manage_messages");
    server.await.unwrap();
}

#[test]
fn ephemeral_buttons_validate_and_preserve_reply_target() {
    let data = json!({
        "id":"example", "token":"example", "custom_id":"next", "type":"button",
        "reply_id": Uuid::nil()
    });
    let event: Event =
        serde_json::from_value(json!({"id":1,"type":"interaction","data":data})).unwrap();
    assert_eq!(
        UiInteraction::from_event(&event)
            .unwrap()
            .interaction
            .reply_id,
        Some(Uuid::nil())
    );
    let callback: InteractionCallback = serde_json::from_value(json!({
        "type":"ephemeral","content":"Page one","components":[{"type":1,"components":[
            {"type":2,"style":1,"label":"Next","custom_id":"next"}
        ]}]
    }))
    .unwrap();
    callback.validate().unwrap();
}

#[tokio::test]
async fn bot_sync_publishes_options_only_for_configured_command_roots() {
    let response = support::response(
        200,
        json!({"commands":[{"name":"find","description":"Find items"}]}),
    );
    let (url, mut requests, server) = support::mock(vec![response]).await;
    let bot = Bot::builder("example")
        .base_url(url)
        .command_spec(
            CommandSpec::<()>::new("find", "Find items")
                .slash_options([SlashOption::new("query", SlashOptionKind::String)]),
            |_, ()| async { Ok(()) },
        )
        .build()
        .unwrap();
    bot.handle().sync_commands().await.unwrap();
    let request = requests.recv().await.unwrap();
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["commands"][0]["options"][0]["name"], "query");
    assert_eq!(body["interactions"], true);
    server.await.unwrap();
}

#[tokio::test]
async fn native_slash_forms_reuse_declared_metadata_and_preserve_original_arguments() {
    let bot = Bot::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .sync_commands(false)
        .command_spec(
            CommandSpec::<Arguments>::new("find", "").slash_options([
                SlashOption::new("query", SlashOptionKind::String),
                SlashOption::new("days", SlashOptionKind::Integer),
            ]),
            |ctx, _args| async move {
                assert_eq!(ctx.args(), r#"days=7 query="two words""#);
                assert_eq!(
                    ctx.parse_slash_options()?,
                    [
                        ("days".into(), json!(7)),
                        ("query".into(), json!("two words"))
                    ]
                    .into()
                );
                Ok(())
            },
        )
        .build()
        .unwrap();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let event = serde_json::from_value(json!({"id":1,"type":"command","data":{
        "id":"fixture","token":"fixture","command":"find","args":r#"days=7 query="two words""#
    }}))
    .unwrap();
    bot.process_event(event).await.unwrap();
}

#[tokio::test]
async fn slash_auto_ack_uses_original_callback_and_does_not_duplicate_a_reply() {
    let (url, mut requests, server) = support::mock(vec![
        support::response(200, json!({})),
        support::response(200, json!({})),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(url)
        .configure(|config| {
            config.execution.command_auto_ack_after = Some(std::time::Duration::from_millis(100))
        })
        .command("silent", "", |_| async { Ok(()) })
        .command("reply", "", |ctx| async move { ctx.reply("done").await })
        .build()
        .unwrap();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    for command in ["reply", "silent"] {
        let event = serde_json::from_value(json!({"id":1,"type":"command","data":{
            "id":command,"token":"fixture","command":command,"args":""
        }}))
        .unwrap();
        bot.process_event(event).await.unwrap();
    }
    let first = requests.recv().await.unwrap();
    let second = requests.recv().await.unwrap();
    assert!(first.starts_with("POST /interactions/reply/fixture/callback"));
    assert!(first.contains(r#""type":"reply""#));
    assert!(second.starts_with("POST /interactions/silent/fixture/callback"));
    assert!(second.contains(r#""type":"ack""#));
    server.await.unwrap();
}

#[tokio::test]
async fn ephemeral_update_uses_reply_id_and_rejects_layouts_before_io() {
    let (url, mut requests, server) = support::mock(vec![support::response(200, json!({}))]).await;
    let client = Client::builder("fixture").base_url(url).build();
    let event: InteractionEvent = serde_json::from_value(json!({
        "id":"click","token":"fixture","type":"button","reply_id":Uuid::nil()
    }))
    .unwrap();
    let layout = CreateMessage::layout([TextDisplay::new("Next")]);
    assert!(client
        .respond(&event, InteractionCallbackType::Update, &layout)
        .await
        .is_err());
    let body = CreateMessage::new("Next")
        .action_row(ActionRow::new([Button::primary("Continue", "next")]));
    client
        .respond(&event, InteractionCallbackType::Update, &body)
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert!(request.starts_with("POST /interactions/click/fixture/callback"));
    assert!(request.contains(r#""type":"update""#));
    server.await.unwrap();
}

#[tokio::test]
async fn threads_and_reaction_cleanup_use_documented_routes_and_receipts() {
    let channel = Uuid::from_u128(1);
    let thread_id = Uuid::from_u128(2);
    let message_id = Uuid::from_u128(3);
    let server_id = Uuid::from_u128(4);
    let thread = json!({
        "id":thread_id,"name":"Discussion","kind":"thread","parent_id":channel,
        "server_id":server_id,"starter_message_id":message_id,"locked":false,
        "created_at":"2026-11-15T00:00:00Z"
    });
    let (url, mut requests, server) = support::mock(vec![
        support::response(201,json!({
            "thread":thread,
            "message":{"id":message_id,"channel_id":thread_id,"author_id":Uuid::nil(),"created_at":"2026-11-15T00:00:00Z"}
        })),
        support::response(200,json!({"threads":[thread]})),
        support::response(200,json!({"ok":true})),
        support::response(200,json!({"ok":true,"removed":100,"more":true})),
        support::response(200,json!({"ok":true})),
    ]).await;
    let client = Client::builder("fixture").base_url(url).build();
    let created = client
        .create_thread(
            channel,
            None,
            Some(message_id),
            &CreateMessage::new("Welcome"),
        )
        .await
        .unwrap();
    assert_eq!(created.thread.id, thread_id);
    assert_eq!(created.message.channel_id, thread_id);
    assert_eq!(
        client.list_threads(channel, Some(3)).await.unwrap()[0].parent_id,
        channel
    );
    let mut edit = ThreadEdit::default();
    edit.archived = Some(true);
    client.edit_thread(thread_id, &edit).await.unwrap();
    let cleanup = client
        .moderate_reactions(thread_id, message_id, ReactionTarget::Emoji("🎉"))
        .await
        .unwrap();
    assert_eq!(cleanup.removed, 100);
    assert!(cleanup.more);
    client.delete_thread(thread_id).await.unwrap();
    for head in [
        format!("POST /channels/{channel}/threads"),
        format!("GET /channels/{channel}/threads?limit=3"),
        format!("PATCH /threads/{thread_id}"),
        format!(
            "DELETE /channels/{thread_id}/messages/{message_id}/reactions/%F0%9F%8E%89?all=true"
        ),
        format!("DELETE /threads/{thread_id}"),
    ] {
        let request = requests.recv().await.unwrap();
        assert!(request.starts_with(&format!("{head} ")), "{request}");
    }
    server.await.unwrap();
}

#[tokio::test]
async fn unexpected_slash_failure_notifies_privately_and_remains_incomplete() {
    let (url, mut requests, server) = support::mock(vec![support::response(200, json!({}))]).await;
    let bot = Bot::builder("fixture")
        .base_url(url)
        .command("broken", "", |_| async {
            Err(Error::Config("private diagnostic".into()))
        })
        .build()
        .unwrap();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let event = serde_json::from_value(json!({"id":1,"type":"command","data":{
        "id":"fixture","token":"fixture","command":"broken","args":""
    }}))
    .unwrap();
    assert!(matches!(
        bot.process_event(event).await,
        Err(Error::Config(_))
    ));
    let request = requests.recv().await.unwrap();
    assert!(request.contains(r#""type":"ephemeral""#));
    assert!(request.contains("Something went wrong running /broken."));
    assert!(!request.contains("private diagnostic"));
    server.await.unwrap();
}
