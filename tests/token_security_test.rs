mod support;
use rappin::{Client, Event, InteractionEvent, OAuth2Token, VoiceTicket};
use reqwest::Method;
use serde_json::{json, Value};

#[tokio::test]
async fn redirects_are_not_followed_even_to_the_same_host() {
    use tokio::net::TcpListener;
    let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = sink.local_addr().unwrap();
    let location = format!("http://{address}/steal");
    // A process-local fixture string is used, never a real credential.
    let (base, _requests, task) = support::mock(vec![(
        307,
        vec![("Location", Box::leak(location.into_boxed_str()))],
        String::new(),
    )])
    .await;
    let client = Client::builder("yb_private_fixture").base_url(base).build();
    let err = client
        .request_json::<Value, ()>(Method::GET, "/me", None)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), Some(307));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), sink.accept())
            .await
            .is_err()
    );
    task.await.unwrap();
}
#[tokio::test]
async fn api_and_deserialization_errors_cannot_echo_the_bot_token() {
    let token = "yb_private_fixture";
    let (base, _requests, task) = support::mock(vec![
        support::response(400, json!({"error":format!("reflected {token}")})),
        support::response(200, json!({"kind":token})),
    ])
    .await;
    let client = Client::builder(token).base_url(base).build();
    let api = client
        .request_json::<Value, ()>(Method::GET, "/me", None)
        .await
        .unwrap_err();
    assert!(!format!("{api:?} {api}").contains(token));
    #[derive(serde::Deserialize)]
    struct Payload {
        #[allow(dead_code)]
        kind: rappin::ChannelKind,
    }
    let parsed = client
        .request_json::<Payload, ()>(Method::GET, "/me", None)
        .await;
    let err = match parsed {
        Ok(_) => panic!("expected invalid enum"),
        Err(e) => e,
    };
    assert!(!format!("{err:?} {err}").contains(token));
    task.await.unwrap();
}
#[test]
fn debugging_token_bearing_models_redacts_their_credentials() {
    let token = "private_bearer_fixture";
    let oauth: OAuth2Token =
        serde_json::from_value(json!({"access_token":token,"refresh_token":token})).unwrap();
    let voice:VoiceTicket=serde_json::from_value(json!({"url":"wss://voice.example","token":token,"channel_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil()})).unwrap();
    let event: InteractionEvent =
        serde_json::from_value(json!({"id":"click","token":token})).unwrap();
    let raw: Event =
        serde_json::from_value(json!({"id":1,"type":"interaction","data":{"token":token}}))
            .unwrap();
    for debug in [
        format!("{oauth:?}"),
        format!("{voice:?}"),
        format!("{event:?}"),
        format!("{raw:?}"),
    ] {
        assert!(!debug.contains(token), "{debug}");
    }
}

#[tokio::test]
async fn rejected_endpoint_and_invalid_cursor_errors_do_not_disclose_file_contents() {
    let client = Client::builder("yb_private_fixture")
        .base_url("http://example.com/api")
        .build();
    let error = client.get_me().await.unwrap_err();
    assert!(error.to_string().contains("TLS"));
    let file = std::env::temp_dir().join(format!("rappin-private-{}.json", uuid::Uuid::new_v4()));
    tokio::fs::write(&file, "\"some_other_credential\"")
        .await
        .unwrap();
    let error = rappin::EventCursor::new(&file).load().await.unwrap_err();
    assert!(!format!("{error:?} {error}").contains("some_other_credential"));
    tokio::fs::remove_file(file).await.unwrap();
}

#[test]
fn webhook_hmac_matches_an_independent_sha256_test_vector() {
    let body = br#"{"id":10,"event":"test","data":{}}"#;
    let signature = "sha256=574bb8c735986387e131d27f263436ff498efa22766cf2a9a31b43e4bb6ae66e";
    assert_eq!(
        rappin::webhook::verify_event("secret", signature, body)
            .unwrap()
            .id,
        10
    );
    assert!(rappin::webhook::verify_event("wrong-key", signature, body).is_err());
}
