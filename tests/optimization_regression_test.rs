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
