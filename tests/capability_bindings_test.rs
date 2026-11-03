mod support;
use async_trait::async_trait;
use rappin::{capabilities::*, *};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use uuid::Uuid;

fn server() -> Uuid {
    Uuid::from_u128(1)
}
fn requester() -> Uuid {
    Uuid::from_u128(2)
}
fn bot_id() -> Uuid {
    Uuid::from_u128(3)
}
fn channel() -> Uuid {
    Uuid::from_u128(4)
}
fn role() -> Uuid {
    Uuid::from_u128(5)
}
fn context() -> OperationContext {
    OperationContext::server(server(), requester())
}
fn operation(name: &str, args: Value) -> ValidatedOperation {
    capability(name).unwrap().validate(&args).unwrap()
}

#[derive(Default)]
struct MockApi {
    requests: Mutex<Vec<(Method, String, Option<Value>)>>,
    principals: Mutex<Vec<(Uuid, Freshness)>>,
    request_grants: Vec<&'static str>,
    bot_grants: Vec<&'static str>,
    channel_server: Option<Uuid>,
    role_ids: Vec<Uuid>,
    category_ids: Vec<Uuid>,
    reply_channel: Option<Uuid>,
    mutation_error: Option<u16>,
    resource_server: Option<Uuid>,
    webhook_owner: Option<Option<Uuid>>,
    fail_step: usize,
    mutations: AtomicUsize,
}
impl MockApi {
    fn standard() -> Self {
        Self {
            channel_server: Some(server()),
            role_ids: vec![role()],
            category_ids: vec![Uuid::from_u128(6)],
            reply_channel: Some(channel()),
            ..Default::default()
        }
    }
}
#[async_trait]
impl Api for MockApi {
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.requests
            .lock()
            .unwrap()
            .push((method.clone(), path.into(), body.clone()));
        if method != Method::GET {
            let step = self.mutations.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(status) = self.mutation_error {
                if self.fail_step == 0 || step == self.fail_step {
                    return Err(YappinError::Api {
                        status,
                        message: "fixture failure".into(),
                        retry_after: None,
                    });
                }
            }
            return Ok(
                json!({"message":{"id":Uuid::from_u128(step as u128),"content":"confirmed"},"ok":true}),
            );
        }
        if path == "/me" {
            return Ok(json!({"id":bot_id()}));
        }
        if path.contains("/messages/") {
            let id = path.rsplit('/').next().unwrap();
            return Ok(
                json!({"message":{"id":id,"channel_id":self.reply_channel,"author_id":bot_id()}}),
            );
        }
        if path.contains("/members/") {
            return Ok(
                json!({"member":{"id":path.rsplit('/').next().unwrap(),"username":"member","invite_code":"private","inviter_id":Uuid::from_u128(99)}}),
            );
        }
        if path.ends_with("/emojis") {
            return Ok(
                json!({"emojis":[{"id":role(),"server_id":self.resource_server.unwrap_or(server())}],"limit":50}),
            );
        }
        if path.ends_with("/webhooks") && path.starts_with("/channels/") {
            return Ok(
                json!({"webhooks":[{"id":role(),"server_id":self.resource_server.unwrap_or(server()),"channel_id":channel(),"bot_id":self.webhook_owner.unwrap_or(Some(bot_id()))}]}),
            );
        }
        if path == "/webhooks" {
            return Ok(
                json!({"webhooks":[{"id":role(),"server_id":server()},{"id":Uuid::from_u128(987),"server_id":Uuid::from_u128(999)}]}),
            );
        }
        if path.starts_with("/channels/") {
            let id = path.split('/').nth(2).unwrap();
            return Ok(json!({"channel":{"id":id,"server_id":self.channel_server,"kind":"voice"}}));
        }
        if path.ends_with("/roles") {
            return Ok(
                json!({"roles":self.role_ids.iter().map(|id|json!({"id":id,"name":"role"})).collect::<Vec<_>>()}),
            );
        }
        if path.ends_with("/channels") {
            return Ok(
                json!({"channels":[],"categories":self.category_ids.iter().map(|id|json!({"id":id})).collect::<Vec<_>>()}),
            );
        }
        if path.ends_with("/invites") {
            return Ok(json!({"invites":[]}));
        }
        if path == format!("/servers/{}", server()) {
            return Ok(
                json!({"server":{"id":server(),"name":"host","owner_id":Uuid::from_u128(99)}}),
            );
        }
        panic!("unexpected mock request {method} {path}")
    }
    async fn permission_snapshot(
        &self,
        id: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<PermissionSnapshot> {
        self.principals.lock().unwrap().push((actor, freshness));
        let grants = if actor == requester() {
            &self.request_grants
        } else {
            &self.bot_grants
        };
        let permissions: serde_json::Map<String, Value> = grants
            .iter()
            .map(|key| ((*key).into(), true.into()))
            .collect();
        Ok(PermissionSnapshot {
            server: serde_json::from_value(
                json!({"id":id,"name":"host","owner_id":Uuid::from_u128(99)}),
            )?,
            actor,
            member: Some(serde_json::from_value(
                json!({"id":actor,"username":"member","roles":[role()]}),
            )?),
            roles: serde_json::from_value(
                json!([{"id":role(),"name":"granted","permissions":permissions}]),
            )?,
        })
    }
}
struct HostEvidence {
    requester: AccessDecision,
    bot: AccessDecision,
}
impl HostEvidence {
    fn allow() -> Self {
        Self {
            requester: AccessDecision::Allowed,
            bot: AccessDecision::Allowed,
        }
    }
}
#[async_trait]
impl OperationPolicy for HostEvidence {
    async fn check(&self, q: &AccessQuestion<'_>) -> Result<CheckResolution> {
        Ok(CheckResolution {
            decision: match q.principal {
                Principal::Requester => self.requester,
                Principal::Bot => self.bot,
                Principal::Context => AccessDecision::Allowed,
            },
            reason: "fixture host evidence, separate from model arguments".into(),
        })
    }
}

#[test]
fn catalogue_is_offline_filtered_complete_and_has_one_schema_source() {
    assert_eq!(catalogue().len(), 56);
    let names = catalogue()
        .iter()
        .map(|c| c.name)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(names.len(), 56);
    for descriptor in catalogue() {
        let tool = descriptor.tool_definition();
        assert_eq!(tool["inputSchema"], descriptor.input_schema());
        assert_eq!(
            tool["x-rappin-permissions"],
            serde_json::to_value(&descriptor.permissions).unwrap()
        );
        assert_eq!(
            tool["x-rappin-availability"],
            serde_json::to_value(descriptor.availability).unwrap()
        );
        assert!(!descriptor.cli.is_empty());
        if let Some(body) = descriptor.arguments.iter().find(|f| f.name == "body") {
            assert_eq!(body.schema, descriptor.input_schema()["properties"]["body"]);
        }
    }
    let filter = CapabilityFilter {
        domains: vec![CapabilityDomain::Messages],
        write: Some(false),
        library_only: true,
        include_test_servers: false,
        ..Default::default()
    };
    let selected = filtered_catalogue(&filter);
    assert!(filter
        .validate(
            "send_message",
            &json!({"channel":channel(),"content":"not selected"})
        )
        .is_err());
    assert!(!selected.is_empty());
    assert!(selected
        .iter()
        .all(|c| !c.write && c.domain == CapabilityDomain::Messages));
    let filter = CapabilityFilter {
        names: vec!["create_role".into()],
        include_test_servers: false,
        ..Default::default()
    };
    assert!(filtered_catalogue(&filter).is_empty());
    assert!(
        capability("create_role")
            .unwrap()
            .availability
            .test_servers_only
    );
    assert_eq!(
        capability("ack_events").unwrap().availability.library,
        LibrarySupport::CliRuntime
    );
}

#[tokio::test]
async fn ordinary_member_reads_do_not_disclose_bot_only_invite_attribution() {
    let api = MockApi {
        bot_grants: vec!["administrator"],
        ..MockApi::standard()
    };
    let call = operation("get_member", json!({"server":server(),"user":requester()}));
    let result = call.execute(&api, &context()).await.unwrap();
    assert_eq!(result.data["member"]["id"], requester().to_string());
    assert!(result.data["member"].get("invite_code").is_none());
    assert!(result.data["member"].get("inviter_id").is_none());
}

#[test]
fn typed_validation_rejects_bad_payloads_without_auth_or_network() {
    let error = capability("edit_role")
        .unwrap()
        .validate(&json!({"id":server(),"role":role(),"body":{"position":i64::MAX}}))
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "validation");
    assert_eq!(error.to_json()["outcome"], "rejected");
    for args in [
        json!({"channel":channel(),"content":"x".repeat(2001)}),
        json!({"channel":channel(),"content":"x","body":{"content":"y"}}),
        json!({"channel":channel(),"body_file":"/private/secret"}),
        json!({"channel":"https://untrusted.invalid","content":"x"}),
        json!({"channel":channel(),"body":{"content":"x","unknown":true}}),
    ] {
        assert!(capability("send_message").unwrap().validate(&args).is_err());
    }
    let duplicate = json!({"channel":channel(),"body":{"content":"x","components":[{"type":1,"components":[{"type":2,"style":1,"label":"a","custom_id":"same"},{"type":2,"style":1,"label":"b","custom_id":"same"}]}]}});
    assert!(capability("send_message")
        .unwrap()
        .validate(&duplicate)
        .is_err());
    assert!(capability("read_messages")
        .unwrap()
        .validate(&json!({"channel":channel(),"before":"bad"}))
        .is_err());
    assert!(capability("read_messages").unwrap().validate(&json!({"channel":channel(),"before":"2026-10-31T00:00:00Z","after":"2026-10-31T00:00:00Z"})).is_err());
    assert!(capability("set_channel_permissions")
        .unwrap()
        .validate(&json!({"id":channel(),"target":"../../me","body":{}}))
        .is_err());
    assert!(capability("poll_events")
        .unwrap()
        .validate(&json!({"after":u64::MAX}))
        .is_ok());
    assert!(capability("poll_events")
        .unwrap()
        .validate(&json!({"after":-1}))
        .is_err());
    let edit = operation(
        "edit_role",
        json!({"id":server(),"role":role(),"body":{"mentionable":false,"permissions":{"manage_messages":false}}}),
    );
    assert_eq!(
        edit.arguments()["body"]["permissions"]["manage_messages"],
        false
    );
    let clear = operation(
        "edit_message",
        json!({"channel":channel(),"message":Uuid::nil(),"body":{"components":null}}),
    );
    assert!(clear.arguments()["body"]["components"].is_null());
}

#[tokio::test]
async fn preview_and_local_metadata_need_no_api_access() {
    let api = MockApi::standard();
    let call = operation(
        "send_message",
        json!({"channel":channel(),"content":"preview","dry_run":true}),
    );
    let result = call.execute(&api, &context()).await.unwrap();
    assert!(result.dry_run);
    assert!(result.authorization.is_none());
    assert!(api.requests.lock().unwrap().is_empty());
    let call = operation("get_capabilities", json!({}));
    let result = call.execute(&api, &context()).await.unwrap();
    assert_eq!(result.data["tools"].as_array().unwrap().len(), 56);
    assert!(api.requests.lock().unwrap().is_empty());
    for name in ["ack_events", "poll_events", "doctor"] {
        let args = if name == "ack_events" {
            json!({"id":1})
        } else {
            json!({})
        };
        let call = operation(name, args);
        let error = call.execute(&api, &context()).await.unwrap_err();
        assert_eq!(error.to_json()["decision"], "unknown");
    }
    assert!(api.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn requester_and_bot_server_permissions_are_independent() {
    let api = MockApi {
        bot_grants: vec!["administrator"],
        ..MockApi::standard()
    };
    let call = operation("list_invites", json!({"id":server()}));
    let report = call
        .assess_with_policy(
            &api,
            &context().test_server_access(true),
            &HostEvidence::allow(),
        )
        .await
        .unwrap();
    assert_eq!(report.decision(), AccessDecision::Denied);
    assert!(report.checks.iter().any(|c| c.check == "server_permissions"
        && c.principal == Principal::Requester
        && c.decision == AccessDecision::Denied));
    assert!(report.checks.iter().any(|c| c.check == "server_permissions"
        && c.principal == Principal::Bot
        && c.decision == AccessDecision::Allowed));
    assert_eq!(
        api.principals.lock().unwrap().as_slice(),
        [
            (requester(), Freshness::Refresh),
            (bot_id(), Freshness::Refresh)
        ]
    );
    assert!(call
        .execute_with_policy(
            &api,
            &context().test_server_access(true),
            &HostEvidence::allow()
        )
        .await
        .is_err());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unestablished_channel_checks_are_reported_and_never_assumed() {
    let api = MockApi::standard();
    let call = operation(
        "send_message",
        json!({"channel":channel(),"content":"hello"}),
    );
    let report = call.assess(&api, &context()).await.unwrap();
    assert_eq!(report.decision(), AccessDecision::Unknown);
    assert!(report
        .checks
        .iter()
        .any(|c| c.principal == Principal::Requester
            && c.check == "channel_permissions"
            && c.decision == AccessDecision::Unknown));
    assert!(call.execute(&api, &context()).await.is_err());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    let policy = HostEvidence {
        requester: AccessDecision::Allowed,
        bot: AccessDecision::Denied,
    };
    assert!(call
        .execute_with_policy(&api, &context(), &policy)
        .await
        .unwrap_err()
        .is_forbidden());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    let result = call
        .execute_with_policy(&api, &context(), &HostEvidence::allow())
        .await
        .unwrap();
    assert!(result.authorization.unwrap().allowed());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn model_arguments_cannot_escape_server_channel_role_category_or_reply_scope() {
    let api = MockApi::standard();
    let foreign = Uuid::from_u128(999);
    let call = operation("list_roles", json!({"id":foreign}));
    assert!(call
        .execute(&api, &context())
        .await
        .unwrap_err()
        .is_forbidden());
    assert!(api.requests.lock().unwrap().is_empty());
    let api = MockApi {
        channel_server: Some(foreign),
        ..MockApi::standard()
    };
    let call = operation("send_message", json!({"channel":channel(),"content":"x"}));
    assert!(call
        .execute_with_policy(&api, &context(), &HostEvidence::allow())
        .await
        .unwrap_err()
        .is_forbidden());
    assert_eq!(api.requests.lock().unwrap().len(), 1);
    let api = MockApi::standard();
    for (name, args) in [
        ("delete_role", json!({"id":server(),"role":foreign})),
        (
            "clear_category_permissions",
            json!({"id":server(),"category":foreign,"target":"everyone"}),
        ),
    ] {
        assert!(operation(name, args)
            .execute_with_policy(
                &api,
                &context().test_server_access(true),
                &HostEvidence::allow()
            )
            .await
            .unwrap_err()
            .is_forbidden());
    }
    let api = MockApi {
        reply_channel: Some(foreign),
        ..MockApi::standard()
    };
    let call = operation(
        "send_message",
        json!({"channel":channel(),"content":"x","reply_to":foreign}),
    );
    assert!(call
        .execute_with_policy(&api, &context(), &HostEvidence::allow())
        .await
        .unwrap_err()
        .is_forbidden());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    let call = operation(
        "send_files",
        json!({"channel":channel(),"body":{"reply_to":foreign},"files":[{"name":"a.txt","data":"eA=="}]}),
    );
    let report = call
        .assess_with_policy(
            &api,
            &context().test_server_access(true),
            &HostEvidence::allow(),
        )
        .await
        .unwrap();
    assert!(report
        .checks
        .iter()
        .any(|check| check.check == "reply_scope" && check.decision == AccessDecision::Denied));
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_server_availability_is_unknown_until_host_evidence_exists() {
    let api = MockApi::standard();
    let call = operation("get_member_counts", json!({"id":server()}));
    let report = call.assess(&api, &context()).await.unwrap();
    assert_eq!(report.decision(), AccessDecision::Unknown);
    assert!(report
        .checks
        .iter()
        .any(|c| c.check == "test_server_availability" && c.decision == AccessDecision::Unknown));
    assert_eq!(
        call.assess(&api, &context().test_server_access(false))
            .await
            .unwrap()
            .decision(),
        AccessDecision::Denied
    );
}

#[tokio::test]
async fn verified_event_server_scope_does_not_imply_requester_destination_access() {
    let api = MockApi {
        request_grants: vec!["manage_events"],
        bot_grants: vec!["manage_events"],
        ..MockApi::standard()
    };
    let call = operation(
        "create_server_event",
        json!({"id":server(),"body":{"title":"Meeting","starts_at":"2027-01-01T12:00:00Z","channel_id":channel()}}),
    );
    let context = context().test_server_access(true);
    let report = call.assess(&api, &context).await.unwrap();
    assert_eq!(report.decision(), AccessDecision::Unknown);
    for principal in [Principal::Requester, Principal::Bot] {
        assert!(report.checks.iter().any(|c| c.principal == principal
            && c.check == "event_destination_scope"
            && c.decision == AccessDecision::Unknown));
    }
    assert!(call.execute(&api, &context).await.is_err());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    assert!(call
        .execute_with_policy(&api, &context, &HostEvidence::allow())
        .await
        .unwrap()
        .authorization
        .unwrap()
        .allowed());
    assert_eq!(api.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mutations_are_not_replayed_and_partial_send_receipts_survive() {
    let api = MockApi {
        mutation_error: Some(503),
        ..MockApi::standard()
    };
    let call = operation(
        "send_message",
        json!({"channel":channel(),"content":"hello"}),
    );
    let error = call
        .execute_with_policy(&api, &context(), &HostEvidence::allow())
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(error.to_json()["retry_safety"], "unknown_write_outcome");
    assert_eq!(api.mutations.load(Ordering::SeqCst), 1);
    let api = MockApi {
        mutation_error: Some(502),
        fail_step: 2,
        ..MockApi::standard()
    };
    let call = operation(
        "send_message",
        json!({"channel":channel(),"content":"x".repeat(4001),"split":true}),
    );
    let error = call
        .execute_with_policy(&api, &context(), &HostEvidence::allow())
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "partial_write");
    assert_eq!(error.to_json()["failed_step"], 2);
    assert_eq!(
        error.to_json()["completed_steps"].as_array().unwrap().len(),
        1
    );
    assert_eq!(api.mutations.load(Ordering::SeqCst), 2);
}

fn binding() -> OperationCommandBinding {
    serde_json::from_value(json!({"operation":"send_message","name":"announce","description":"Post an announcement","aliases":["ann"],"fixed_arguments":{"channel":channel()},"parameters":[{"name":"text","target":"/content","rest":true}]})).unwrap()
}
#[test]
fn permanent_command_schema_and_payload_come_from_the_same_metadata() {
    let binding = binding();
    let schema = binding.command_schema().unwrap();
    assert_eq!(schema.info().aliases, vec!["ann"]);
    assert_eq!(schema.info().arguments[0].name, "text");
    let values = schema.parse("'Hello Rust' and friends").unwrap();
    let call = binding.prepare(&values).unwrap();
    assert_eq!(call.arguments()["content"], "Hello Rust and friends");
    assert_eq!(call.arguments()["channel"], channel().to_string());
    assert!(binding
        .prepare(&[("text".into(), "x".repeat(2001).into())].into())
        .is_err());
    let mut invalid = binding.clone();
    invalid.parameters[0].target = "/channel".into();
    assert!(invalid.validate().is_err());
    invalid = binding.clone();
    invalid.parameters[0].target = "/unknown".into();
    assert!(invalid.validate().is_err());
    let serialized = serde_json::to_string(&binding).unwrap();
    assert_eq!(
        serde_json::from_str::<OperationCommandBinding>(&serialized)
            .unwrap()
            .command_schema()
            .unwrap()
            .info(),
        schema.info()
    );
    let role_binding:OperationCommandBinding=serde_json::from_value(json!({"operation":"create_role","name":"role","description":"Create a role","fixed_arguments":{"id":server()},"parameters":[{"name":"name","target":"/body/name"}]})).unwrap();
    let values = role_binding
        .command_schema()
        .unwrap()
        .parse("Helper")
        .unwrap();
    assert_eq!(
        role_binding.prepare(&values).unwrap().arguments()["body"]["name"],
        "Helper"
    );
    let json_binding:OperationCommandBinding=serde_json::from_value(json!({"operation":"edit_role","name":"editrole","description":"Edit","fixed_arguments":{"id":server(),"role":role()},"parameters":[{"name":"changes","target":"/body"}]})).unwrap();
    let values = json_binding
        .command_schema()
        .unwrap()
        .parse("'{\"mentionable\":false}'")
        .unwrap();
    assert_eq!(
        json_binding.prepare(&values).unwrap().arguments()["body"]["mentionable"],
        false
    );
}

#[tokio::test]
async fn permanent_commands_preflight_reload_and_keep_bound_help_without_info_patching() {
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .prefix("?")
        .build();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let binding = binding();
    binding
        .preflight(&bot.handle(), Some("permanent"))
        .await
        .unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let observed = calls.clone();
    let command = binding
        .clone()
        .bind(move |_, call| {
            let observed = observed.clone();
            async move {
                observed.lock().unwrap().push(call.arguments().clone());
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(command.info, binding.command_schema().unwrap().info());
    bot.handle()
        .add_cog(
            CogBuilder::new("permanent")
                .registered_command(command)
                .build(),
        )
        .await
        .unwrap();
    assert!(binding.preflight(&bot.handle(), None).await.is_err());
    assert!(bot.handle().help("announce").await.contains("<text>"));
    let event = |content: &str| {
        serde_json::from_value(json!({"id":1,"type":"message","data":{"id":Uuid::nil(),"channel_id":channel(),"author_id":requester(),"server_id":server(),"content":content}})).unwrap()
    };
    bot.process_event(event("?ann first")).await.unwrap();
    assert_eq!(calls.lock().unwrap()[0]["content"], "first");
    let mut replacement = binding.clone();
    replacement.aliases = vec!["notice".into()];
    let observed = calls.clone();
    let command = replacement
        .bind(move |_, call| {
            let observed = observed.clone();
            async move {
                observed.lock().unwrap().push(call.arguments().clone());
                Ok(())
            }
        })
        .unwrap();
    bot.handle()
        .reload_cog(
            CogBuilder::new("permanent")
                .registered_command(command)
                .build(),
        )
        .await
        .unwrap();
    bot.process_event(event("?ann ignored")).await.unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
    bot.process_event(event("?notice second")).await.unwrap();
    assert_eq!(calls.lock().unwrap()[1]["content"], "second");
}

#[tokio::test]
async fn bound_execute_futures_are_usable_in_native_handlers() {
    let b:OperationCommandBinding=serde_json::from_value(json!({"operation":"list_roles","name":"roles","description":"Roles","fixed_arguments":{"id":server()}})).unwrap();
    let _command = b
        .bind(|ctx, call| async move {
            let scope = OperationContext::server(ctx.server_id().unwrap(), ctx.author_id());
            call.execute(&ctx, &scope).await?;
            Ok(())
        })
        .unwrap();
}

struct PublicationGate {
    ready: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl EventHandler for PublicationGate {
    async fn on_ready(&self, _: &Client, _: &BotProfile) {
        self.ready.notify_one();
        self.resume.notified().await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_publication_preserves_permanent_command_routes_and_help() {
    use support::{response, routed_mock};
    let (base, mut requests, http) = routed_mock(vec![
        ("GET /me".into(), vec![response(200, support::profile())]),
        (
            "PUT /commands".into(),
            vec![
                response(
                    200,
                    json!({"commands":[{"name":"persist","description":"Old"}]}),
                ),
                response(503, json!({"error":"publication failed"})),
            ],
        ),
        (
            "GET /events".into(),
            vec![
                response(200, json!({"events":[],"last":1})),
                response(
                    200,
                    json!({"events":[support::event(2,"persist")],"last":2}),
                ),
            ],
        ),
    ])
    .await;
    let ready = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let bot = Bot::builder("fixture")
        .base_url(base)
        .log(false)
        .event_handler(PublicationGate {
            ready: ready.clone(),
            resume: resume.clone(),
        })
        .build();
    let handle = bot.handle();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let old: OperationCommandBinding = serde_json::from_value(
        json!({"operation":"get_capabilities","name":"persist","description":"Old"}),
    )
    .unwrap();
    let command = old
        .clone()
        .bind(move |ctx, _| {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                ctx.bot().unwrap().stop();
                Ok(())
            }
        })
        .unwrap();
    handle
        .add_cog(
            CogBuilder::new("permanent")
                .registered_command(command)
                .build(),
        )
        .await
        .unwrap();
    let runner = tokio::spawn(bot.run());
    tokio::time::timeout(std::time::Duration::from_secs(5), ready.notified())
        .await
        .unwrap();
    let mut new = old.clone();
    new.description = "New".into();
    new.aliases = vec!["newalias".into()];
    new.preflight(&handle, Some("permanent")).await.unwrap();
    let command = new
        .bind(|_, _| async { panic!("failed candidate was published") })
        .unwrap();
    assert!(handle
        .add_cog(
            CogBuilder::new("permanent")
                .registered_command(command)
                .build()
        )
        .await
        .is_err());
    assert!(handle.help("persist").await.contains("Old"));
    assert!(handle.help("newalias").await.is_empty());
    resume.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    http.await.unwrap();
    let mut publications = 0;
    while let Ok(request) = requests.try_recv() {
        if request.starts_with("PUT /commands ") {
            publications += 1;
        }
    }
    assert_eq!(
        publications, 2,
        "failed publication must not be blindly retried"
    );
}

#[cfg(feature = "cli")]
#[test]
fn library_and_cli_publish_identical_operation_metadata() {
    let root = std::env::temp_dir().join(format!("rappin-catalogue-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let config = root.join("empty.toml");
    std::fs::write(&config, "").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_rappin"))
        .env_remove("YAPPER_BOT_TOKEN")
        .env_remove("YAPPER_API_BASE")
        .args(["--config", config.to_str().unwrap(), "tools", "list"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["data"]["tools"], json!(tools()));
    std::fs::remove_dir_all(root).unwrap();
}

fn management_calls() -> Vec<(&'static str, Value, Method, String)> {
    let category = Uuid::from_u128(6);
    vec![
        (
            "create_channel",
            json!({"server":server(),"body":{"name":"chat","kind":"text","category_id":category}}),
            Method::POST,
            format!("/servers/{}/channels", server()),
        ),
        (
            "create_channel",
            json!({"server":server(),"body":{"name":"voice","kind":"voice"}}),
            Method::POST,
            format!("/servers/{}/channels", server()),
        ),
        (
            "edit_channel",
            json!({"id":channel(),"body":{"name":"renamed","category_id":null,"slowmode":21600,"read_only":false}}),
            Method::PATCH,
            format!("/channels/{}", channel()),
        ),
        (
            "delete_channel",
            json!({"id":channel()}),
            Method::DELETE,
            format!("/channels/{}", channel()),
        ),
        (
            "create_category",
            json!({"id":server(),"body":{"name":"Topics","position":-1}}),
            Method::POST,
            format!("/servers/{}/categories", server()),
        ),
        (
            "edit_category",
            json!({"id":server(),"category":category,"body":{"name":"Renamed"}}),
            Method::PATCH,
            format!("/servers/{}/categories/{}", server(), category),
        ),
        (
            "delete_category",
            json!({"id":server(),"category":category}),
            Method::DELETE,
            format!("/servers/{}/categories/{}", server(), category),
        ),
    ]
}

#[test]
fn channel_category_schemas_validate_bounds_types_and_nullable_patch_semantics() {
    for (name, args, _, _) in management_calls() {
        let call = operation(name, args.clone());
        assert_eq!(call.arguments(), &args, "canonical body for {name}");
        assert!(!call.capability().availability.test_servers_only);
        assert_eq!(
            call.capability().availability.library,
            LibrarySupport::ScopedApi
        );
    }
    let edit = capability("edit_channel").unwrap();
    for body in [
        json!({"slowmode":-1}),
        json!({"slowmode":21601}),
        json!({"slowmode":1.5}),
        json!({"category_id":"invalid"}),
        json!({"read_only":null}),
        json!({"role_ids":["invalid"]}),
        json!({"unknown":true}),
    ] {
        assert!(
            edit.validate(&json!({"id":channel(),"body":body})).is_err(),
            "{body}"
        );
    }
    for body in [
        json!({"slowmode":0}),
        json!({"category_id":null}),
        json!({"category_id":Uuid::from_u128(6)}),
        json!({"read_only":true}),
        json!({"private":false,"role_ids":[],"post_role_ids":[role()]}),
    ] {
        assert_eq!(
            edit.validate(&json!({"id":channel(),"body":body}))
                .unwrap()
                .arguments()["body"],
            body
        );
    }
    let unchanged = edit
        .validate(&json!({"id":channel(),"body":{"name":"x"}}))
        .unwrap();
    assert!(unchanged.arguments()["body"].get("category_id").is_none());
    assert!(EditChannelRequest::new()
        .slowmode(21601)
        .validate()
        .is_err());
    assert!(capability("create_channel")
        .unwrap()
        .validate(&json!({"server":server(),"body":{"name":"x","kind":"video"}}))
        .is_err());
    for position in [json!(i32::MIN), json!(i32::MAX)] {
        operation(
            "create_category",
            json!({"id":server(),"body":{"name":"x","position":position}}),
        );
    }
    assert!(capability("create_category")
        .unwrap()
        .validate(&json!({"id":server(),"body":{"name":"x","position":2147483648_i64}}))
        .is_err());
    assert!(capability("edit_category")
        .unwrap()
        .validate(&json!({"id":server(),"category":Uuid::from_u128(6),"body":{}}))
        .is_err());
}

#[tokio::test]
async fn channel_category_bindings_use_separate_principals_and_exact_typed_requests() {
    for (name, args, method, path) in management_calls() {
        let api = MockApi {
            request_grants: vec!["manage_channels"],
            bot_grants: vec!["manage_channels"],
            ..MockApi::standard()
        };
        let call = operation(name, args);
        let result = call
            .execute_with_policy(&api, &context(), &HostEvidence::allow())
            .await
            .unwrap();
        assert!(result.authorization.unwrap().allowed());
        let mutations: Vec<_> = api
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| *m != Method::GET)
            .cloned()
            .collect();
        assert_eq!(mutations.len(), 1);
        assert_eq!(
            (&mutations[0].0, &mutations[0].1, &mutations[0].2),
            (&method, &path, &call.arguments().get("body").cloned())
        );
        assert_eq!(
            api.principals
                .lock()
                .unwrap()
                .iter()
                .map(|(actor, _)| *actor)
                .collect::<Vec<_>>(),
            vec![requester(), bot_id()]
        );
        for (request_grants, bot_grants) in [
            (vec![], vec!["administrator"]),
            (vec!["administrator"], vec![]),
        ] {
            let api = MockApi {
                request_grants,
                bot_grants,
                ..MockApi::standard()
            };
            assert!(
                call.execute_with_policy(&api, &context(), &HostEvidence::allow())
                    .await
                    .unwrap_err()
                    .is_forbidden(),
                "{name}"
            );
            assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
        }
        let api = MockApi {
            request_grants: vec!["manage_channels"],
            bot_grants: vec!["manage_channels"],
            mutation_error: Some(503),
            ..MockApi::standard()
        };
        let error = call
            .execute_with_policy(&api, &context(), &HostEvidence::allow())
            .await
            .unwrap_err();
        assert_eq!(error.to_json()["retry_safety"], "unknown_write_outcome");
        assert_eq!(
            api.mutations.load(Ordering::SeqCst),
            1,
            "must not replay {name}"
        );
    }
}

#[tokio::test]
async fn management_scope_and_category_permissions_require_evidence_before_mutation() {
    let managed = || MockApi {
        request_grants: vec!["manage_channels"],
        bot_grants: vec!["manage_channels"],
        ..MockApi::standard()
    };
    for (name, args, _, _) in management_calls() {
        let api = managed();
        let call = operation(name, args);
        let report = call.assess(&api, &context()).await.unwrap();
        if matches!(name, "create_category")
            || (name == "create_channel" && call.arguments()["body"]["category_id"].is_null())
        {
            assert!(report.allowed());
        } else {
            assert_eq!(report.decision(), AccessDecision::Unknown, "{name}");
        }
        assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    }
    let foreign = Uuid::from_u128(999);
    for (name, args) in [
        (
            "create_channel",
            json!({"server":foreign,"body":{"name":"x"}}),
        ),
        (
            "create_channel",
            json!({"server":server(),"body":{"name":"x","category_id":foreign}}),
        ),
        (
            "edit_channel",
            json!({"id":channel(),"body":{"category_id":foreign}}),
        ),
        (
            "edit_channel",
            json!({"id":channel(),"body":{"role_ids":[role(),foreign]}}),
        ),
        (
            "edit_channel",
            json!({"id":channel(),"body":{"post_role_ids":[foreign]}}),
        ),
        (
            "edit_category",
            json!({"id":server(),"category":foreign,"body":{"name":"x"}}),
        ),
        ("delete_category", json!({"id":server(),"category":foreign})),
        ("create_category", json!({"id":foreign,"body":{"name":"x"}})),
    ] {
        let api = managed();
        assert!(
            operation(name, args)
                .execute_with_policy(&api, &context(), &HostEvidence::allow())
                .await
                .unwrap_err()
                .is_forbidden(),
            "{name}"
        );
        assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    }
    for name in ["edit_channel", "delete_channel"] {
        let api = MockApi {
            channel_server: Some(foreign),
            ..managed()
        };
        let args = if name == "edit_channel" {
            json!({"id":channel(),"body":{"name":"x"}})
        } else {
            json!({"id":channel()})
        };
        assert!(operation(name, args)
            .execute_with_policy(&api, &context(), &HostEvidence::allow())
            .await
            .unwrap_err()
            .is_forbidden());
        assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    }
    let call = operation(
        "create_channel",
        json!({"server":server(),"body":{"name":"x","category_id":Uuid::from_u128(6)}}),
    );
    let report = call.assess(&managed(), &context()).await.unwrap();
    for principal in [Principal::Requester, Principal::Bot] {
        assert!(report
            .checks
            .iter()
            .any(|c| c.check == "category_permissions"
                && c.principal == principal
                && c.decision == AccessDecision::Unknown));
    }
}

fn channel_binding() -> OperationCommandBinding {
    serde_json::from_value(json!({"operation":"edit_channel","name":"configure","description":"Configure a channel","aliases":["cfg"],"fixed_arguments":{"id":channel()},"parameters":[{"name":"category","target":"/body/category_id"},{"name":"slowmode","target":"/body/slowmode"},{"name":"read_only","target":"/body/read_only"}]})).unwrap()
}
#[tokio::test]
async fn channel_command_parsing_help_preflight_and_reload_share_operation_metadata() {
    let binding = channel_binding();
    let schema = binding.command_schema().unwrap();
    let call = binding
        .prepare(&schema.parse("null 21600 false").unwrap())
        .unwrap();
    assert_eq!(
        call.arguments()["body"],
        json!({"category_id":null,"slowmode":21600,"read_only":false})
    );
    assert!(schema.parse("null 21601 false").is_err());
    assert!(schema.parse("invalid 0 false").is_err());
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .prefix("?")
        .build();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let calls = Arc::new(Mutex::new(vec![]));
    let install = |binding: &OperationCommandBinding| {
        let observed = calls.clone();
        binding
            .clone()
            .bind(move |_, call| {
                let observed = observed.clone();
                async move {
                    observed.lock().unwrap().push(call.arguments().clone());
                    Ok(())
                }
            })
            .unwrap()
    };
    binding
        .preflight(&bot.handle(), Some("settings"))
        .await
        .unwrap();
    bot.handle()
        .add_cog(
            CogBuilder::new("settings")
                .registered_command(install(&binding))
                .build(),
        )
        .await
        .unwrap();
    assert!(binding.preflight(&bot.handle(), None).await.is_err());
    assert!(bot
        .handle()
        .help("configure")
        .await
        .contains("<category> <slowmode> <read_only>"));
    let event = |content: &str| {
        serde_json::from_value(json!({"id":1,"type":"message","data":{"id":Uuid::nil(),"channel_id":channel(),"author_id":requester(),"server_id":server(),"content":content}})).unwrap()
    };
    bot.process_event(event("?cfg null 0 true")).await.unwrap();
    let mut replacement = binding.clone();
    replacement.aliases = vec!["settings".into()];
    replacement
        .preflight(&bot.handle(), Some("settings"))
        .await
        .unwrap();
    bot.handle()
        .reload_cog(
            CogBuilder::new("settings")
                .registered_command(install(&replacement))
                .build(),
        )
        .await
        .unwrap();
    bot.process_event(event("?cfg null 0 true")).await.unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
    bot.process_event(event("?settings null 12 false"))
        .await
        .unwrap();
    assert_eq!(
        calls.lock().unwrap()[1]["body"],
        json!({"category_id":null,"slowmode":12,"read_only":false})
    );
}

#[test]
fn create_and_rename_command_schemas_cover_required_names_and_channel_kinds() {
    for (name, fixed) in [
        (
            "create_channel",
            json!({"server":server(),"body":{"kind":"voice"}}),
        ),
        (
            "create_category",
            json!({"id":server(),"body":{"position":1}}),
        ),
        (
            "edit_category",
            json!({"id":server(),"category":Uuid::from_u128(6)}),
        ),
    ] {
        let binding: OperationCommandBinding = serde_json::from_value(json!({"operation":name,"name":"label","description":"Set a name","fixed_arguments":fixed,"parameters":[{"name":"name","target":"/body/name","rest":true}]})).unwrap();
        let values = binding
            .command_schema()
            .unwrap()
            .parse("two words")
            .unwrap();
        assert_eq!(
            binding.prepare(&values).unwrap().arguments()["body"]["name"],
            "two words"
        );
        let mut incomplete = binding.clone();
        incomplete.parameters.clear();
        assert!(
            incomplete.validate().is_err(),
            "required name must be bound for {name}"
        );
    }
}

#[tokio::test]
async fn typed_channel_slowmode_validation_precedes_http() {
    let client = Client::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .build();
    let error = client
        .edit_channel(channel(), &EditChannelRequest::new().slowmode(21601))
        .await
        .unwrap_err();
    assert!(matches!(error, YappinError::Validation { .. }));
}

#[tokio::test]
async fn category_access_and_privacy_authority_are_independent_host_facts() {
    let managed = || MockApi {
        request_grants: vec!["manage_channels"],
        bot_grants: vec!["manage_channels"],
        ..MockApi::standard()
    };
    let call = operation(
        "create_channel",
        json!({"server":server(),"body":{"name":"x","category_id":Uuid::from_u128(6)}}),
    );
    for (requester, bot) in [
        (AccessDecision::Denied, AccessDecision::Allowed),
        (AccessDecision::Allowed, AccessDecision::Denied),
    ] {
        let api = managed();
        assert!(call
            .execute_with_policy(&api, &context(), &HostEvidence { requester, bot })
            .await
            .unwrap_err()
            .is_forbidden());
        assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    }
    let api = managed();
    let call = operation(
        "edit_channel",
        json!({"id":channel(),"body":{"private":true,"role_ids":[role()],"post_role_ids":[role()]}}),
    );
    let report = call.assess(&api, &context()).await.unwrap();
    for principal in [Principal::Requester, Principal::Bot] {
        assert!(report.checks.iter().any(|c| c.check == "override_authority"
            && c.principal == principal
            && c.decision == AccessDecision::Unknown));
    }
    assert_eq!(
        api.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, path, _)| path.ends_with("/roles"))
            .count(),
        1
    );
}

#[tokio::test]
async fn api12_bindings_keep_principals_scope_and_webhook_ownership_separate() {
    let cases = [
        (
            "delete_emoji",
            json!({"id":server(),"emoji":role()}),
            "manage_emojis",
        ),
        (
            "delete_bot_webhook",
            json!({"channel":channel(),"webhook":role()}),
            "manage_webhooks",
        ),
    ];
    for (name, args, permission) in cases {
        for (requester_allowed, bot_allowed) in [(true, false), (false, true), (true, true)] {
            let mut api = MockApi::standard();
            if requester_allowed {
                api.request_grants = vec![permission];
            }
            if bot_allowed {
                api.bot_grants = vec![permission];
            }
            let call = operation(name, args.clone());
            let result = call
                .execute_with_policy(
                    &api,
                    &context().test_server_access(true),
                    &HostEvidence::allow(),
                )
                .await;
            assert_eq!(result.is_ok(), requester_allowed && bot_allowed, "{name}");
            assert_eq!(
                api.mutations.load(Ordering::SeqCst),
                usize::from(requester_allowed && bot_allowed)
            );
        }
        let mut api = MockApi::standard();
        api.request_grants = vec![permission];
        api.bot_grants = vec![permission];
        api.resource_server = Some(Uuid::from_u128(999));
        let error = operation(name, args.clone())
            .execute_with_policy(
                &api,
                &context().test_server_access(true),
                &HostEvidence::allow(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.to_json()["decision"], "denied");
        assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    }
    for (owner, decision) in [(Some(Uuid::from_u128(999)), "denied"), (None, "unknown")] {
        let mut api = MockApi::standard();
        api.request_grants = vec!["manage_webhooks"];
        api.bot_grants = vec!["manage_webhooks"];
        api.webhook_owner = Some(owner);
        let error = operation(
            "delete_bot_webhook",
            json!({"channel":channel(),"webhook":role()}),
        )
        .execute_with_policy(
            &api,
            &context().test_server_access(true),
            &HostEvidence::allow(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_json()["decision"], decision);
        assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
    }
    let mut api = MockApi::standard();
    api.request_grants = vec!["manage_webhooks"];
    api.bot_grants = vec!["manage_webhooks"];
    let call = operation(
        "delete_bot_webhook",
        json!({"channel":channel(),"webhook":role()}),
    );
    assert_eq!(
        call.assess(&api, &context().test_server_access(true))
            .await
            .unwrap()
            .decision(),
        AccessDecision::Unknown
    );
    assert_eq!(api.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn api12_bot_webhook_lists_are_scoped_and_command_schemas_use_current_metadata() {
    let mut api = MockApi::standard();
    api.request_grants = vec!["manage_webhooks"];
    api.bot_grants = vec!["manage_webhooks"];
    let result = operation("list_bot_webhooks", json!({}))
        .execute_with_policy(
            &api,
            &context().test_server_access(true),
            &HostEvidence::allow(),
        )
        .await
        .unwrap();
    assert_eq!(result.data["webhooks"].as_array().unwrap().len(), 1);
    assert_eq!(
        result.data["webhooks"][0]["server_id"],
        server().to_string()
    );
    let binding:OperationCommandBinding=serde_json::from_value(json!({"operation":"create_emoji","name":"emoji add","description":"Create a server emoji","fixed_arguments":{"id":server(),"body":{"image":"YQ=="}},"parameters":[{"name":"name","target":"/body/name"}]})).unwrap();
    let schema = binding.command_schema().unwrap();
    let values = schema.parse("party_name").unwrap();
    assert_eq!(
        binding.prepare(&values).unwrap().arguments()["body"]["name"],
        "party_name"
    );
    assert!(binding.prepare(&schema.parse("bad-name").unwrap()).is_err());
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .build();
    let command = binding.bind(|_, _| async { Ok(()) }).unwrap();
    let cog = CogBuilder::new("emoji_commands")
        .registered_command(command)
        .build();
    bot.handle().preflight_cog(&cog).await.unwrap();
    bot.handle().add_cog(cog).await.unwrap();
    assert!(bot
        .handle()
        .command_info()
        .await
        .iter()
        .any(|c| c.name == "emoji add"));
}
