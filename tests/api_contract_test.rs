mod support;
use rappin::*;
use reqwest::Method;
use serde_json::json;
use support::*;
use uuid::Uuid;

#[tokio::test]
async fn timestamps_are_query_encoded_and_limits_validate() {
    let (base, mut requests, task) = mock(vec![response(200, json!({"messages":[]}))]).await;
    let client = Client::builder("secret").base_url(base).build();
    let time = "2026-10-26T12:34:56+00:00".parse().unwrap();
    client
        .get_messages(Uuid::nil(), Some(50), Some(time), None)
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert!(request.contains("before=2026-10-26T12%3A34%3A56%2B00%3A00"));
    assert!(client
        .get_messages(Uuid::nil(), Some(101), None, None)
        .await
        .is_err());
    task.await.unwrap();
}
#[tokio::test]
async fn retry_preserves_error_and_headerless_429_is_not_retried() {
    let (base, mut requests, task) = mock(vec![
        response(503, json!({"error":"busy"})),
        response(
            429,
            json!({"error":"interaction exhausted","retry_after":1}),
        ),
    ])
    .await;
    let client = Client::builder("secret").base_url(base).build();
    let err = client
        .request_json::<serde_json::Value, ()>(Method::POST, "/x", None)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), Some(503));
    assert!(err.to_string().contains("busy"));
    let err = client
        .request_json::<serde_json::Value, ()>(Method::POST, "/x", None)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), Some(429));
    assert!(err.retry_after().is_none());
    assert!(requests.recv().await.is_some());
    assert!(requests.recv().await.is_some());
    task.await.unwrap();
}
#[tokio::test]
async fn retry_after_zero_retries_and_invalid_delay_does_not_panic() {
    let (base, _requests, task) = mock(vec![
        (
            503,
            vec![("Retry-After", "0")],
            "{\"error\":\"busy\"}".into(),
        ),
        response(200, json!({"ok":true})),
        (
            429,
            vec![("Retry-After", "NaN")],
            "{\"error\":\"limit\"}".into(),
        ),
    ])
    .await;
    let client = Client::builder("secret").base_url(base).build();
    assert!(client
        .request_json::<serde_json::Value, ()>(Method::GET, "/x", None)
        .await
        .is_ok());
    assert!(client
        .request_json::<serde_json::Value, ()>(Method::GET, "/x", None)
        .await
        .unwrap_err()
        .retry_after()
        .is_none());
    assert!(YappinError::RateLimited { retry_after: -1.0 }
        .retry_after()
        .is_none());
    task.await.unwrap();
}
#[tokio::test]
async fn webhook_handles_204_and_requests_wait_for_message() {
    let message = json!({"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"content":"hi","created_at":"2026-10-26T00:00:00Z"});
    let (base, mut requests, task) =
        mock(vec![(204, vec![], String::new()), response(200, message)]).await;
    let hook = WebhookClient::new(format!("{base}/api/webhooks/id/token?wait=false"));
    hook.send_no_wait(&webhook::WebhookMessage {
        content: Some("hi".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    hook.send_content("hi").await.unwrap();
    assert!(requests.recv().await.unwrap().contains("wait=false"));
    assert!(requests.recv().await.unwrap().contains("wait=true"));
    task.await.unwrap();
}
#[tokio::test]
async fn oauth_uses_direct_me_and_public_pkce_exchange() {
    let (base, mut requests, task) = mock(vec![
        response(
            200,
            json!({"id":Uuid::nil(),"username":"alice","display_name":null,"avatar_url":null}),
        ),
        response(200, json!({"access_token":"yo_test","token_type":"Bearer"})),
    ])
    .await;
    let oauth = OAuth2Client::public("client", "http://localhost/callback").base_url(base);
    assert_eq!(
        oauth.get_user_profile("yo_test").await.unwrap().username,
        "alice"
    );
    let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").unwrap();
    assert_eq!(
        pkce.challenge(),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    assert!(oauth
        .authorization_url_pkce("state", &pkce)
        .contains("code_challenge_method=S256"));
    oauth.exchange_code_pkce("code", &pkce).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .starts_with("GET /api/oauth/me "));
    let exchange = requests.recv().await.unwrap();
    assert!(exchange.contains("code_verifier="));
    assert!(!exchange.contains("client_secret="));
    task.await.unwrap();
}
#[test]
fn patch_null_and_component_discriminators_survive_round_trips() {
    let callback: InteractionCallback =
        serde_json::from_value(json!({"type":"update","components":null,"content":null})).unwrap();
    let value = serde_json::to_value(callback).unwrap();
    assert!(value.get("components").unwrap().is_null());
    assert!(value.get("content").unwrap().is_null());
    assert!(value.get("embeds").is_none());
    let section:Component=serde_json::from_value(json!({"type":9,"components":[{"type":10,"content":"text"}],"accessory":{"type":2,"style":1,"label":"go","custom_id":"go"}})).unwrap();
    let value = serde_json::to_value(section).unwrap();
    assert_eq!(value["type"], 9);
    assert_eq!(value["accessory"]["custom_id"], "go");
    let edit: EditMessage = serde_json::from_value(json!({"embeds":null})).unwrap();
    assert_eq!(edit.embeds, Some(None));
}
#[test]
fn validation_and_unicode_splitting() {
    assert!(CreateMessage::new("🙂".repeat(2001)).validate().is_err());
    assert!(CreateMessage::new("hi")
        .action_row(ActionRow::new(vec![
            Button::primary("yes", "same"),
            Button::primary("no", "same")
        ]))
        .validate()
        .is_err());
    assert!(
        CreateMessage::layout(vec![Container::new().add(TextDisplay::new("hello"))], None)
            .validate()
            .is_ok()
    );
    let original = "🙂 hello\n".repeat(1000);
    let chunks = split_message(&original);
    assert_eq!(chunks.concat(), original);
    assert!(chunks.iter().all(|s| s.chars().count() <= 2000));
}
#[test]
fn authenticated_webhook_normalizes_event_field_and_rejects_tampering() {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let body = br#"{"id":10,"event":"test","data":{}}"#;
    let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
    mac.update(body);
    let signature = format!(
        "sha256={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        webhook::verify_event("secret", &signature, body)
            .unwrap()
            .kind,
        "test"
    );
    assert!(webhook::verify_event("secret", &signature, b"{}").is_err());
}
#[tokio::test]
async fn full_bot_profile_retains_absolute_urls() {
    let p: BotProfile = serde_json::from_value(profile()).unwrap();
    assert_eq!(
        p.avatar_url().as_deref(),
        Some("https://example.com/avatar.png")
    );
    assert!(p.bot.is_some());
}

#[tokio::test]
async fn bounded_member_pagination_uses_last_user_id_and_resolution_rejects_ambiguity() {
    let members: Vec<_> = (1..=500u128)
        .map(|id| json!({"id":Uuid::from_u128(id),"username":format!("user{id}")}))
        .collect();
    let channels = json!({"channels":[{"id":Uuid::from_u128(1),"name":"general","type":"text"},{"id":Uuid::from_u128(2),"name":"general","type":"text"}]});
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"members":members})),
        response(
            200,
            json!({"members":[{"id":Uuid::from_u128(501),"username":"last"}]}),
        ),
        response(200, channels),
    ])
    .await;
    let client = Client::builder("secret").base_url(base).build();
    assert_eq!(
        client
            .member_pages(Uuid::nil(), None, 2)
            .await
            .unwrap()
            .len(),
        501
    );
    assert!(requests.recv().await.unwrap().contains("limit=500"));
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains(&format!("after={}", Uuid::from_u128(500))));
    let error = client.resolve_channel(None, "general").await.unwrap_err();
    assert!(error.to_string().contains("ambiguous"));
    task.await.unwrap();
}

#[tokio::test]
async fn channel_webhooks_normalize_link_buttons_and_reject_callback_buttons_locally() {
    let (base, mut requests, task) = mock(vec![(204, vec![], String::new())]).await;
    let webhook = WebhookClient::new(format!("{base}/webhook"));
    let payload = WebhookMessage {
        content: Some("hi".into()),
        components: vec![serde_json::to_value(ActionRow::new(vec![Button::link(
            "Docs",
            "https://www.yapin.gg/developers",
        )]))
        .unwrap()],
        ..Default::default()
    };
    webhook.send_no_wait(&payload).await.unwrap();
    let request = requests.recv().await.unwrap();
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["components"][0]["type"], 1);
    assert_eq!(body["components"][0]["components"][0]["style"], 5);
    let invalid = WebhookMessage {
        content: Some("hi".into()),
        components: vec![serde_json::to_value(ActionRow::new(vec![Button::primary(
            "click", "callback",
        )]))
        .unwrap()],
        ..Default::default()
    };
    assert!(webhook
        .send_no_wait(&invalid)
        .await
        .unwrap_err()
        .to_string()
        .contains("link"));
    task.await.unwrap();
}
