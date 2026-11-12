mod support;
use rappin::{capabilities::*, *};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn computed(user: Uuid, channel: Option<Uuid>, permissions: Value) -> Value {
    json!({"permissions":{"object":"permissions","server_id":id(1),"user_id":user,
        "channel_id":channel,"owner":false,"administrator":false,"permissions":permissions}})
}
#[derive(Default)]
struct MockApi {
    calls: Mutex<Vec<(Method, String, Option<Value>)>>,
    deny_requester: AtomicBool,
    missing_permission: AtomicBool,
    wrong_scope: AtomicBool,
    uncertain_write: AtomicBool,
}
#[async_trait::async_trait]
impl Api for MockApi {
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.clone(), path.into(), body));
        if method != Method::GET {
            if self.uncertain_write.load(Ordering::SeqCst) {
                return Err(YappinError::Remote(RemoteError {
                    kind: "fixture_partial".into(),
                    message: "write may have completed".into(),
                    status: Some(504),
                    retry_after: Some(2.0),
                    details: [
                        ("outcome".into(), json!("unknown")),
                        ("receipts".into(), json!([{ "id":id(6) }])),
                    ]
                    .into(),
                }));
            }
            return Ok(json!({"channel":{"id":id(3),"server_id":id(1)},"roles":[]}));
        }
        if path == "/me" {
            return Ok(json!({"id":id(2)}));
        }
        if path == format!("/channels/{}", id(3)) {
            return Ok(
                json!({"channel":{"id":id(3),"server_id":id(1),"name":"general","kind":"text"}}),
            );
        }
        if path.ends_with("/permissions") || path.contains("/permissions/") {
            let parts: Vec<_> = path.split('/').collect();
            let channel = (parts[1] == "channels").then(|| id(3));
            let user = parts[4].parse().unwrap();
            let flags = if self.missing_permission.load(Ordering::SeqCst) {
                json!({"view_channel":true})
            } else {
                json!({"view_channel":true,"manage_channels":!(user==id(4) && self.deny_requester.load(Ordering::SeqCst)),"send_messages":true})
            };
            let mut response = computed(user, channel, flags);
            if self.wrong_scope.load(Ordering::SeqCst) {
                response["permissions"]["server_id"] = json!(id(99));
            }
            return Ok(response);
        }
        if path.contains("/resolve/") {
            return Ok(json!({"id":id(3),"object":"channel","server_id":id(1),"channel_id":id(3)}));
        }
        if path.ends_with("/overrides") {
            return Ok(json!({"overrides":[]}));
        }
        if path.ends_with("/roles") {
            return Ok(json!({"roles":[{"id":id(5),"name":"helper"}]}));
        }
        if path.contains("/roles/") {
            return Ok(json!({"role":{"id":id(5),"name":"helper"}}));
        }
        if path.ends_with("/channels") {
            return Ok(json!({"channels":[],"categories":[{"id":id(7),"name":"main"}]}));
        }
        panic!("unexpected mock request {method} {path}");
    }
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        _: Freshness,
    ) -> Result<PermissionSnapshot> {
        Ok(PermissionSnapshot {
            server: serde_json::from_value(json!({"id":server,"name":"fixture","owner_id":id(9)}))?,
            actor,
            member: Some(serde_json::from_value(
                json!({"id":actor,"username":"member","roles":[id(5)]}),
            )?),
            roles: serde_json::from_value(
                json!([{"id":id(5),"name":"staff","perms":{"manage_channels":true,"manage_roles":true}}]),
            )?,
        })
    }
}
impl MockApi {
    fn writes(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _, _)| method != Method::GET)
            .count()
    }
}

#[test]
fn catalogue_14_and_prior_gaps_have_matching_validation() {
    assert_eq!(bundled_schema()["info"]["version"], "1.4.0");
    for name in [
        "resolve_id",
        "get_member_permissions",
        "get_channel_member_permissions",
        "get_role",
        "reorder_roles",
        "get_channel_overrides",
        "set_channel_override",
        "delete_channel_override",
        "get_category_overrides",
        "set_category_override",
        "delete_category_override",
        "send_sticker",
    ] {
        let entry = capability(name).unwrap();
        assert_eq!(entry.cli[0], "platform");
        assert!(entry.availability.test_servers_only);
        assert!(entry.tool_definition()["inputSchema"].is_object());
    }
    for name in [
        "set_channel_permissions",
        "set_category_permissions",
        "set_channel_override",
        "set_category_override",
    ] {
        let mut args =
            json!({"id":id(1),"target":"everyone","body":{"allow":["use_activities"],"deny":[]}});
        if name.contains("category") {
            args["category"] = json!(id(7));
        }
        if name == "set_channel_override" {
            args.as_object_mut().unwrap().remove("id");
            args["channel"] = json!(id(3));
        }
        capability(name).unwrap().validate(&args).unwrap();
    }
    assert!(capability("reorder_roles")
        .unwrap()
        .validate(&json!({"id":id(1),"body":{"ids":[id(5),id(5)]}}))
        .is_err());
    let op = capability("send_sticker")
        .unwrap()
        .validate(&json!({"channel":id(3),"sticker":id(8)}))
        .unwrap();
    assert_eq!(
        op.api_request().unwrap().body.unwrap(),
        json!({"sticker_id":id(8)})
    );
}

#[test]
fn stickers_are_standalone_and_legacy_messages_still_validate() {
    let entry = capability("send_message").unwrap();
    entry
        .validate(&json!({"channel":id(3),"body":{"sticker_id":id(8)}}))
        .unwrap();
    for (key, value) in [
        ("content", json!("text")),
        ("embeds", json!([{"title":"x"}])),
        ("components", json!([{"type":1,"components":[]}])),
        ("flags", json!(32768)),
    ] {
        let mut body = json!({"sticker_id":id(8)});
        body[key] = value;
        assert!(entry
            .validate(&json!({"channel":id(3),"body":body}))
            .is_err());
    }
    let body: UiMessageBody = serde_json::from_value(json!({"sticker_id":id(8)})).unwrap();
    assert!(body.validate(false).is_err());
    CreateMessage::new("legacy text").validate().unwrap();
    assert!(capability("send_files").unwrap().validate(&json!({"channel":id(3),"body":{"sticker_id":id(8)},"files":[{"name":"a.txt","data":"eA=="}]})).is_err());
}

#[test]
fn computed_permissions_retain_missing_facts_and_scope() {
    let mut value =
        computed(id(4), Some(id(3)), json!({"view_channel":false}))["permissions"].clone();
    let access: MemberPermissions = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(access.permission("view_channel"), Some(false));
    assert_eq!(access.permission("manage_channels"), None);
    assert!(access.validate_scope(id(1), id(4), Some(id(99))).is_err());
    value["owner"] = json!(true);
    assert_eq!(
        serde_json::from_value::<MemberPermissions>(value.clone())
            .unwrap()
            .permission("future"),
        Some(true)
    );
    value.as_object_mut().unwrap().remove("channel_id");
    assert!(serde_json::from_value::<MemberPermissions>(value).is_err());
}

#[tokio::test]
async fn shared_resource_methods_validate_identity_and_avoid_bad_writes() {
    let api = MockApi::default();
    assert_eq!(api.resolve_id(id(3)).await.unwrap().object, "channel");
    assert!(api.resolve_id(id(99)).await.is_err());
    api.get_member_permissions(id(1), id(4), None)
        .await
        .unwrap();
    api.get_member_permissions(id(1), id(4), Some(id(3)))
        .await
        .unwrap();
    assert!(api
        .get_member_permissions(id(99), id(4), Some(id(3)))
        .await
        .is_err());
    assert_eq!(api.get_role(id(1), id(5)).await.unwrap().id, id(5));
    assert!(api.get_role(id(1), id(99)).await.is_err());
    assert!(api.reorder_roles(id(1), &[id(5), id(5)]).await.is_err());
    assert_eq!(api.writes(), 0);
    api.reorder_roles(id(1), &[id(5)]).await.unwrap();
    api.get_channel_overrides(id(3)).await.unwrap();
    api.get_category_overrides(id(7)).await.unwrap();
    api.set_channel_override(
        id(3),
        PermissionTarget::Everyone,
        &PermissionOverride::default(),
    )
    .await
    .unwrap();
    api.delete_channel_override(id(3), PermissionTarget::Everyone)
        .await
        .unwrap();
    api.set_category_override(
        id(7),
        PermissionTarget::Role(id(5)),
        &PermissionOverride::default(),
    )
    .await
    .unwrap();
    api.delete_category_override(id(7), PermissionTarget::Role(id(5)))
        .await
        .unwrap();
    assert_eq!(api.writes(), 5);
}

#[tokio::test]
async fn policy_rechecks_requester_and_bot_access_and_preserves_unknown_outcomes() {
    let api = MockApi::default();
    let policy = ComputedPermissionsPolicy::new(&api);
    let context = OperationContext::server(id(1), id(4));
    let operation = capability("edit_channel")
        .unwrap()
        .validate(&json!({"id":id(3),"body":{"name":"renamed"}}))
        .unwrap();
    let report = operation
        .assess_with_policy(&api, &context, &policy)
        .await
        .unwrap();
    assert!(report.allowed(), "{report:?}");
    for actor in [id(4), id(2)] {
        assert!(api
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, path, _)| path.ends_with(&format!("/permissions/{actor}"))));
    }
    api.deny_requester.store(true, Ordering::SeqCst);
    let error = operation
        .execute_with_policy(&api, &context, &policy)
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "operation_authorization");
    assert_eq!(api.writes(), 0);
    let denied = operation
        .assess_with_policy(&api, &context, &policy)
        .await
        .unwrap();
    assert_eq!(denied.decision(), AccessDecision::Denied);
    api.deny_requester.store(false, Ordering::SeqCst);
    api.missing_permission.store(true, Ordering::SeqCst);
    assert_eq!(
        operation
            .assess_with_policy(&api, &context, &policy)
            .await
            .unwrap()
            .decision(),
        AccessDecision::Unknown
    );
    api.missing_permission.store(false, Ordering::SeqCst);
    api.wrong_scope.store(true, Ordering::SeqCst);
    assert!(operation
        .execute_with_policy(&api, &context, &policy)
        .await
        .is_err());
    assert_eq!(api.writes(), 0);
    api.wrong_scope.store(false, Ordering::SeqCst);
    api.uncertain_write.store(true, Ordering::SeqCst);
    let failure = operation
        .execute_with_policy(&api, &context, &policy)
        .await
        .unwrap_err()
        .to_remote();
    assert_eq!(failure.status, Some(504));
    assert_eq!(failure.retry_after, Some(2.0));
    assert_eq!(failure.details["outcome"], "unknown");
    assert_eq!(failure.details["receipts"][0]["id"], id(6).to_string());
    assert_eq!(api.writes(), 1);
}

fn resource(kind: &str, event_id: u64) -> Event {
    let object = kind.split('_').next().unwrap();
    serde_json::from_value(json!({"id":event_id,"type":kind,"data":{"id":id(3),
        "server_id":id(1),"object":object,"type":if object=="thread" {"thread"} else {"voice"},"future":true}})).unwrap()
}
#[test]
fn lifecycle_events_keep_their_own_type_and_reject_inconsistent_scope() {
    for object in ["channel", "thread", "category"] {
        for action in ["create", "update", "delete"] {
            let event = resource(&format!("{object}_{action}"), 1);
            let EventData::Resource(parsed) = event.parse_data().unwrap() else {
                panic!("wrong event kind")
            };
            assert_eq!(parsed.data, event.data);
            assert_eq!(parsed.kind.as_str(), event.kind);
        }
    }
    let mut bad = resource("channel_update", 1);
    bad.data["object"] = json!("user");
    assert!(bad.parse_data().is_err());
    bad = resource("channel_update", 1);
    bad.data["channel_id"] = json!(id(99));
    assert!(bad.parse_data().is_err());
    bad.data["channel_id"] = json!(42);
    assert!(bad.parse_data().is_err());
}

#[test]
fn opt_in_ids_and_object_views_round_trip() {
    let user = UserId::from(id(4));
    assert_eq!(user.to_string(), id(4).to_string());
    assert_eq!(
        serde_json::from_value::<UserId>(json!(id(4))).unwrap(),
        user
    );
    let value = json!({"id":id(4),"object":"user","future":true});
    let tagged = ApiObject::<ResolvedObject>::decode(value.clone(), "user").unwrap();
    assert_eq!(tagged.data.id, id(4));
    assert!(ApiObject::<ResolvedObject>::decode(value, "channel").is_err());
}

#[tokio::test]
async fn computed_endpoint_rollout_errors_keep_structured_status() {
    let (base, _calls, http) = support::mock(vec![support::response(
        403,
        json!({"error":"This isn't on for this server yet"}),
    )])
    .await;
    let client = Client::builder("fixture")
        .base_url(base)
        .max_retries(0)
        .build();
    let error = client
        .get_member_permissions(id(1), id(4), Some(id(3)))
        .await
        .unwrap_err()
        .to_remote();
    assert_eq!(error.status, Some(403));
    http.await.unwrap();
}

struct ResourceObserver(std::sync::Arc<Mutex<Vec<String>>>);
#[async_trait::async_trait]
impl EventHandler for ResourceObserver {
    async fn on_resource_event(&self, _client: &Client, event: &ResourceEvent) -> Result<()> {
        self.0.lock().unwrap().push(event.kind.as_str().into());
        if event.kind == ResourceEventKind::ChannelUpdate {
            return Err(YappinError::Remote(RemoteError {
                kind: "resource_listener_failure".into(),
                message: "incomplete resource work".into(),
                status: Some(502),
                retry_after: Some(0.5),
                details: [("outcome".into(), json!("unknown"))].into(),
            }));
        }
        Ok(())
    }
}
#[tokio::test]
async fn resource_listener_failure_keeps_ordered_polling_checkpoint_and_cause() {
    let path = std::env::temp_dir().join(format!("rappin-resource-cursor-{}.json", Uuid::new_v4()));
    EventCursor::new(&path).save(1).await.unwrap();
    let events = [
        resource("channel_create", 2),
        resource("channel_update", 3),
        resource("channel_delete", 4),
    ];
    let (base, _calls, http) = support::mock(vec![
        support::response(200, support::profile()),
        support::response(200, json!({"events":events,"last":4})),
    ])
    .await;
    let seen = std::sync::Arc::new(Mutex::new(vec![]));
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&path)
        .event_handler(ResourceObserver(seen.clone()))
        .build();
    bot.cache()
        .update_channel(
            serde_json::from_value(
                json!({"id":id(3),"server_id":id(1),"kind":"text","name":"cached"}),
            )
            .unwrap(),
        )
        .await;
    let cache = bot.cache().clone();
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), bot.run())
        .await
        .unwrap()
        .unwrap_err()
        .to_remote();
    assert_eq!(error.status, Some(502));
    assert_eq!(
        error.details["report"]["failed_events"][0]["error"]["outcome"],
        "unknown"
    );
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(2));
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["channel_create", "channel_update"]
    );
    assert!(cache.get_channel(&id(3)).await.is_none());
    http.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn permanent_sticker_command_uses_metadata_for_parsing_and_help() {
    let binding: OperationCommandBinding = serde_json::from_value(json!({
        "operation":"send_sticker","name":"sticker","description":"Send a server sticker",
        "fixed_arguments":{"channel":id(3)},"parameters":[{"name":"sticker","target":"/sticker"}]
    }))
    .unwrap();
    let schema = binding.command_schema().unwrap();
    assert!(schema.info().usage().contains("sticker"));
    let operation = binding
        .prepare(&schema.parse(&id(8).to_string()).unwrap())
        .unwrap();
    assert_eq!(operation.arguments()["sticker"], id(8).to_string());
    assert!(schema.parse("invalid-id").is_err());

    let order: OperationCommandBinding = serde_json::from_value(json!({
        "operation":"reorder_roles","name":"roleorder","description":"Reorder roles",
        "fixed_arguments":{"id":id(1)},"parameters":[{"name":"ids","target":"/body/ids"}]
    }))
    .unwrap();
    let schema = order.command_schema().unwrap();
    let values = schema.parse(&format!("'[\"{}\"]'", id(5))).unwrap();
    assert_eq!(
        order.prepare(&values).unwrap().arguments()["body"]["ids"],
        json!([id(5)])
    );
    capability("reorder_roles")
        .unwrap()
        .validate(&json!({"id":id(1),"body":[{"id":id(5),"position":3}]}))
        .unwrap();
}

#[tokio::test]
async fn referenced_roles_categories_and_local_server_arguments_block_cross_server_work() {
    let api = MockApi::default();
    let context = OperationContext::server(id(1), id(4)).test_server_access(true);
    for (name, args) in [
        ("reorder_roles", json!({"id":id(1),"body":{"ids":[id(99)]}})),
        (
            "get_category_overrides",
            json!({"id":id(1),"category":id(99)}),
        ),
        ("get_member_permissions", json!({"id":id(99),"user":id(4)})),
    ] {
        let operation = capability(name).unwrap().validate(&args).unwrap();
        assert!(operation.execute(&api, &context).await.is_err());
    }
    assert_eq!(api.writes(), 0);
}

#[test]
fn emoji_decoded_size_accepts_two_mib_and_rejects_one_byte_more() {
    use base64::Engine;
    for (size, valid) in [(1_048_577, true), (2_097_152, true), (2_097_153, false)] {
        let image = base64::engine::general_purpose::STANDARD.encode(vec![0; size]);
        assert_eq!(CreateEmoji::new("test", image).validate().is_ok(), valid);
    }
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn resource_events_keep_duplicate_and_failure_barriers_over_gateway() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let websocket = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ws.send(Message::Text(
            json!({"op":"hello","heartbeat_interval":100})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let _identify = ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(
            json!({"op":"ready","seq":1}).to_string().into(),
        ))
        .await
        .unwrap();
        for event in [
            resource("channel_create", 2),
            resource("channel_create", 2),
            resource("channel_update", 3),
            resource("channel_delete", 4),
        ] {
            if ws
                .send(Message::Text(
                    json!({"op":"event","s":event.id,"d":event})
                        .to_string()
                        .into(),
                ))
                .await
                .is_err()
            {
                return;
            }
        }
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
            if message.to_text().is_ok_and(|s| s.contains("heartbeat")) {
                let _ = ws
                    .send(Message::Text(
                        json!({"op":"heartbeat_ack"}).to_string().into(),
                    ))
                    .await;
            }
        }
    });
    let path =
        std::env::temp_dir().join(format!("rappin-resource-gateway-{}.json", Uuid::new_v4()));
    EventCursor::new(&path).save(1).await.unwrap();
    let (base, _calls, http) =
        support::mock(vec![support::response(200, support::profile())]).await;
    let seen = std::sync::Arc::new(Mutex::new(vec![]));
    let bot = Bot::builder("fixture")
        .base_url(base)
        .gateway_only(url)
        .sync_commands(false)
        .log(false)
        .cursor_file(&path)
        .event_handler(ResourceObserver(seen.clone()))
        .build();
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), bot.run())
        .await
        .unwrap()
        .unwrap_err();
    let YappinError::EventProcessing { report } = error else {
        panic!("expected checkpoint report")
    };
    assert_eq!(report.failed_events[0].error.details["outcome"], "unknown");
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(2));
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["channel_create", "channel_update"]
    );
    http.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), websocket)
        .await
        .unwrap()
        .unwrap();
    std::fs::remove_file(path).unwrap();
}
