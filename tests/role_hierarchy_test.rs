mod support;
use async_trait::async_trait;
use rappin::{capabilities::*, *};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use uuid::Uuid;
fn server() -> Uuid {
    Uuid::from_u128(1)
}
fn actor() -> Uuid {
    Uuid::from_u128(2)
}
fn bot() -> Uuid {
    Uuid::from_u128(3)
}
fn target() -> Uuid {
    Uuid::from_u128(10)
}
fn own_role() -> Uuid {
    Uuid::from_u128(20)
}
fn bot_role() -> Uuid {
    Uuid::from_u128(30)
}
fn role(id: Uuid, position: i32, admin: bool) -> Value {
    json!({"id":id,"name":"role","position":position,"auto":null,"perms":{"manage_roles":true,"administrator":admin}})
}
fn server_data(owner: Uuid) -> Value {
    json!({"server":{"id":server(),"name":"fixture","owner_id":owner}})
}
fn member(id: Uuid, roles: Value) -> Value {
    json!({"member":{"id":id,"username":"member","roles":roles}})
}
struct State {
    owner: Uuid,
    roles: Value,
    requester_roles: Value,
    bot_roles: Value,
}
struct Mock {
    state: Mutex<State>,
    requests: Mutex<Vec<(Method, String)>>,
    deletes: AtomicUsize,
    ordinary_deletes: AtomicUsize,
}
impl Mock {
    fn new() -> Self {
        Self {
            state: Mutex::new(State {
                owner: Uuid::from_u128(99),
                roles: json!([
                    role(target(), 10, false),
                    role(own_role(), 20, false),
                    role(bot_role(), 30, false)
                ]),
                requester_roles: json!([own_role()]),
                bot_roles: json!([bot_role()]),
            }),
            requests: Default::default(),
            deletes: AtomicUsize::new(0),
            ordinary_deletes: AtomicUsize::new(0),
        }
    }
}
#[async_trait]
impl Api for Mock {
    async fn request(&self, method: Method, path: &str, _: Option<Value>) -> Result<Value> {
        self.requests
            .lock()
            .unwrap()
            .push((method.clone(), path.into()));
        if method == Method::DELETE {
            self.ordinary_deletes.fetch_add(1, Ordering::SeqCst);
            return Err(YappinError::Config(
                "ordinary DELETE must not be used".into(),
            ));
        }
        let s = self.state.lock().unwrap();
        if path == "/me" {
            return Ok(json!({"id":bot()}));
        }
        if path == format!("/servers/{}", server()) {
            return Ok(server_data(s.owner));
        }
        if path.ends_with("/roles") {
            return Ok(json!({"roles":s.roles}));
        }
        if path.ends_with(&actor().to_string()) {
            return Ok(member(actor(), s.requester_roles.clone()));
        }
        if path.ends_with(&bot().to_string()) {
            return Ok(member(bot(), s.bot_roles.clone()));
        }
        panic!("unexpected request {method} {path}");
    }
    async fn request_once(&self, method: Method, path: &str, _: Option<Value>) -> Result<Value> {
        assert_eq!(method, Method::DELETE);
        assert_eq!(path, format!("/servers/{}/roles/{}", server(), target()));
        self.deletes.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"ok":true}))
    }
}
fn operation() -> ValidatedOperation {
    capability("delete_role")
        .unwrap()
        .validate(&json!({"id":server(),"role":target()}))
        .unwrap()
}
fn context() -> OperationContext {
    OperationContext::server(server(), actor()).test_server_access(true)
}

#[tokio::test]
async fn explicit_owner_admin_policy_evaluates_requester_and_bot_independently() {
    let api = Mock::new();
    let policy = RoleHierarchyPolicy::new(&api).owner_admin_strict();
    let report = operation()
        .assess_with_policy(&api, &context(), &policy)
        .await
        .unwrap();
    assert!(report.allowed());
    assert_eq!(
        report
            .checks
            .iter()
            .filter(|c| c.check == "role_hierarchy" && c.decision == AccessDecision::Allowed)
            .count(),
        2
    );
    policy
        .execute_deletion(&operation(), &context())
        .await
        .unwrap();
    assert_eq!(api.deletes.load(Ordering::SeqCst), 1);
    assert_eq!(api.ordinary_deletes.load(Ordering::SeqCst), 0);
    api.state.lock().unwrap().roles[2]["position"] = 5.into();
    let report = operation()
        .assess_with_policy(&api, &context(), &policy)
        .await
        .unwrap();
    assert_eq!(report.decision(), AccessDecision::Denied);
    assert!(report.checks.iter().any(|c| c.check == "role_hierarchy"
        && c.principal == Principal::Requester
        && c.decision == AccessDecision::Allowed));
    assert!(report.checks.iter().any(|c| c.check == "role_hierarchy"
        && c.principal == Principal::Bot
        && c.decision == AccessDecision::Denied));
}

#[tokio::test]
async fn owner_above_admin_admin_peers_and_ties_follow_explicit_application_rules() {
    let api = Mock::new();
    let policy = RoleHierarchyPolicy::new(&api).owner_admin_strict();
    {
        let mut s = api.state.lock().unwrap();
        s.roles[0]["perms"]["administrator"] = true.into();
        s.roles[1]["perms"]["administrator"] = true.into();
    }
    let peer = policy
        .assess(server(), actor(), target(), Principal::Requester)
        .await
        .unwrap();
    assert_eq!(peer.decision, AccessDecision::Denied);
    api.state.lock().unwrap().owner = actor();
    let owner = policy
        .assess(server(), actor(), target(), Principal::Requester)
        .await
        .unwrap();
    assert!(owner.owner);
    assert_eq!(owner.decision, AccessDecision::Allowed);
    // An owner's request does not elevate the bot's independent authority.
    assert_eq!(
        operation()
            .assess_with_policy(&api, &context(), &policy)
            .await
            .unwrap()
            .decision(),
        AccessDecision::Denied
    );
    {
        let mut s = api.state.lock().unwrap();
        s.owner = Uuid::from_u128(99);
        s.roles[0]["perms"]["administrator"] = false.into();
        s.roles[0]["position"] = 20.into();
    }
    assert_eq!(
        policy
            .assess(server(), actor(), target(), Principal::Requester)
            .await
            .unwrap()
            .decision,
        AccessDecision::Denied
    );
}

#[tokio::test]
async fn public_evidence_does_not_invent_requester_ties_or_owner_admin_bypasses() {
    let api = Mock::new();
    let policy = RoleHierarchyPolicy::new(&api);
    assert_eq!(
        policy
            .assess(server(), actor(), target(), Principal::Requester)
            .await
            .unwrap()
            .decision,
        AccessDecision::Unknown
    );
    assert_eq!(
        policy
            .assess(server(), bot(), target(), Principal::Bot)
            .await
            .unwrap()
            .decision,
        AccessDecision::Allowed
    );
    api.state.lock().unwrap().roles[0]["position"] = 30.into();
    assert_eq!(
        policy
            .assess(server(), bot(), target(), Principal::Bot)
            .await
            .unwrap()
            .decision,
        AccessDecision::Unknown
    );
    {
        let mut s = api.state.lock().unwrap();
        s.roles[0]["position"] = 40.into();
        s.roles[2]["perms"]["administrator"] = true.into();
    }
    assert_eq!(
        policy
            .assess(server(), bot(), target(), Principal::Bot)
            .await
            .unwrap()
            .decision,
        AccessDecision::Unknown
    );
}

#[tokio::test]
async fn protected_roles_missing_data_and_incomplete_inventory_never_grant() {
    for mode in 0..8 {
        let api = Mock::new();
        let policy = RoleHierarchyPolicy::new(&api).owner_admin_strict();
        {
            let mut s = api.state.lock().unwrap();
            match mode {
                0 => s.roles[0]["auto"] = "everyone".into(),
                1 => s.roles[0]["auto"] = "booster".into(),
                2 => {
                    s.roles[0].as_object_mut().unwrap().remove("auto");
                }
                3 => {
                    s.roles[0].as_object_mut().unwrap().remove("position");
                }
                4 => {
                    s.roles[1].as_object_mut().unwrap().remove("position");
                }
                5 => s.requester_roles = json!([own_role(), Uuid::from_u128(1234)]),
                6 => s
                    .roles
                    .as_array_mut()
                    .unwrap()
                    .push(role(target(), 10, false)),
                _ => {
                    s.roles[0].as_object_mut().unwrap().remove("perms");
                }
            }
        }
        let assessment = policy
            .assess(server(), actor(), target(), Principal::Requester)
            .await
            .unwrap();
        assert_eq!(
            assessment.decision,
            if mode < 2 {
                AccessDecision::Denied
            } else {
                AccessDecision::Unknown
            },
            "mode {mode}"
        );
        assert_eq!(api.deletes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn scoped_lookup_rollout_and_revocation_are_rechecked_before_execution() {
    let api = Mock::new();
    let policy = RoleHierarchyPolicy::new(&api).owner_admin_strict();
    let foreign = capability("delete_role")
        .unwrap()
        .validate(&json!({"id":Uuid::from_u128(999),"role":target()}))
        .unwrap();
    assert!(policy
        .execute_deletion(&foreign, &context())
        .await
        .unwrap_err()
        .is_forbidden());
    assert!(api.requests.lock().unwrap().is_empty());
    assert_eq!(
        operation()
            .assess_with_policy(&api, &OperationContext::server(server(), actor()), &policy)
            .await
            .unwrap()
            .decision(),
        AccessDecision::Unknown
    );
    assert_eq!(
        operation()
            .assess_with_policy(
                &api,
                &OperationContext::server(server(), actor()).test_server_access(false),
                &policy
            )
            .await
            .unwrap()
            .decision(),
        AccessDecision::Denied
    );
    assert!(operation()
        .assess_with_policy(&api, &context(), &policy)
        .await
        .unwrap()
        .allowed());
    api.state.lock().unwrap().requester_roles = json!([]);
    let error = policy
        .execute_deletion(&operation(), &context().freshness(Freshness::PreferCache))
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "operation_authorization");
    assert!(error.is_forbidden());
    assert_eq!(api.deletes.load(Ordering::SeqCst), 0);
    api.state.lock().unwrap().roles[0]["server_id"] = json!(Uuid::from_u128(999));
    assert_eq!(
        policy
            .assess(server(), bot(), target(), Principal::Bot)
            .await
            .unwrap()
            .decision,
        AccessDecision::Denied
    );
}

#[tokio::test]
async fn native_transport_never_retries_a_role_deletion_and_preserves_failure_outcomes() {
    for status in [200, 403, 502, 503] {
        let roles = json!([
            role(target(), 10, false),
            role(own_role(), 20, false),
            role(bot_role(), 30, false)
        ]);
        let (base, mut requests, http) = support::routed_mock(vec![
            (
                "GET /me".into(),
                vec![support::response(200, json!({"id":bot()})); 2],
            ),
            (
                format!("GET /servers/{}", server()),
                vec![support::response(200, server_data(Uuid::from_u128(99))); 4],
            ),
            (
                format!("GET /servers/{}/roles", server()),
                vec![support::response(200, json!({"roles":roles})); 5],
            ),
            (
                format!("GET /servers/{}/members/{}", server(), actor()),
                vec![support::response(200, member(actor(), json!([own_role()]))); 2],
            ),
            (
                format!("GET /servers/{}/members/{}", server(), bot()),
                vec![support::response(200, member(bot(), json!([bot_role()]))); 2],
            ),
            (
                format!("DELETE /servers/{}/roles/{}", server(), target()),
                vec![(
                    status,
                    vec![("Retry-After", "0.001")],
                    if status == 200 {
                        "{".into()
                    } else {
                        json!({"error":"delete failed"}).to_string()
                    },
                )],
            ),
        ])
        .await;
        let api = Client::builder("fixture")
            .base_url(base)
            .max_retries(7)
            .build();
        let policy = RoleHierarchyPolicy::new(&api).owner_admin_strict();
        let error = policy
            .execute_deletion(&operation(), &context())
            .await
            .unwrap_err();
        assert_eq!(error.status_code(), Some(status));
        assert_eq!(
            error.retry_after(),
            if status == 200 {
                None
            } else {
                Some(std::time::Duration::from_millis(1))
            }
        );
        let value = error.to_json();
        assert_eq!(value["retry_safety"], "do_not_replay_workflow");
        if status == 502 || status == 200 {
            assert_eq!(value["outcome"], "unknown");
        }
        let mut deletes = 0;
        while let Ok(request) = requests.try_recv() {
            if request.starts_with("DELETE ") {
                deletes += 1;
            }
        }
        assert_eq!(deletes, 1);
        http.await.unwrap();
    }
}

#[tokio::test]
async fn custom_adapters_without_single_attempt_support_fail_closed() {
    struct ReadsOnly(Mock);
    #[async_trait]
    impl Api for ReadsOnly {
        async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
            self.0.request(method, path, body).await
        }
    }
    let api = ReadsOnly(Mock::new());
    let policy = RoleHierarchyPolicy::new(&api).owner_admin_strict();
    let error = policy
        .execute_deletion(&operation(), &context())
        .await
        .unwrap_err();
    assert_eq!(error.to_json()["kind"], "validation");
    assert!(error.to_string().contains("single-attempt"));
    assert_eq!(api.0.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(api.0.ordinary_deletes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fresh_evidence_failures_keep_structured_status_instead_of_guessing_authority() {
    struct Unavailable;
    #[async_trait]
    impl Api for Unavailable {
        async fn request(&self, _: Method, _: &str, _: Option<Value>) -> Result<Value> {
            Err(YappinError::Api {
                status: 503,
                message: "fresh role evidence unavailable".into(),
                retry_after: Some(2.0),
            })
        }
    }
    let error = RoleHierarchyPolicy::new(&Unavailable)
        .owner_admin_strict()
        .assess(server(), actor(), target(), Principal::Requester)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(2)));
    assert_eq!(error.to_json()["kind"], "api");
}
