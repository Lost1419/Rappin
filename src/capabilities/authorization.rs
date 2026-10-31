use super::*;
use crate::{Api, ApiExt, Freshness, Permission, PermissionEvaluation, RemoteError};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessDecision {
    Allowed,
    Denied,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Principal {
    Requester,
    Bot,
    Context,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalCheck {
    ChannelPermissions,
    MessageContentRules,
    RoleHierarchy,
    GrantablePermissions,
    OverrideAuthority,
    CategoryScope,
    EventDestinationScope,
    TestServerAvailability,
}

/// Host-derived identities, never arguments supplied by the model.
#[derive(Debug, Clone)]
pub struct OperationContext {
    server_id: Uuid,
    requester_id: Uuid,
    freshness: Freshness,
    test_server: Option<bool>,
}
impl OperationContext {
    pub fn server(server_id: Uuid, requester_id: Uuid) -> Self {
        Self {
            server_id,
            requester_id,
            freshness: Freshness::Refresh,
            test_server: None,
        }
    }
    pub fn server_id(&self) -> Uuid {
        self.server_id
    }
    pub fn requester_id(&self) -> Uuid {
        self.requester_id
    }
    pub fn freshness(mut self, freshness: Freshness) -> Self {
        self.freshness = freshness;
        self
    }
    /// Only set from trusted host configuration, not a model argument.
    pub fn test_server_access(mut self, enabled: bool) -> Self {
        self.test_server = Some(enabled);
        self
    }
}

/// A question the generic SDK cannot establish from server permission bits.
pub struct AccessQuestion<'a> {
    pub operation: &'static str,
    pub check: ExternalCheck,
    pub principal: Principal,
    pub actor_id: Option<Uuid>,
    pub server_id: Uuid,
    pub channel_id: Option<Uuid>,
    pub permissions: &'a [&'static str],
    pub arguments: &'a Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResolution {
    pub decision: AccessDecision,
    pub reason: String,
}
impl CheckResolution {
    pub fn allowed(reason: impl Into<String>) -> Self {
        Self {
            decision: AccessDecision::Allowed,
            reason: reason.into(),
        }
    }
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            decision: AccessDecision::Denied,
            reason: reason.into(),
        }
    }
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self {
            decision: AccessDecision::Unknown,
            reason: reason.into(),
        }
    }
}
/// Application access facts, supplied independently of model output. Unknown
/// is the default; this is not a permission-bypass or a user confirmation flow.
#[async_trait::async_trait]
pub trait OperationPolicy: Send + Sync {
    async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution>;
}
pub struct NoAdditionalAccess;
#[async_trait::async_trait]
impl OperationPolicy for NoAdditionalAccess {
    async fn check(&self, _: &AccessQuestion<'_>) -> Result<CheckResolution> {
        Ok(CheckResolution::unknown(
            "host evidence is required for this check",
        ))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCheck {
    pub check: String,
    pub principal: Principal,
    pub decision: AccessDecision,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RemoteError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationReport {
    pub operation: String,
    pub server_id: Uuid,
    pub requester_id: Uuid,
    pub bot_id: Option<Uuid>,
    pub checks: Vec<AuthorizationCheck>,
}
impl AuthorizationReport {
    pub fn decision(&self) -> AccessDecision {
        if self
            .checks
            .iter()
            .any(|c| c.decision == AccessDecision::Denied)
        {
            AccessDecision::Denied
        } else if self
            .checks
            .iter()
            .any(|c| c.decision == AccessDecision::Unknown)
        {
            AccessDecision::Unknown
        } else {
            AccessDecision::Allowed
        }
    }
    pub fn allowed(&self) -> bool {
        self.decision() == AccessDecision::Allowed
    }
    fn record(
        &mut self,
        check: &str,
        principal: Principal,
        resolution: CheckResolution,
        error: Option<RemoteError>,
    ) {
        self.checks.push(AuthorizationCheck {
            check: check.into(),
            principal,
            decision: resolution.decision,
            reason: resolution.reason,
            error,
        });
    }
    fn failure(&mut self, check: &str, principal: Principal, error: YappinError) {
        self.record(
            check,
            principal,
            CheckResolution::unknown("the API could not establish this access fact"),
            Some(error.to_remote()),
        );
    }
}
fn new_report(operation: &ValidatedOperation, context: &OperationContext) -> AuthorizationReport {
    AuthorizationReport {
        operation: operation.name().into(),
        server_id: context.server_id,
        requester_id: context.requester_id,
        bot_id: None,
        checks: vec![],
    }
}

impl ValidatedOperation {
    pub(super) fn local_scope(&self, context: &OperationContext) -> AuthorizationReport {
        let mut report = new_report(self, context);
        if let OperationScope::Server(key) | OperationScope::OptionalServer(key) = self.entry.scope
        {
            if self.args[key]
                .as_str()
                .is_some_and(|id| id.parse::<Uuid>().ok() != Some(context.server_id))
            {
                report.record(
                    "server_scope",
                    Principal::Context,
                    CheckResolution::denied("operation targets a different server"),
                    None,
                );
            } else {
                report.record(
                    "server_scope",
                    Principal::Context,
                    CheckResolution::allowed("server argument is bound to the host context"),
                    None,
                );
            }
        }
        report
    }
    /// Read-only assessment, not a reservation. execute() reassesses immediately
    /// before execution; the API still enforces races and channel-specific rules.
    pub async fn assess<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
    ) -> Result<AuthorizationReport> {
        self.assess_with_policy(api, context, &NoAdditionalAccess)
            .await
    }
    pub async fn assess_with_policy<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
        policy: &dyn OperationPolicy,
    ) -> Result<AuthorizationReport> {
        let mut report = self.assess_inner(api, context, policy).await?;
        for check in &mut report.checks {
            check.reason = api
                .sanitize(Value::String(check.reason.clone()))
                .as_str()
                .unwrap_or("[REDACTED]")
                .into();
            if let Some(error) = &check.error {
                check.error = Some(serde_json::from_value(api.sanitize(error.to_json()))?);
            }
        }
        Ok(report)
    }
    async fn assess_inner<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
        policy: &dyn OperationPolicy,
    ) -> Result<AuthorizationReport> {
        let mut report = self.local_scope(context);
        if report.decision() == AccessDecision::Denied {
            return Ok(report);
        }
        match self.entry.availability.library {
            LibrarySupport::Offline => return Ok(report),
            LibrarySupport::CliRuntime | LibrarySupport::GlobalEventConsumer => {
                report.record("execution_runtime",Principal::Context,CheckResolution::unknown("this operation needs CLI receipt/diagnostic state or a global event consumer; it is unavailable to server-scoped bindings"),None);
                return Ok(report);
            }
            _ => (),
        }
        let mut channel = None;
        if let OperationScope::Channel(key) = self.entry.scope {
            let id = self.uuid(key);
            let response = api
                .request("GET".parse().unwrap(), &format!("/channels/{id}"), None)
                .await;
            let value = match response {
                Ok(value) => value,
                Err(error) => {
                    report.failure("channel_scope", Principal::Context, error);
                    return Ok(report);
                }
            };
            let row = &value["channel"];
            if row["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) != Some(id) {
                report.record(
                    "channel_scope",
                    Principal::Context,
                    CheckResolution::unknown("channel identity was not established"),
                    None,
                );
                return Ok(report);
            }
            match row["server_id"]
                .as_str()
                .and_then(|s| s.parse::<Uuid>().ok())
            {
                Some(server) if server == context.server_id => {
                    channel = Some(id);
                    report.record(
                        "channel_scope",
                        Principal::Context,
                        CheckResolution::allowed("channel belongs to the host server"),
                        None,
                    );
                }
                Some(_) => {
                    report.record(
                        "channel_scope",
                        Principal::Context,
                        CheckResolution::denied("channel belongs to a different server"),
                        None,
                    );
                    return Ok(report);
                }
                None => {
                    report.record(
                        "channel_scope",
                        Principal::Context,
                        CheckResolution::unknown(
                            "channel has no established server scope (including DMs)",
                        ),
                        None,
                    );
                    return Ok(report);
                }
            }
        }
        if !self
            .referenced_scopes(api, context, channel, &mut report)
            .await
        {
            return Ok(report);
        }
        if self.entry.availability.test_servers_only {
            match context.test_server {
                Some(true) => report.record(
                    "test_server_availability",
                    Principal::Context,
                    CheckResolution::allowed("test-server access attested by host configuration"),
                    None,
                ),
                Some(false) => {
                    report.record(
                        "test_server_availability",
                        Principal::Context,
                        CheckResolution::denied(
                            "test-server access is disabled in host configuration",
                        ),
                        None,
                    );
                    return Ok(report);
                }
                None => {
                    self.policy_check(
                        &mut report,
                        policy,
                        context,
                        channel,
                        ExternalCheck::TestServerAvailability,
                        Principal::Context,
                        None,
                        &[],
                    )
                    .await
                }
            }
        }
        let me = api.request("GET".parse().unwrap(), "/me", None).await;
        let bot = match me {
            Ok(value) => match value["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) {
                Some(id) => id,
                None => {
                    report.record(
                        "bot_identity",
                        Principal::Bot,
                        CheckResolution::unknown("authenticated bot identity was not returned"),
                        None,
                    );
                    return Ok(report);
                }
            },
            Err(error) => {
                report.failure("bot_identity", Principal::Bot, error);
                return Ok(report);
            }
        };
        report.bot_id = Some(bot);
        report.record(
            "bot_identity",
            Principal::Bot,
            CheckResolution::allowed("bot ID obtained through the authenticated API"),
            None,
        );
        let needs_message = matches!(
            self.entry.permissions.requester,
            PermissionRule::MessageAuthorOr(_) | PermissionRule::BotMessageAuthor
        ) || matches!(
            self.entry.permissions.bot,
            PermissionRule::MessageAuthorOr(_) | PermissionRule::BotMessageAuthor
        );
        let mut author = None;
        if needs_message {
            let channel = channel.expect("message operation has a verified channel");
            let message = self.uuid("message");
            match api
                .request(
                    "GET".parse().unwrap(),
                    &format!("/channels/{channel}/messages/{message}"),
                    None,
                )
                .await
            {
                Ok(value) => {
                    let value = &value["message"];
                    if value["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) != Some(message)
                        || value["channel_id"]
                            .as_str()
                            .and_then(|s| s.parse::<Uuid>().ok())
                            != Some(channel)
                    {
                        report.record(
                            "message_scope",
                            Principal::Context,
                            CheckResolution::denied(
                                "message identity/channel do not match the requested scope",
                            ),
                            None,
                        );
                        return Ok(report);
                    }
                    author = value["author_id"]
                        .as_str()
                        .and_then(|s| s.parse::<Uuid>().ok());
                }
                Err(error) => report.failure("message_scope", Principal::Context, error),
            }
        }
        for (principal, actor, rule, channel_perms) in [
            (
                Principal::Requester,
                context.requester_id,
                &self.entry.permissions.requester,
                &self.entry.permissions.requester_channel,
            ),
            (
                Principal::Bot,
                bot,
                &self.entry.permissions.bot,
                &self.entry.permissions.bot_channel,
            ),
        ] {
            match api
                .evaluate_permissions(context.server_id, actor, context.freshness)
                .await
            {
                Ok(permissions) => {
                    let decision = rule_decision(rule, &permissions, author, bot);
                    report.record(
                        "server_permissions",
                        principal,
                        CheckResolution {
                            decision,
                            reason: format!("server rule: {rule:?}"),
                        },
                        None,
                    );
                }
                Err(error) => report.failure("server_permissions", principal, error),
            }
            if !channel_perms.is_empty() {
                if principal == Principal::Bot && channel_perms.as_slice() == ["view_channel"] {
                    report.record(
                        "channel_permissions",
                        principal,
                        CheckResolution::allowed("channel read succeeded through the bot API"),
                        None,
                    );
                } else {
                    self.policy_check(
                        &mut report,
                        policy,
                        context,
                        channel,
                        ExternalCheck::ChannelPermissions,
                        principal,
                        Some(actor),
                        channel_perms,
                    )
                    .await;
                }
            }
            for check in &self.entry.permissions.additional_checks {
                if matches!(check, ExternalCheck::CategoryScope) {
                    continue;
                }
                self.policy_check(
                    &mut report,
                    policy,
                    context,
                    channel,
                    *check,
                    principal,
                    Some(actor),
                    &[],
                )
                .await;
            }
        }
        Ok(report)
    }

    async fn referenced_scopes<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
        channel: Option<Uuid>,
        report: &mut AuthorizationReport,
    ) -> bool {
        let server = context.server_id;
        let role = self.args["role"]
            .as_str()
            .or_else(|| self.args["target"].as_str().filter(|s| *s != "everyone"));
        if let Some(role) = role {
            match api
                .request(
                    "GET".parse().unwrap(),
                    &format!("/servers/{server}/roles"),
                    None,
                )
                .await
            {
                Ok(value) => match value["roles"].as_array() {
                    Some(roles)
                        if roles.iter().any(|r| {
                            r["id"].as_str().and_then(|s| s.parse::<Uuid>().ok())
                                == role.parse().ok()
                        }) =>
                    {
                        report.record(
                            "role_scope",
                            Principal::Context,
                            CheckResolution::allowed("role is listed in the host server"),
                            None,
                        )
                    }
                    Some(_) => {
                        report.record(
                            "role_scope",
                            Principal::Context,
                            CheckResolution::denied(
                                "role is outside the visible host-server scope",
                            ),
                            None,
                        );
                        return false;
                    }
                    None => {
                        report.record(
                            "role_scope",
                            Principal::Context,
                            CheckResolution::unknown("role list was not returned"),
                            None,
                        );
                        return false;
                    }
                },
                Err(error) => {
                    report.failure("role_scope", Principal::Context, error);
                    return false;
                }
            }
        }
        if let Some(category) = self.args["category"].as_str() {
            match api
                .request(
                    "GET".parse().unwrap(),
                    &format!("/servers/{server}/channels"),
                    None,
                )
                .await
            {
                Ok(value) => match value["categories"].as_array() {
                    Some(categories)
                        if categories.iter().any(|c| {
                            c["id"].as_str().and_then(|s| s.parse::<Uuid>().ok())
                                == category.parse().ok()
                        }) =>
                    {
                        report.record(
                            "category_scope",
                            Principal::Context,
                            CheckResolution::allowed("category is listed in the host server"),
                            None,
                        )
                    }
                    Some(_) => {
                        report.record(
                            "category_scope",
                            Principal::Context,
                            CheckResolution::denied(
                                "category is outside the visible host-server scope",
                            ),
                            None,
                        );
                        return false;
                    }
                    None => {
                        report.record(
                            "category_scope",
                            Principal::Context,
                            CheckResolution::unknown("category list was not returned"),
                            None,
                        );
                        return false;
                    }
                },
                Err(error) => {
                    report.failure("category_scope", Principal::Context, error);
                    return false;
                }
            }
        }
        if self.name() == "create_server_event" {
            for key in ["channel_id", "announce_channel_id"] {
                if let Some(id) = self.args["body"][key].as_str() {
                    let id: Uuid = id.parse().expect("validated event channel");
                    match api
                        .request("GET".parse().unwrap(), &format!("/channels/{id}"), None)
                        .await
                    {
                        Ok(value) => {
                            let row = &value["channel"];
                            if row["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) != Some(id)
                                || row["server_id"]
                                    .as_str()
                                    .and_then(|s| s.parse::<Uuid>().ok())
                                    != Some(server)
                            {
                                report.record(
                                    "event_destination_scope",
                                    Principal::Context,
                                    CheckResolution::denied(
                                        "event channel identity/server do not match the host scope",
                                    ),
                                    None,
                                );
                                return false;
                            }
                            if key == "channel_id"
                                && row["kind"] != "voice"
                                && row["type"] != "voice"
                            {
                                report.record(
                                    "event_destination_scope",
                                    Principal::Context,
                                    CheckResolution::unknown(
                                        "event destination was not established as a voice channel",
                                    ),
                                    None,
                                );
                                return false;
                            }
                        }
                        Err(error) => {
                            report.failure("event_destination_scope", Principal::Context, error);
                            return false;
                        }
                    }
                }
            }
            report.record("event_destination_scope",Principal::Context,CheckResolution::allowed("event channel references are within the host scope; external locations have no channel reference"),None);
        }
        if self.name() == "send_message" {
            if let Some(reply) = self.writes.first().and_then(|v| v["reply_to"].as_str()) {
                let reply: Uuid = reply.parse().expect("typed reply ID");
                let channel = channel.expect("verified message channel");
                match api
                    .request(
                        "GET".parse().unwrap(),
                        &format!("/channels/{channel}/messages/{reply}"),
                        None,
                    )
                    .await
                {
                    Ok(value) => {
                        let row = &value["message"];
                        if row["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) != Some(reply)
                            || row["channel_id"]
                                .as_str()
                                .and_then(|s| s.parse::<Uuid>().ok())
                                != Some(channel)
                        {
                            report.record(
                                "reply_scope",
                                Principal::Context,
                                CheckResolution::denied(
                                    "reply target is outside the requested channel",
                                ),
                                None,
                            );
                            return false;
                        }
                        report.record(
                            "reply_scope",
                            Principal::Context,
                            CheckResolution::allowed(
                                "reply target belongs to the requested channel",
                            ),
                            None,
                        );
                    }
                    Err(error) => {
                        report.failure("reply_scope", Principal::Context, error);
                        return false;
                    }
                }
            }
        }
        true
    }
    #[allow(clippy::too_many_arguments)]
    async fn policy_check(
        &self,
        report: &mut AuthorizationReport,
        policy: &dyn OperationPolicy,
        context: &OperationContext,
        channel: Option<Uuid>,
        check: ExternalCheck,
        principal: Principal,
        actor: Option<Uuid>,
        permissions: &[&'static str],
    ) {
        let question = AccessQuestion {
            operation: self.entry.name,
            check,
            principal,
            actor_id: actor,
            server_id: context.server_id,
            channel_id: channel,
            permissions,
            arguments: &self.args,
        };
        let name = serde_json::to_value(check)
            .expect("check enum")
            .as_str()
            .expect("check name")
            .to_owned();
        match policy.check(&question).await {
            Ok(resolution) => report.record(&name, principal, resolution, None),
            Err(error) => report.failure(&name, principal, error),
        }
    }
}
fn rule_decision(
    rule: &PermissionRule,
    permissions: &PermissionEvaluation,
    author: Option<Uuid>,
    bot: Uuid,
) -> AccessDecision {
    let allows = |key: &&'static str| permissions.allows(&Permission::new(*key));
    let allowed = match rule {
        PermissionRule::NotRequired => true,
        PermissionRule::Member => true,
        PermissionRule::All(keys) => keys.iter().all(allows),
        PermissionRule::Any(keys) => keys.iter().any(allows),
        PermissionRule::MessageAuthorOr(key) => {
            if author.is_none() && !permissions.allows(&Permission::new(*key)) {
                return AccessDecision::Unknown;
            }
            author == Some(permissions.actor_id) || permissions.allows(&Permission::new(*key))
        }
        PermissionRule::BotMessageAuthor => {
            let Some(author) = author else {
                return AccessDecision::Unknown;
            };
            author == bot
        }
    };
    if allowed {
        AccessDecision::Allowed
    } else {
        AccessDecision::Denied
    }
}
