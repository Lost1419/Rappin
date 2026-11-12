//! Independent requester/bot assessment with explicit Denied and Unknown requirements.
use super::*;
use crate::{Api, ApiExt, Freshness, Permission, PermissionEvaluation, RemoteError};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Three-valued access decision; Unknown is never an implicit grant.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{AccessDecision, CheckResolution};
/// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
/// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
/// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
/// ```
pub enum AccessDecision {
    /// Required evidence establishes access under the selected policy.
    Allowed,
    /// Evidence establishes that a required condition is not satisfied.
    Denied,
    /// Evidence or authoritative platform semantics are unavailable.
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Requester, bot or non-actor context being assessed.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{AccessDecision, CheckResolution};
/// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
/// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
/// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
/// ```
pub enum Principal {
    /// Original requester, assessed independently from the bot.
    Requester,
    /// Authenticated bot identity from /me.
    Bot,
    /// Scope, availability or other non-actor requirement.
    Context,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Fact that generic server-wide permission evaluation cannot establish.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{AccessDecision, CheckResolution};
/// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
/// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
/// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
/// ```
pub enum ExternalCheck {
    /// Effective channel access, independently for each principal.
    ChannelPermissions,
    /// Platform-specific content restrictions beyond local payload validation.
    MessageContentRules,
    /// Authority over a target role under known hierarchy rules.
    RoleHierarchy,
    /// Whether the actor may grant the requested permission set.
    GrantablePermissions,
    /// Authority to modify the specified resource override.
    OverrideAuthority,
    /// Category belongs to the operation's bound server.
    CategoryScope,
    /// Effective access to a destination or edited category.
    CategoryPermissions,
    /// Scheduled-event destination belongs to the bound server.
    EventDestinationScope,
    /// Trusted evidence of the endpoint's rollout/test-server eligibility.
    TestServerAvailability,
}

/// Host-derived identities, never arguments supplied by the model.
#[derive(Debug, Clone)]
/// # Example
///
/// ```rust
/// use rappin::{Freshness, capabilities::OperationContext};
/// let server = uuid::Uuid::nil();
/// let requester = uuid::Uuid::from_u128(1);
/// let context = OperationContext::server(server, requester).freshness(Freshness::Refresh);
/// assert_eq!(context.server_id(), server);
/// assert_eq!(context.requester_id(), requester);
/// // Derive identities from authenticated dispatch, never operation arguments.
/// ```
pub struct OperationContext {
    server_id: Uuid,
    requester_id: Uuid,
    freshness: Freshness,
    test_server: Option<bool>,
}
impl OperationContext {
    /// Bind host-authenticated server and requester identities; defaults to fresh evidence.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, capabilities::OperationContext};
    /// let server = uuid::Uuid::nil();
    /// let requester = uuid::Uuid::from_u128(1);
    /// let context = OperationContext::server(server, requester).freshness(Freshness::Refresh);
    /// assert_eq!(context.server_id(), server);
    /// assert_eq!(context.requester_id(), requester);
    /// // Derive identities from authenticated dispatch, never operation arguments.
    /// ```
    pub fn server(server_id: Uuid, requester_id: Uuid) -> Self {
        Self {
            server_id,
            requester_id,
            freshness: Freshness::Refresh,
            test_server: None,
        }
    }
    /// Bound host server identity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, capabilities::OperationContext};
    /// let server = uuid::Uuid::nil();
    /// let requester = uuid::Uuid::from_u128(1);
    /// let context = OperationContext::server(server, requester).freshness(Freshness::Refresh);
    /// assert_eq!(context.server_id(), server);
    /// assert_eq!(context.requester_id(), requester);
    /// // Derive identities from authenticated dispatch, never operation arguments.
    /// ```
    pub fn server_id(&self) -> Uuid {
        self.server_id
    }
    /// Original authenticated requester identity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, capabilities::OperationContext};
    /// let server = uuid::Uuid::nil();
    /// let requester = uuid::Uuid::from_u128(1);
    /// let context = OperationContext::server(server, requester).freshness(Freshness::Refresh);
    /// assert_eq!(context.server_id(), server);
    /// assert_eq!(context.requester_id(), requester);
    /// // Derive identities from authenticated dispatch, never operation arguments.
    /// ```
    pub fn requester_id(&self) -> Uuid {
        self.requester_id
    }
    /// Choose fresh or explicitly cached membership/role evidence.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, capabilities::OperationContext};
    /// let server = uuid::Uuid::nil();
    /// let requester = uuid::Uuid::from_u128(1);
    /// let context = OperationContext::server(server, requester).freshness(Freshness::Refresh);
    /// assert_eq!(context.server_id(), server);
    /// assert_eq!(context.requester_id(), requester);
    /// // Derive identities from authenticated dispatch, never operation arguments.
    /// ```
    pub fn freshness(mut self, freshness: Freshness) -> Self {
        self.freshness = freshness;
        self
    }
    /// Only set from trusted host configuration, not a model argument.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, capabilities::OperationContext};
    /// let server = uuid::Uuid::nil();
    /// let requester = uuid::Uuid::from_u128(1);
    /// let context = OperationContext::server(server, requester).freshness(Freshness::Refresh);
    /// assert_eq!(context.server_id(), server);
    /// assert_eq!(context.requester_id(), requester);
    /// // Derive identities from authenticated dispatch, never operation arguments.
    /// // Only after the operator independently established endpoint eligibility:
    /// let context = context.test_server_access(true);
    /// ```
    pub fn test_server_access(mut self, enabled: bool) -> Self {
        self.test_server = Some(enabled);
        self
    }
}

/// A question the generic SDK cannot establish from server permission bits.
/// # Example
///
/// ```rust
/// use rappin::{Result, capabilities::{OperationPolicy, AccessQuestion, CheckResolution}};
/// struct ConservativePolicy;
/// #[async_trait::async_trait]
/// impl OperationPolicy for ConservativePolicy {
///     async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution> {
///         // Obtain authoritative evidence for question.principal and resource here.
///         // A successful bot read does not establish requester effective access.
///         Ok(CheckResolution::unknown(format!("No evidence for {:?}", question.check)))
///     }
/// }
/// ```
pub struct AccessQuestion<'a> {
    /// Canonical validated operation name.
    pub operation: &'static str,
    /// Specific external fact to establish.
    pub check: ExternalCheck,
    /// Requester, bot or context to assess independently.
    pub principal: Principal,
    /// Scoped actor identity, when the question concerns an actor.
    pub actor_id: Option<Uuid>,
    /// Bound host server.
    pub server_id: Uuid,
    /// Resolved channel, when relevant.
    pub channel_id: Option<Uuid>,
    /// Effective resource permissions required by this question.
    pub permissions: &'a [&'static str],
    /// Validated operation arguments, not identity or permission grants.
    pub arguments: &'a Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
/// One policy answer and its evidence explanation.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{AccessDecision, CheckResolution};
/// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
/// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
/// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
/// ```
pub struct CheckResolution {
    /// Explicit grant, denial or unresolved fact.
    pub decision: AccessDecision,
    /// Evidence-based explanation supplied by the host policy.
    pub reason: String,
}
impl CheckResolution {
    /// Return an explicit allowed policy answer with its reason.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::{AccessDecision, CheckResolution};
    /// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
    /// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
    /// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
    /// ```
    pub fn allowed(reason: impl Into<String>) -> Self {
        Self {
            decision: AccessDecision::Allowed,
            reason: reason.into(),
        }
    }
    /// Return an explicit denied policy answer with its reason.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::{AccessDecision, CheckResolution};
    /// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
    /// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
    /// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
    /// ```
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            decision: AccessDecision::Denied,
            reason: reason.into(),
        }
    }
    /// Return an explicit unknown policy answer with its reason.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::{AccessDecision, CheckResolution};
    /// assert_eq!(CheckResolution::allowed("Established by current evidence").decision, AccessDecision::Allowed);
    /// assert_eq!(CheckResolution::denied("Revoked").decision, AccessDecision::Denied);
    /// assert_eq!(CheckResolution::unknown("No authoritative effective access data").decision, AccessDecision::Unknown);
    /// ```
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
/// # Example
///
/// ```rust
/// use rappin::{Result, capabilities::{OperationPolicy, AccessQuestion, CheckResolution}};
/// struct ConservativePolicy;
/// #[async_trait::async_trait]
/// impl OperationPolicy for ConservativePolicy {
///     async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution> {
///         // Obtain authoritative evidence for question.principal and resource here.
///         // A successful bot read does not establish requester effective access.
///         Ok(CheckResolution::unknown(format!("No evidence for {:?}", question.check)))
///     }
/// }
/// ```
pub trait OperationPolicy: Send + Sync {
    /// Evaluate only the requested fact for the specified principal; unavailable evidence must remain Unknown.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Result, capabilities::{OperationPolicy, AccessQuestion, CheckResolution}};
    /// struct ConservativePolicy;
    /// #[async_trait::async_trait]
    /// impl OperationPolicy for ConservativePolicy {
    ///     async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution> {
    ///         // Obtain authoritative evidence for question.principal and resource here.
    ///         // A successful bot read does not establish requester effective access.
    ///         Ok(CheckResolution::unknown(format!("No evidence for {:?}", question.check)))
    ///     }
    /// }
    /// ```
    async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution>;
}
/// Default policy returning Unknown for every external access fact.
///
/// # Example
///
/// ```rust
/// use rappin::{Result, capabilities::{OperationPolicy, AccessQuestion, CheckResolution}};
/// struct ConservativePolicy;
/// #[async_trait::async_trait]
/// impl OperationPolicy for ConservativePolicy {
///     async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution> {
///         // Obtain authoritative evidence for question.principal and resource here.
///         // A successful bot read does not establish requester effective access.
///         Ok(CheckResolution::unknown(format!("No evidence for {:?}", question.check)))
///     }
/// }
/// ```
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
/// Read-only access evidence; execution always reassesses and the API remains authoritative for races.
///
/// # Example
///
/// ```no_run
/// use rappin::{Api, Result, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
/// async fn execute<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> Result<()> {
///     let report = operation.assess_with_policy(api, context, policy).await?;
///     if !report.allowed() { return Ok(()); } // Show unresolved/denied checks to the caller.
///     // Execution reassesses both principals; assessment is not a reservation.
///     let result = operation.execute_with_policy(api, context, policy).await?;
///     println!("{} completed", result.operation);
///     Ok(())
/// }
/// ```
pub struct AuthorizationCheck {
    /// Stable check label in this assessment.
    pub check: String,
    /// Principal whose condition was assessed.
    pub principal: Principal,
    /// Allowed, Denied or Unknown for this check.
    pub decision: AccessDecision,
    /// Explanation including unavailable evidence.
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Structured underlying failure when evidence could not be fetched.
    pub error: Option<RemoteError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Read-only access evidence; execution always reassesses and the API remains authoritative for races.
///
/// # Example
///
/// ```no_run
/// use rappin::{Api, Result, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
/// async fn execute<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> Result<()> {
///     let report = operation.assess_with_policy(api, context, policy).await?;
///     if !report.allowed() { return Ok(()); } // Show unresolved/denied checks to the caller.
///     // Execution reassesses both principals; assessment is not a reservation.
///     let result = operation.execute_with_policy(api, context, policy).await?;
///     println!("{} completed", result.operation);
///     Ok(())
/// }
/// ```
pub struct AuthorizationReport {
    /// Canonical operation assessed.
    pub operation: String,
    /// Server scope checked by the assessment.
    pub server_id: Uuid,
    /// Authenticated requester assessed independently.
    pub requester_id: Uuid,
    /// Authenticated bot identity, when /me established it.
    pub bot_id: Option<Uuid>,
    /// Individual scope, permission and external-policy decisions.
    pub checks: Vec<AuthorizationCheck>,
}
impl AuthorizationReport {
    /// Denied takes precedence over Unknown; only all-Allowed checks yield Allowed.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, Result, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
    /// async fn execute<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> Result<()> {
    ///     let report = operation.assess_with_policy(api, context, policy).await?;
    ///     if !report.allowed() { return Ok(()); } // Show unresolved/denied checks to the caller.
    ///     // Execution reassesses both principals; assessment is not a reservation.
    ///     let result = operation.execute_with_policy(api, context, policy).await?;
    ///     println!("{} completed", result.operation);
    ///     Ok(())
    /// }
    /// ```
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
    /// Whether every recorded requirement is Allowed; Unknown blocks execution.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, Result, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
    /// async fn execute<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> Result<()> {
    ///     let report = operation.assess_with_policy(api, context, policy).await?;
    ///     if !report.allowed() { return Ok(()); } // Show unresolved/denied checks to the caller.
    ///     // Execution reassesses both principals; assessment is not a reservation.
    ///     let result = operation.execute_with_policy(api, context, policy).await?;
    ///     println!("{} completed", result.operation);
    ///     Ok(())
    /// }
    /// ```
    pub fn allowed(&self) -> bool {
        self.decision() == AccessDecision::Allowed
    }
    pub(super) fn record(
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
    pub(super) fn failure(&mut self, check: &str, principal: Principal, error: YappinError) {
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, Result, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
    /// async fn execute<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> Result<()> {
    ///     let report = operation.assess(api, context).await?;
    ///     if !report.allowed() { return Ok(()); } // Show unresolved/denied checks to the caller.
    ///     // Execution reassesses both principals; assessment is not a reservation.
    ///     let result = operation.execute_with_policy(api, context, policy).await?;
    ///     println!("{} completed", result.operation);
    ///     Ok(())
    /// }
    /// ```
    pub async fn assess<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
    ) -> Result<AuthorizationReport> {
        self.assess_with_policy(api, context, &NoAdditionalAccess)
            .await
    }
    /// Read current scope/authorization evidence without executing a mutation. This is advisory and does not reserve access.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Api, Result, capabilities::{ValidatedOperation, OperationContext, OperationPolicy}};
    /// async fn execute<A: Api + ?Sized>(api: &A, operation: &ValidatedOperation, context: &OperationContext, policy: &dyn OperationPolicy) -> Result<()> {
    ///     let report = operation.assess_with_policy(api, context, policy).await?;
    ///     if !report.allowed() { return Ok(()); } // Show unresolved/denied checks to the caller.
    ///     // Execution reassesses both principals; assessment is not a reservation.
    ///     let result = operation.execute_with_policy(api, context, policy).await?;
    ///     println!("{} completed", result.operation);
    ///     Ok(())
    /// }
    /// ```
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
        if matches!(self.name(), "get_api_version" | "get_api_changelog") {
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
        let sticker = self.args["sticker"]
            .as_str()
            .or_else(|| self.args["body"]["sticker_id"].as_str());
        if self.entry.availability.test_servers_only || sticker.is_some() {
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
        let me = match report.bot_id {
            Some(bot) => Ok(json!({"id":bot})),
            None => api.request("GET".parse().unwrap(), "/me", None).await,
        };
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
        let needs_message = self.name() == "get_message_emojis"
            || matches!(
                self.entry.permissions.requester,
                PermissionRule::MessageAuthorOr(_) | PermissionRule::BotMessageAuthor
            )
            || matches!(
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
                if *check == ExternalCheck::CategoryPermissions
                    && matches!(self.name(), "create_channel" | "edit_channel")
                    && self.args["body"]["category_id"].as_str().is_none()
                {
                    continue;
                }
                if *check == ExternalCheck::OverrideAuthority
                    && self.name() == "edit_channel"
                    && !["private", "role_ids", "post_role_ids"]
                        .iter()
                        .any(|key| self.args["body"].get(key).is_some())
                {
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
        if self.name() == "resolve_id" {
            let id = self.uuid("id");
            match api
                .request("GET".parse().unwrap(), &format!("/resolve/{id}"), None)
                .await
            {
                Ok(value) => {
                    let valid =
                        value["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) == Some(id);
                    let scope = value["server_id"]
                        .as_str()
                        .and_then(|s| s.parse::<Uuid>().ok());
                    if !valid || scope.is_some_and(|scope| scope != server) {
                        report.record(
                            "resolved_scope",
                            Principal::Context,
                            CheckResolution::denied("resolved identity is outside the bound scope"),
                            None,
                        );
                        return false;
                    }
                    if scope.is_none() {
                        report.record("resolved_scope", Principal::Context, CheckResolution::unknown("resolver did not establish the bound server; use the raw typed API for a host-authorized global lookup"), None);
                        return false;
                    }
                    report.record(
                        "resolved_scope",
                        Principal::Context,
                        CheckResolution::allowed("resolved resource belongs to the bound server"),
                        None,
                    );
                }
                Err(error) => {
                    report.failure("resolved_scope", Principal::Context, error);
                    return false;
                }
            }
        }
        if let Some(sticker) = self.args["sticker"]
            .as_str()
            .or_else(|| self.args["body"]["sticker_id"].as_str())
        {
            match api
                .request("GET".parse().unwrap(), &format!("/resolve/{sticker}"), None)
                .await
            {
                Ok(value)
                    if value["id"].as_str() == Some(sticker)
                        && value["object"] == "sticker"
                        && value["server_id"]
                            .as_str()
                            .and_then(|s| s.parse::<Uuid>().ok())
                            == Some(server) =>
                {
                    report.record(
                        "sticker_scope",
                        Principal::Context,
                        CheckResolution::allowed("sticker belongs to the bound server"),
                        None,
                    );
                }
                Ok(_) => {
                    report.record(
                        "sticker_scope",
                        Principal::Context,
                        CheckResolution::denied("sticker identity or server does not match"),
                        None,
                    );
                    return false;
                }
                Err(error) => {
                    report.failure("sticker_scope", Principal::Context, error);
                    return false;
                }
            }
        }
        if !self.platform_scopes(api, context, channel, report).await {
            return false;
        }
        let mut roles: Vec<&str> = self.args["role"]
            .as_str()
            .or_else(|| self.args["target"].as_str().filter(|s| *s != "everyone"))
            .into_iter()
            .collect();
        if self.name() == "reorder_roles" {
            if let Some(ids) = self.args["body"]["ids"].as_array() {
                roles.extend(ids.iter().filter_map(Value::as_str));
            } else if let Some(positions) = self.args["body"].as_array() {
                roles.extend(
                    positions
                        .iter()
                        .filter_map(|position| position["id"].as_str()),
                );
            }
        }
        if self.name() == "edit_channel" {
            for key in ["role_ids", "post_role_ids"] {
                if let Some(ids) = self.args["body"][key].as_array() {
                    roles.extend(ids.iter().filter_map(Value::as_str));
                }
            }
        }
        roles.sort_unstable();
        roles.dedup();
        // Fetch one server role list for all references in a channel edit.
        if !roles.is_empty() {
            match api
                .request(
                    "GET".parse().unwrap(),
                    &format!("/servers/{server}/roles"),
                    None,
                )
                .await
            {
                Ok(value) => match value["roles"].as_array() {
                    Some(listed)
                        if roles.iter().all(|role| {
                            listed.iter().any(|row| {
                                row["id"].as_str().and_then(|s| s.parse::<Uuid>().ok())
                                    == role.parse().ok()
                            })
                        }) =>
                    {
                        report.record(
                            "role_scope",
                            Principal::Context,
                            CheckResolution::allowed("roles are listed in the host server"),
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
        if let Some(category) = self.args["category"].as_str().or_else(|| {
            matches!(self.name(), "create_channel" | "edit_channel")
                .then(|| self.args["body"]["category_id"].as_str())
                .flatten()
        }) {
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
        if matches!(self.name(), "send_message" | "send_files") {
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
