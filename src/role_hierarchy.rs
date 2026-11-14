//! Fresh role evidence and explicit application hierarchy rules.
use crate::{
    capabilities::*, Api, Error, Freshness, MemberDetail, PermissionSnapshot, Result, Role, Server,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// Select conservative public evidence or an explicit application hierarchy rule.
///
/// # Example
///
/// ```rust
/// use rappin::role_hierarchy::RoleHierarchyRules;
/// let rules = RoleHierarchyRules::PublicApi;
/// ```
pub enum RoleHierarchyRules {
    /// Public documentation establishes bot position checks, but not arbitrary
    /// requester authority, ties or owner/administrator hierarchy exemptions.
    PublicApi,
    /// Explicit application rule: owner above editable roles; other actors cannot
    /// modify administrator roles, and require a strictly higher assigned position.
    /// Neither Administrator nor an equal position bypasses that rule.
    OwnerAdminStrict,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
/// Scoped hierarchy evidence; execution must reassess current access.
///
/// # Example
///
/// ```no_run
/// use rappin::{Api, role_hierarchy::RoleHierarchyPolicy, capabilities::Principal};
/// use uuid::Uuid;
/// async fn inspect<A: Api + ?Sized>(api: &A, server: Uuid, actor: Uuid, role: Uuid) -> rappin::Result<()> {
///     let report = RoleHierarchyPolicy::new(api).assess(server, actor, role, Principal::Requester).await?;
///     let answer = report.resolution();
///     println!("{:?}: {}", answer.decision, answer.reason);
///     // Assess the bot separately; Unknown does not authorize deletion.
///     Ok(())
/// }
/// ```
pub struct RoleHierarchyAssessment {
    /// Server that supplied the evidence.
    pub server_id: Uuid,
    /// Requester or bot assessed independently.
    pub actor_id: Uuid,
    /// Role whose deletion is assessed.
    pub target_role_id: Uuid,
    /// Allowed, Denied or Unknown; Unknown blocks validated execution.
    pub decision: AccessDecision,
    /// Explanation of this evidence and selected hierarchy rule.
    pub reason: String,
    /// Whether server evidence identifies this actor as owner.
    pub owner: bool,
    /// Actor Administrator grant, or None if unavailable.
    pub administrator: Option<bool>,
    /// Target role Administrator grant, or None if unavailable.
    pub target_administrator: Option<bool>,
    /// Highest resolved assigned role position, when known.
    pub highest_assigned_position: Option<i32>,
    /// Target role numeric position, when supplied.
    pub target_position: Option<i32>,
    /// Every role needed by this decision was resolved. This does not attest
    /// that the API exposed every other role in the server.
    pub required_roles_complete: bool,
    /// Evidence or authoritative semantics unavailable for this decision.
    pub unavailable_facts: Vec<String>,
}
impl RoleHierarchyAssessment {
    /// Convert this evidence into a policy answer without changing its decision.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, role_hierarchy::RoleHierarchyPolicy, capabilities::Principal};
    /// use uuid::Uuid;
    /// async fn inspect<A: Api + ?Sized>(api: &A, server: Uuid, actor: Uuid, role: Uuid) -> rappin::Result<()> {
    ///     let report = RoleHierarchyPolicy::new(api).assess(server, actor, role, Principal::Requester).await?;
    ///     let answer = report.resolution();
    ///     println!("{:?}: {}", answer.decision, answer.reason);
    ///     // Assess the bot separately; Unknown does not authorize deletion.
    ///     Ok(())
    /// }
    /// ```
    pub fn resolution(&self) -> CheckResolution {
        CheckResolution {
            decision: self.decision,
            reason: self.reason.clone(),
        }
    }
    fn finish(mut self, decision: AccessDecision, reason: &str) -> Self {
        self.decision = decision;
        self.reason = reason.into();
        if decision == AccessDecision::Unknown {
            self.unavailable_facts.push(reason.into());
        }
        self
    }
}

/// Fresh-data hierarchy policy for native/worker Api implementations. Unknown facts remain unresolved.
///
/// # Example
///
/// ```rust
/// use rappin::{Api, role_hierarchy::RoleHierarchyPolicy};
/// fn configured<A: Api + ?Sized>(api: &A) {
///     let conservative = RoleHierarchyPolicy::new(api);
///     let explicit_rule = RoleHierarchyPolicy::new(api).owner_admin_strict();
///     // The explicit application rule is not a claim about undocumented platform exemptions.
/// }
/// ```
pub struct RoleHierarchyPolicy<'a, A: Api + ?Sized> {
    api: &'a A,
    rules: RoleHierarchyRules,
}
impl<'a, A: Api + ?Sized> RoleHierarchyPolicy<'a, A> {
    /// Construct a conservative policy using fresh public evidence.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Api, role_hierarchy::RoleHierarchyPolicy};
    /// fn configured<A: Api + ?Sized>(api: &A) {
    ///     let conservative = RoleHierarchyPolicy::new(api);
    ///     let explicit_rule = RoleHierarchyPolicy::new(api).owner_admin_strict();
    ///     // The explicit application rule is not a claim about undocumented platform exemptions.
    /// }
    /// ```
    pub fn new(api: &'a A) -> Self {
        Self {
            api,
            rules: RoleHierarchyRules::PublicApi,
        }
    }
    /// Opt in to the application's owner/admin/strict-position rule. This does
    /// not claim these undocumented exemptions/ties are platform semantics.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Api, role_hierarchy::RoleHierarchyPolicy};
    /// fn configured<A: Api + ?Sized>(api: &A) {
    ///     let conservative = RoleHierarchyPolicy::new(api);
    ///     let explicit_rule = RoleHierarchyPolicy::new(api).owner_admin_strict();
    ///     // The explicit application rule is not a claim about undocumented platform exemptions.
    /// }
    /// ```
    pub fn owner_admin_strict(mut self) -> Self {
        self.rules = RoleHierarchyRules::OwnerAdminStrict;
        self
    }

    /// Fetch fresh scoped server, role and membership evidence; propagate sanitized structured API failures.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, role_hierarchy::RoleHierarchyPolicy, capabilities::Principal};
    /// use uuid::Uuid;
    /// async fn inspect<A: Api + ?Sized>(api: &A, server: Uuid, actor: Uuid, role: Uuid) -> rappin::Result<()> {
    ///     let report = RoleHierarchyPolicy::new(api).assess(server, actor, role, Principal::Requester).await?;
    ///     let answer = report.resolution();
    ///     println!("{:?}: {}", answer.decision, answer.reason);
    ///     // Assess the bot separately; Unknown does not authorize deletion.
    ///     Ok(())
    /// }
    /// ```
    pub async fn assess(
        &self,
        server: Uuid,
        actor: Uuid,
        target: Uuid,
        principal: Principal,
    ) -> Result<RoleHierarchyAssessment> {
        self.assess_inner(server, actor, target, principal)
            .await
            .map_err(|error| {
                let cause = self.api.sanitize(error.to_json());
                serde_json::from_value::<crate::RemoteError>(cause)
                    .expect("structured cause")
                    .into()
            })
    }
    async fn assess_inner(
        &self,
        server: Uuid,
        actor: Uuid,
        target: Uuid,
        principal: Principal,
    ) -> Result<RoleHierarchyAssessment> {
        let mut report = RoleHierarchyAssessment {
            server_id: server,
            actor_id: actor,
            target_role_id: target,
            decision: AccessDecision::Unknown,
            reason: String::new(),
            owner: false,
            administrator: None,
            target_administrator: None,
            highest_assigned_position: None,
            target_position: None,
            required_roles_complete: false,
            unavailable_facts: vec![],
        };
        if principal == Principal::Context {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "a requester or bot identity is required",
            ));
        }
        if principal == Principal::Bot {
            let me = self.api.request(Method::GET, "/me", None).await?;
            match id(&me["id"]) {
                Some(bot) if bot == actor => (),
                Some(_) => {
                    return Ok(report.finish(
                        AccessDecision::Denied,
                        "bot identity differs from authenticated /me",
                    ))
                }
                None => {
                    return Ok(report.finish(
                        AccessDecision::Unknown,
                        "authenticated bot identity is unavailable",
                    ))
                }
            }
        }
        let value = self
            .api
            .request(Method::GET, &format!("/servers/{server}"), None)
            .await?;
        if id(&value["server"]["id"]) != Some(server) {
            return Ok(report.finish(
                AccessDecision::Denied,
                "server response does not match the requested scope",
            ));
        }
        let server_data: Server = serde_json::from_value(value["server"].clone())?;
        report.owner = server_data.owner_id == actor;
        let inventory = self
            .api
            .request(Method::GET, &format!("/servers/{server}/roles"), None)
            .await?;
        let Some(rows) = inventory["roles"].as_array() else {
            return Ok(report.finish(AccessDecision::Unknown, "role inventory is unavailable"));
        };
        // A future paging contract needs an explicit implementation, not a claim
        // that one page proves a complete assigned-role inventory.
        if ["next", "next_cursor", "has_more", "cursor"]
            .iter()
            .any(|key| inventory.get(*key).is_some())
        {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "role response contains unsupported paging metadata",
            ));
        }
        let mut ids = HashSet::new();
        if rows
            .iter()
            .any(|row| id(&row["id"]).is_none_or(|role| !ids.insert(role)))
        {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "role inventory contains missing or duplicate identities",
            ));
        }
        if rows
            .iter()
            .any(|row| row.get("server_id").is_some_and(|v| id(v) != Some(server)))
        {
            return Ok(report.finish(
                AccessDecision::Denied,
                "role inventory includes a different server scope",
            ));
        }
        let Some(target_row) = rows.iter().find(|row| id(&row["id"]) == Some(target)) else {
            return Ok(report.finish(
                AccessDecision::Denied,
                "target role was not established in the scoped server inventory",
            ));
        };
        let target_role: Role = serde_json::from_value(target_row.clone())?;
        report.target_position = target_role.position;
        report.target_administrator = permission(target_row, "administrator");
        match target_row.get("auto") {
            Some(Value::Null) => (),
            Some(Value::String(kind)) if kind == "none" => (),
            Some(Value::String(kind)) if !kind.is_empty() => {
                return Ok(report.finish(
                    AccessDecision::Denied,
                    "policy forbids deleting everyone, automatic or system-managed roles",
                ))
            }
            _ => {
                return Ok(report.finish(
                    AccessDecision::Unknown,
                    "target auto classification is missing or unsupported",
                ))
            }
        }
        if report.owner && self.rules == RoleHierarchyRules::OwnerAdminStrict {
            report.required_roles_complete = true;
            return Ok(report.finish(
                AccessDecision::Allowed,
                "application owner rule permits this scoped, explicitly normal role",
            ));
        }
        let member_value = self
            .api
            .request(
                Method::GET,
                &format!("/servers/{server}/members/{actor}"),
                None,
            )
            .await?;
        if id(&member_value["member"]["id"]) != Some(actor) {
            return Ok(report.finish(
                AccessDecision::Denied,
                "member identity does not match the requested actor",
            ));
        }
        if member_value["member"]
            .get("server_id")
            .is_some_and(|value| id(value) != Some(server))
        {
            return Ok(report.finish(
                AccessDecision::Denied,
                "member response includes a different server scope",
            ));
        }
        if !member_value["member"]["roles"].is_array() {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "assigned-role IDs were not supplied",
            ));
        }
        let member: MemberDetail = serde_json::from_value(member_value["member"].clone())?;
        if member.member.roles.iter().collect::<HashSet<_>>().len() != member.member.roles.len()
            || member.member.roles.iter().any(|role| !ids.contains(role))
        {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "assigned-role inventory is incomplete or duplicated",
            ));
        }
        let assigned: Vec<_> = rows
            .iter()
            .filter(|row| id(&row["id"]).is_some_and(|id| member.member.roles.contains(&id)))
            .collect();
        if assigned.iter().any(|row| {
            permission(row, "administrator").is_none() || permission(row, "manage_roles").is_none()
        }) {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "assigned-role permission evidence is missing",
            ));
        }
        let roles: Vec<Role> = serde_json::from_value(inventory["roles"].clone())?;
        let snapshot = PermissionSnapshot {
            server: server_data,
            actor,
            member: Some(member),
            roles,
        };
        let permissions = snapshot.evaluate()?;
        report.administrator = Some(permissions.administrator);
        if !permissions.allows(&crate::Permission::new("manage_roles")) {
            return Ok(report.finish(
                AccessDecision::Denied,
                "fresh actor permissions do not grant Manage Roles",
            ));
        }
        if self.rules == RoleHierarchyRules::OwnerAdminStrict
            && report.target_administrator == Some(true)
        {
            return Ok(report.finish(AccessDecision::Denied,"application rule reserves administrator roles for the owner; administrator peers cannot modify one another"));
        }
        if report.target_administrator.is_none() {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "target administrator classification is unavailable",
            ));
        }
        if assigned.is_empty() {
            return Ok(report.finish(AccessDecision::Unknown,"no assigned-role position establishes the actor's hierarchy; implicit-role rank is undocumented"));
        }
        let positions: Option<Vec<i32>> = assigned
            .iter()
            .map(|row| row["position"].as_i64().and_then(|p| p.try_into().ok()))
            .collect();
        let Some(positions) = positions else {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "an assigned role is missing its position",
            ));
        };
        report.highest_assigned_position = positions.into_iter().max();
        report.required_roles_complete = true;
        let Some(target_position) = report.target_position else {
            return Ok(report.finish(
                AccessDecision::Unknown,
                "target role position is unavailable",
            ));
        };
        if self.rules == RoleHierarchyRules::PublicApi {
            if principal == Principal::Requester {
                return Ok(report.finish(AccessDecision::Unknown,"public bot-API documentation does not establish arbitrary requester's hierarchy authority"));
            }
            if report.highest_assigned_position == Some(target_position) {
                return Ok(report.finish(
                    AccessDecision::Unknown,
                    "equal-position tie ordering is undocumented",
                ));
            }
            if report.target_administrator == Some(true) && permissions.administrator {
                return Ok(report.finish(
                    AccessDecision::Unknown,
                    "administrator peer hierarchy behavior is undocumented",
                ));
            }
            if report
                .highest_assigned_position
                .is_some_and(|p| p <= target_position)
                && (report.owner || permissions.administrator)
            {
                return Ok(report.finish(
                    AccessDecision::Unknown,
                    "owner/administrator hierarchy bypass is undocumented",
                ));
            }
        }
        if report
            .highest_assigned_position
            .is_some_and(|p| p > target_position)
        {
            Ok(report.finish(AccessDecision::Allowed,"fresh assigned-role position is strictly above the target (larger positions are higher)"))
        } else {
            Ok(report.finish(AccessDecision::Denied,"policy requires a strictly higher assigned role; equal or lower positions cannot modify this target"))
        }
    }

    /// Reassess immediately, force fresh SDK permission reads, then issue one
    /// DELETE attempt. Custom Api implementations must support request_once;
    /// workers use the protocol 11 single-attempt transport.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
    /// use rappin::role_hierarchy::{execute_role_deletion, RoleHierarchyPolicy};
    /// async fn delete<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> rappin::Result<()> {
    ///     let receipt = RoleHierarchyPolicy::new(api).execute_deletion(operation, context).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn execute_deletion(
        &self,
        operation: &ValidatedOperation,
        context: &OperationContext,
    ) -> Result<OperationResult> {
        execute_role_deletion(self.api, operation, context, self).await
    }
}
#[async_trait::async_trait]
impl<A: Api + ?Sized> OperationPolicy for RoleHierarchyPolicy<'_, A> {
    async fn check(&self, q: &AccessQuestion<'_>) -> Result<CheckResolution> {
        if q.check != ExternalCheck::RoleHierarchy || q.operation != "delete_role" {
            return Ok(CheckResolution::unknown("this policy supplies role-deletion hierarchy evidence only; rollout and other checks require independent host evidence"));
        }
        if id(&q.arguments["id"]) != Some(q.server_id) {
            return Ok(CheckResolution::denied(
                "role operation does not match the host server scope",
            ));
        }
        let Some(actor) = q.actor_id else {
            return Ok(CheckResolution::unknown("actor identity is missing"));
        };
        let Some(target) = id(&q.arguments["role"]) else {
            return Ok(CheckResolution::unknown("target role identity is missing"));
        };
        Ok(self
            .assess(q.server_id, actor, target, q.principal)
            .await?
            .resolution())
    }
}

/// Compose an application policy with validated role deletion. Fresh permission
/// checks precede exactly one DELETE attempt; assessment is not a reservation.
/// # Example
///
/// ```no_run
/// use rappin::{Api, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
/// use rappin::role_hierarchy::{execute_role_deletion, RoleHierarchyPolicy};
/// async fn delete<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> rappin::Result<()> {
///     let receipt = execute_role_deletion(api, operation, context, policy).await?;
///     Ok(())
/// }
/// ```
pub async fn execute_role_deletion<A: Api + ?Sized>(
    api: &A,
    operation: &ValidatedOperation,
    context: &OperationContext,
    policy: &dyn OperationPolicy,
) -> Result<OperationResult> {
    if operation.name() != "delete_role" {
        return Err(Error::validation(
            "operation",
            "this helper executes only delete_role",
        ));
    }
    operation
        .execute_with_policy(
            &DeleteOnce(api),
            &context.clone().freshness(Freshness::Refresh),
            policy,
        )
        .await
}
fn id(value: &Value) -> Option<Uuid> {
    value.as_str().and_then(|s| s.parse().ok())
}
fn permission(row: &Value, key: &str) -> Option<bool> {
    if let Some(value) = row.get("permissions").and_then(|v| v.get(key)) {
        return value.as_bool();
    }
    match row.get("perms").or_else(|| row.get("permissions")) {
        Some(Value::Null) => Some(false),
        Some(Value::Object(map)) => map.get(key).map_or(Some(false), Value::as_bool),
        _ => None,
    }
}
struct DeleteOnce<'a, A: Api + ?Sized>(&'a A);
#[async_trait::async_trait]
impl<A: Api + ?Sized> Api for DeleteOnce<'_, A> {
    fn sanitize(&self, value: Value) -> Value {
        self.0.sanitize(value)
    }
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        if method == Method::DELETE {
            let result = self.0.request_once(method, path, body).await;
            result.map_err(|error| {
                let mut cause = error.to_remote();
                // Preserve the cause/outcome while making the workflow's no-replay
                // contract explicit, even when a response includes Retry-After.
                if let Some(original) = cause.details.get("retry_safety").cloned() {
                    cause
                        .details
                        .insert("transport_retry_safety".into(), original);
                }
                cause
                    .details
                    .insert("retry_safety".into(), json!("do_not_replay_workflow"));
                cause.into()
            })
        } else {
            self.0.request(method, path, body).await
        }
    }
}
