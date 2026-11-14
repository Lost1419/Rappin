mod support;
use rappin::{
    capabilities::capability, collectors::Collectible, DmMessageEvent, Event, MessageEvent,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn owned_schema_edits_do_not_change_canonical_validation_or_other_operations() {
    let capability = capability("send_message").unwrap();
    let args = json!({"channel":Uuid::nil(),"content":"hello"});
    capability.validate(&args).unwrap();
    let mut exported = capability.input_schema();
    exported["properties"]["channel"]["type"] = "boolean".into();
    let mut custom = capability.clone();
    custom
        .arguments
        .iter_mut()
        .find(|a| a.name == "channel")
        .unwrap()
        .schema = json!({"type":"boolean"});
    assert_eq!(
        custom.input_schema()["properties"]["channel"]["type"],
        "boolean"
    );
    // Validation is against trusted SDK metadata even for a caller-owned descriptor.
    custom.validate(&args).unwrap();
    assert!(custom
        .validate(&json!({"channel":true,"content":"hello"}))
        .is_err());
    assert_eq!(
        capability.input_schema()["properties"]["channel"]["type"],
        "string"
    );
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..16 {
                    capability.validate(&args).unwrap();
                    rappin::capabilities::capability("get_api_version")
                        .unwrap()
                        .validate(&json!({}))
                        .unwrap();
                    assert!(rappin::capabilities::capability("edit_channel")
                        .unwrap()
                        .validate(&json!({"id":Uuid::nil(),"body":{"slowmode":-1}}))
                        .is_err());
                }
            });
        }
    });
}

#[test]
fn typed_decoding_keeps_raw_payloads_reusable_and_dm_scope_validation_strict() {
    let event: Event = serde_json::from_value(json!({"id":1,"type":"message","data":{
        "id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),
        "content":"hello","embeds":[{"title":"report","fields":[{"name":"a","value":"b"}]}],
        "attachments":[{"path":"files/report.txt","name":"report.txt","size":42}],
        "future_field":{"nested":[1,2,3]}
    }}))
    .unwrap();
    let original = event.clone();
    let expected: MessageEvent = serde_json::from_value(event.data.clone()).unwrap();
    for _ in 0..2 {
        assert_eq!(MessageEvent::from_event(&event).unwrap(), expected);
    }
    assert_eq!(event, original);
    let mut dm = event;
    dm.kind = "dm_message".into();
    dm.data["command"] = "must_not_dispatch".into();
    let normalized = MessageEvent::from_event(&dm).unwrap();
    assert!(normalized.is_dm);
    assert!(normalized.command.is_none());
    assert_eq!(dm.data["command"], "must_not_dispatch");
    assert_eq!(DmMessageEvent::from_event(&dm).unwrap().0, normalized);
    dm.data["server_id"] = json!(Uuid::nil());
    assert!(DmMessageEvent::from_event(&dm).is_none());
    assert!(MessageEvent::from_event(&dm).is_none());
    assert!(dm.parse_data().is_err());
}

#[test]
fn schema_integer_bounds_remain_exact_across_signed_unsigned_and_float_values() {
    use rappin::capabilities::validate_schema;
    for boundary in [i64::MIN, -9_007_199_254_740_992, -1, 0, i64::MAX] {
        let minimum = json!({"type":"integer","minimum":boundary});
        let maximum = json!({"type":"integer","maximum":boundary});
        assert!(validate_schema(&minimum, &json!(boundary)).is_ok());
        assert!(validate_schema(&maximum, &json!(boundary)).is_ok());
        if let Some(before) = boundary.checked_sub(1) {
            assert!(validate_schema(&minimum, &json!(before)).is_err());
        }
        if let Some(after) = boundary.checked_add(1) {
            assert!(validate_schema(&maximum, &json!(after)).is_err());
        }
    }
    assert!(validate_schema(&json!({"maximum":i64::MAX}), &json!(u64::MAX)).is_err());
    assert!(validate_schema(&json!({"minimum":0u64}), &json!(-1)).is_err());
    assert!(validate_schema(&json!({"maximum":1.5}), &json!(2)).is_err());
    assert!(validate_schema(
        &json!({"minimum":9_223_372_036_854_775_808.0}),
        &json!(i64::MAX)
    )
    .is_err());
    assert!(validate_schema(
        &json!({"maximum":-9_007_199_254_740_992.0}),
        &json!(-9_007_199_254_740_991i64)
    )
    .is_err());
}

#[tokio::test]
async fn split_sends_retain_confirmed_receipts_and_never_resume_after_uncertain_failure() {
    for (status, outcome) in [(502, "unknown"), (503, "not_applied")] {
        let first = rappin::Message::default();
        let (base, mut requests, http) = support::mock(vec![
            support::response(201, json!({"message":first})),
            (
                status,
                vec![("Retry-After", "0.25")],
                json!({"error":"unknown"}).to_string(),
            ),
        ])
        .await;
        let client = rappin::Client::builder("unused-fixture")
            .base_url(base)
            .build();
        let error = client
            .send_long_message(Uuid::nil(), &"x".repeat(4001))
            .await
            .unwrap_err();
        match error {
            rappin::Error::PartialWrite {
                completed,
                failed_step,
                source,
            } => {
                assert_eq!(completed, vec![json!(first)]);
                assert_eq!(failed_step, 2);
                assert_eq!(source.status_code(), Some(status));
                assert_eq!(
                    source.retry_after(),
                    Some(std::time::Duration::from_millis(250))
                );
                assert_eq!(source.to_json()["outcome"], outcome);
            }
            other => panic!("unexpected error {other:?}"),
        }
        http.await.unwrap();
        assert!(requests
            .recv()
            .await
            .unwrap()
            .starts_with("POST /channels/"));
        assert!(requests
            .recv()
            .await
            .unwrap()
            .starts_with("POST /channels/"));
        assert!(requests.try_recv().is_err());
    }
}

#[test]
fn callback_patches_share_message_storage_and_keep_flat_nullable_wire_fields() {
    let callback = rappin::InteractionCallback {
        kind: rappin::InteractionCallbackType::Update,
        message: rappin::EditMessage::new()
            .clear_content()
            .clear_components(),
    };
    let value = serde_json::to_value(&callback).unwrap();
    assert_eq!(
        value,
        json!({"type":"update","content":null,"components":null})
    );
    let decoded: rappin::InteractionCallback = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, callback);
}

#[tokio::test]
async fn malformed_owned_response_envelopes_fail_without_panicking() {
    struct Malformed(serde_json::Value);
    #[async_trait::async_trait]
    impl rappin::Api for Malformed {
        async fn request(
            &self,
            _: reqwest::Method,
            _: &str,
            _: Option<serde_json::Value>,
        ) -> rappin::Result<serde_json::Value> {
            Ok(self.0.clone())
        }
    }
    use rappin::ApiExt;
    for value in [json!(null), json!("unexpected"), json!([]), json!({})] {
        let api = Malformed(value);
        assert!(api
            .send_message(Uuid::nil(), &rappin::CreateMessage::new("hello"))
            .await
            .is_err());
        assert!(api
            .edit_message(
                Uuid::nil(),
                Uuid::nil(),
                &rappin::EditMessage::new().content("updated")
            )
            .await
            .is_err());
    }
}
