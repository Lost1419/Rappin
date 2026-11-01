use super::{call, fixture::*, policy::*};
use rappin::{capabilities::*, Api, Freshness, Permission, Result};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use uuid::Uuid;

fn context() -> OperationContext {
    OperationContext::server(server(), requester())
}
fn question<'a>(
    check: ExternalCheck,
    operation: &'static str,
    principal: Principal,
    permissions: &'a [&'static str],
    args: &'a Value,
) -> AccessQuestion<'a> {
    AccessQuestion {
        check,
        operation,
        principal,
        actor_id: Some(if principal == Principal::Bot {
            bot()
        } else {
            requester()
        }),
        server_id: server(),
        channel_id: Some(channel()),
        permissions,
        arguments: args,
    }
}
#[tokio::test]
async fn positive_proof_is_limited_to_the_authenticated_bots_own_view() {
    let api = Fixture::default();
    let policy = PublicApiPolicy { api: &api };
    let args = json!({"id":channel()});
    let q = question(
        ExternalCheck::ChannelPermissions,
        "get_channel",
        Principal::Bot,
        &["view_channel"],
        &args,
    );
    assert_eq!(
        policy.check(&q).await.unwrap().decision,
        AccessDecision::Allowed
    );
    let q = question(
        ExternalCheck::ChannelPermissions,
        "edit_channel",
        Principal::Bot,
        &["view_channel", "manage_channels"],
        &args,
    );
    assert_eq!(
        policy.check(&q).await.unwrap().decision,
        AccessDecision::Unknown
    );
    let q = question(
        ExternalCheck::ChannelPermissions,
        "get_channel",
        Principal::Requester,
        &["view_channel"],
        &args,
    );
    assert_eq!(
        policy.check(&q).await.unwrap().decision,
        AccessDecision::Unknown
    );
}

#[tokio::test]
async fn bot_identity_and_cross_server_resource_mismatches_are_denied() {
    let api = Fixture::default();
    let args = json!({"id":channel()});
    let mut q = question(
        ExternalCheck::ChannelPermissions,
        "delete_channel",
        Principal::Bot,
        &["view_channel", "manage_channels"],
        &args,
    );
    q.actor_id = Some(requester());
    assert_eq!(
        PublicApiPolicy { api: &api }
            .check(&q)
            .await
            .unwrap()
            .decision,
        AccessDecision::Denied
    );
    api.channel_row.lock().unwrap()["server_id"] = json!(Uuid::from_u128(900));
    q.actor_id = Some(bot());
    assert_eq!(
        PublicApiPolicy { api: &api }
            .check(&q)
            .await
            .unwrap()
            .decision,
        AccessDecision::Denied
    );
    let error = execute(&api, &context(), &call("delete_channel", &args).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["decision"], "denied");
    assert_eq!(error.status_code(), Some(403));
    assert_eq!(api.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fresh_server_denials_remain_separate_for_requester_and_bot() {
    for principal in [Principal::Requester, Principal::Bot] {
        let api = Fixture::default();
        if principal == Principal::Requester {
            api.requester_grant.store(false, Ordering::SeqCst);
        } else {
            api.bot_grant.store(false, Ordering::SeqCst);
        }
        let operation = call("delete_channel", &json!({"id":channel()})).unwrap();
        let report = operation
            .assess_with_policy(&api, &context(), &PublicApiPolicy { api: &api })
            .await
            .unwrap();
        assert_eq!(report.decision(), AccessDecision::Denied);
        assert!(report.checks.iter().any(|c| c.check == "server_permissions"
            && c.principal == principal
            && c.decision == AccessDecision::Denied));
        assert!(execute(&api, &context(), &operation).await.is_err());
        assert_eq!(api.writes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn server_ownership_and_administrator_are_not_invented_channel_bypasses() {
    for owner in [false, true] {
        let api = Fixture::default();
        if owner {
            *api.owner.lock().unwrap() = requester();
        } else {
            api.administrator.store(true, Ordering::SeqCst);
        }
        let args = json!({"id":channel()});
        let q = question(
            ExternalCheck::ChannelPermissions,
            "delete_channel",
            Principal::Requester,
            &["view_channel", "manage_channels"],
            &args,
        );
        assert_eq!(
            PublicApiPolicy { api: &api }
                .check(&q)
                .await
                .unwrap()
                .decision,
            AccessDecision::Unknown
        );
    }
}

#[tokio::test]
async fn configured_grants_denies_conflicts_and_category_fallback_do_not_define_precedence() {
    for parent in [Value::Null, json!(category())] {
        for rules in [
            json!([]),
            json!([{"role_id":null,"allow":["view_channel"],"deny":[]}]),
            json!([{"role_id":null,"allow":[],"deny":["view_channel"]}]),
            json!([{"role_id":null,"allow":[],"deny":["view_channel"]},{"role_id":Uuid::from_u128(10),"allow":["view_channel"],"deny":[]}]),
            json!([{"role_id":Uuid::from_u128(10),"allow":["view_channel"],"deny":["view_channel"]}]),
        ] {
            let api = Fixture::default();
            api.channel_row.lock().unwrap()["category_id"] = parent.clone();
            *api.overrides.lock().unwrap() = json!({"overrides":rules});
            let args = json!({"id":channel()});
            let q = question(
                ExternalCheck::ChannelPermissions,
                "delete_channel",
                Principal::Requester,
                &["view_channel", "manage_channels"],
                &args,
            );
            assert_eq!(
                PublicApiPolicy { api: &api }
                    .check(&q)
                    .await
                    .unwrap()
                    .decision,
                AccessDecision::Unknown
            );
        }
    }
}

#[tokio::test]
async fn category_assignment_edits_and_deletes_leave_both_principals_unknown() {
    for (name, args) in [
        (
            "create_channel",
            json!({"server":server(),"body":{"name":"New","category_id":category()}}),
        ),
        (
            "edit_channel",
            json!({"id":channel(),"body":{"category_id":category()}}),
        ),
        (
            "edit_category",
            json!({"id":server(),"category":category(),"body":{"name":"Renamed"}}),
        ),
        (
            "delete_category",
            json!({"id":server(),"category":category()}),
        ),
    ] {
        let api = Fixture::default();
        let operation = call(name, &args).unwrap();
        let report = operation
            .assess_with_policy(&api, &context(), &PublicApiPolicy { api: &api })
            .await
            .unwrap();
        for principal in [Principal::Requester, Principal::Bot] {
            assert!(
                report
                    .checks
                    .iter()
                    .any(|c| c.check == "category_permissions"
                        && c.principal == principal
                        && c.decision == AccessDecision::Unknown),
                "{name}"
            );
        }
        assert!(execute(&api, &context(), &operation).await.is_err());
        assert_eq!(api.writes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn null_category_removal_is_not_mistaken_for_category_assignment() {
    let api = Fixture::default();
    let operation = call(
        "edit_channel",
        &json!({"id":channel(),"body":{"category_id":null}}),
    )
    .unwrap();
    let report = operation
        .assess_with_policy(&api, &context(), &PublicApiPolicy { api: &api })
        .await
        .unwrap();
    assert!(!report
        .checks
        .iter()
        .any(|c| c.check == "category_permissions"));
    assert_eq!(report.decision(), AccessDecision::Unknown); // channel ACL still unproven
}

#[tokio::test]
async fn read_failures_are_unknown_with_structured_causes_not_requester_denials() {
    for status in [403, 404, 429] {
        let api = Fixture::default();
        api.override_status.store(status, Ordering::SeqCst);
        let operation = call("delete_channel", &json!({"id":channel()})).unwrap();
        let error = execute(&api, &context(), &operation).await.unwrap_err();
        let output = error.to_json();
        assert_eq!(output["kind"], "operation_authorization");
        assert_eq!(output["decision"], "unknown");
        assert_eq!(output["status"], Value::Null);
        assert_eq!(output["outcome"], "not_applied");
        let checks = output["authorization"]["checks"].as_array().unwrap();
        for principal in ["requester", "bot"] {
            assert!(checks.iter().any(|c| c["check"] == "channel_permissions"
                && c["principal"] == principal
                && c["decision"] == "unknown"
                && c["error"]["status"] == status));
        }
        assert_eq!(api.writes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn missing_member_or_category_data_cannot_prove_denial_or_access() {
    let api = Fixture::default();
    api.member_status.store(404, Ordering::SeqCst);
    let report = call("delete_channel", &json!({"id":channel()}))
        .unwrap()
        .assess_with_policy(&api, &context(), &PublicApiPolicy { api: &api })
        .await
        .unwrap();
    assert_eq!(report.decision(), AccessDecision::Unknown);
    let api = Fixture::default();
    *api.categories.lock().unwrap() = Value::Null;
    let args = json!({"id":server(),"category":category()});
    let q = question(
        ExternalCheck::CategoryPermissions,
        "delete_category",
        Principal::Requester,
        &[],
        &args,
    );
    assert_eq!(
        PublicApiPolicy { api: &api }
            .check(&q)
            .await
            .unwrap()
            .decision,
        AccessDecision::Unknown
    );
}

#[tokio::test]
async fn every_assessment_and_execution_refreshes_each_actor() {
    let api = Fixture::default();
    let operation = call("delete_channel", &json!({"id":channel()})).unwrap();
    operation
        .assess_with_policy(&api, &context(), &PublicApiPolicy { api: &api })
        .await
        .unwrap();
    assert!(execute(&api, &context(), &operation).await.is_err());
    let observed = api.freshness.lock().unwrap();
    assert!(observed
        .iter()
        .all(|(_, freshness)| *freshness == Freshness::Refresh));
    for actor in [requester(), bot()] {
        assert!(observed.iter().filter(|(id, _)| *id == actor).count() >= 4);
    }
}

// TEST ONLY. This is deliberately not the public-API policy and represents a
// hypothetical trusted attestation. It tests SDK reassessment, not Yapin ACLs.
struct TestAttestation;
#[async_trait::async_trait]
impl OperationPolicy for TestAttestation {
    async fn check(&self, _: &AccessQuestion<'_>) -> Result<CheckResolution> {
        Ok(CheckResolution::allowed(
            "synthetic test attestation; not platform evidence",
        ))
    }
}
#[tokio::test]
async fn execution_rechecks_after_test_attestation_and_server_grant_revocation() {
    let api = Fixture::default();
    let operation = call("delete_channel", &json!({"id":channel()})).unwrap();
    let report = operation
        .assess_with_policy(&api, &context(), &TestAttestation)
        .await
        .unwrap();
    assert!(report.allowed());
    api.requester_grant.store(false, Ordering::SeqCst);
    let error = operation
        .execute_with_policy(&api, &context(), &TestAttestation)
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["decision"], "denied");
    assert_eq!(api.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn existing_server_evaluator_precedence_is_not_channel_precedence() {
    let api = Fixture::default();
    let mut snapshot = api
        .permission_snapshot(server(), requester(), Freshness::Refresh)
        .await
        .unwrap();
    snapshot.roles[0]
        .permissions
        .as_mut()
        .unwrap()
        .insert("manage_channels".into(), false);
    snapshot.roles[0].perms = Some(json!({"manage_channels":true}));
    assert!(!snapshot
        .evaluate()
        .unwrap()
        .allows(&Permission::new("manage_channels")));
    snapshot.roles.push(serde_json::from_value(json!({"id":Uuid::from_u128(12),"name":"everyone","auto":"everyone","permissions":{"manage_channels":true}})).unwrap());
    assert!(snapshot
        .evaluate()
        .unwrap()
        .allows(&Permission::new("manage_channels")));
}

#[test]
fn native_and_worker_registration_use_the_same_policy_and_schema() {
    let binding:OperationCommandBinding=serde_json::from_value(json!({"operation":"delete_channel","name":"remove","description":"Remove a channel","fixed_arguments":{"id":channel()}})).unwrap();
    assert!(native_command(binding.clone()).is_ok());
    #[cfg(feature = "hot-reload")]
    assert!(worker_cog(binding).is_ok());
}

#[tokio::test]
async fn category_rule_fields_without_a_contract_do_not_establish_inheritance() {
    let api = Fixture::default();
    for rules in [
        json!({"allow":["view_channel"],"deny":[]}),
        json!({"allow":[],"deny":["view_channel"]}),
    ] {
        *api.categories.lock().unwrap() =
            json!([{"id":category(),"name":"fixture","overrides":[rules]}]);
        let args = json!({"id":server(),"category":category()});
        for principal in [Principal::Requester, Principal::Bot] {
            let q = question(
                ExternalCheck::CategoryPermissions,
                "delete_category",
                principal,
                &[],
                &args,
            );
            assert_eq!(
                PublicApiPolicy { api: &api }
                    .check(&q)
                    .await
                    .unwrap()
                    .decision,
                AccessDecision::Unknown
            );
        }
    }
}

#[tokio::test]
async fn generic_execution_refreshes_even_if_the_caller_selected_cached_data() {
    let api = Fixture::default();
    let operation = call("delete_channel", &json!({"id":channel()})).unwrap();
    assert!(execute(
        &api,
        &context().freshness(Freshness::PreferCache),
        &operation
    )
    .await
    .is_err());
    assert!(api
        .freshness
        .lock()
        .unwrap()
        .iter()
        .all(|(_, f)| *f == Freshness::Refresh));
}

struct WrongActor<'a>(&'a Fixture);
#[async_trait::async_trait]
impl Api for WrongActor<'_> {
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.0.request(method, path, body).await
    }
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<rappin::PermissionSnapshot> {
        let mut snapshot = self.0.permission_snapshot(server, actor, freshness).await?;
        snapshot.actor = bot();
        Ok(snapshot)
    }
}
#[tokio::test]
async fn missing_or_mismatched_actor_evidence_cannot_grant_access() {
    let api = Fixture::default();
    let args = json!({"id":channel()});
    let mut q = question(
        ExternalCheck::ChannelPermissions,
        "delete_channel",
        Principal::Requester,
        &["view_channel", "manage_channels"],
        &args,
    );
    q.actor_id = None;
    assert_eq!(
        PublicApiPolicy { api: &api }
            .check(&q)
            .await
            .unwrap()
            .decision,
        AccessDecision::Unknown
    );
    q.actor_id = Some(requester());
    assert_eq!(
        PublicApiPolicy {
            api: &WrongActor(&api)
        }
        .check(&q)
        .await
        .unwrap()
        .decision,
        AccessDecision::Denied
    );
}

#[tokio::test]
async fn public_policy_never_relaxes_binding_server_or_destination_scope() {
    let foreign = Uuid::from_u128(900);
    for (name, args) in [
        (
            "create_channel",
            json!({"server":foreign,"body":{"name":"New"}}),
        ),
        (
            "create_channel",
            json!({"server":server(),"body":{"name":"New","category_id":foreign}}),
        ),
        (
            "edit_channel",
            json!({"id":channel(),"body":{"category_id":foreign}}),
        ),
        (
            "edit_category",
            json!({"id":server(),"category":foreign,"body":{"name":"New"}}),
        ),
        ("delete_category", json!({"id":server(),"category":foreign})),
    ] {
        let api = Fixture::default();
        let error = execute(&api, &context(), &call(name, &args).unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.to_json()["decision"], "denied", "{name}");
        assert_eq!(api.writes.load(Ordering::SeqCst), 0);
    }
}
