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
    let body = UiMessageBody::new("hello");
    assert!(NonceFixture
        .send_with_nonce(Uuid::nil(), &body, "bad")
        .await
        .is_err());
    let result = NonceFixture
        .send_with_nonce(Uuid::nil(), &body, NONCE)
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
    let command = SlashCommand::new(
        Command::new("ban", ""),
        vec![user, SlashOption::new("reason", SlashOptionKind::String)],
    );
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
fn metadata_and_new_events_decode_without_reclassifying_existing_dispatch() {
    let packages: SdkPackages = serde_json::from_value(json!({"npm":null,"pypi":null})).unwrap();
    assert!(packages.npm.is_empty());
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
        EventData::Unknown { .. }
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
async fn structured_errors_are_opt_in_and_preserve_the_platform_code() {
    let error = support::response(
        403,
        json!({
            "error":"Feature unavailable",
            "code":"feature_off",
            "permission":"manage_messages"
        }),
    );
    let (url, _requests, server) = support::mock(vec![error.clone(), error]).await;
    let legacy = Client::builder("example").base_url(&url).build();
    assert!(matches!(
        legacy.list_threads(Uuid::nil(), None).await,
        Err(YappinError::Api { status: 403, .. })
    ));
    let client = Client::builder("example")
        .base_url(url)
        .structured_api_errors(true)
        .build();
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
        UiInteraction::from_event(&event).unwrap().reply_id,
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
        .command("find", "Find items", |_| async { Ok(()) })
        .slash_options("find", [SlashOption::new("query", SlashOptionKind::String)])
        .build();
    bot.handle().sync_commands().await.unwrap();
    let request = requests.recv().await.unwrap();
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["commands"][0]["options"][0]["name"], "query");
    assert!(body.get("interactions").is_none());
    server.await.unwrap();
}
