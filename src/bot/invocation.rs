//! Checked invocation within the lifetime of an authenticated message dispatch.
use super::{failure, BotHandle, Cog, CommandDef, MessageContext, MessageDisposition};
use crate::{Error, MessageEvent, Result};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc};

tokio::task_local! { static ORIGIN: Origin; }
#[derive(Clone)]
struct Origin {
    bot: BotHandle,
    event: Arc<MessageEvent>,
    depth: usize,
    gate_running: bool,
    operation_id: Option<OperationId>,
}
/// Consumer-owned durable operation key, separate from authenticated identity.
/// Values are 1..=128 UTF-8 bytes with no control characters. No normalization,
/// uniqueness, deduplication, permission grant or replay policy is implied.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
/// # Example
///
/// ```rust
/// use rappin::OperationId;
/// let id = OperationId::new("request:42:step:1")?;
/// assert_eq!(id.as_str(), "request:42:step:1");
/// assert!(OperationId::new("invalid\nkey").is_err());
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct OperationId(String);
impl OperationId {
    /// Maximum UTF-8 byte length accepted for a consumer operation key.
    pub const MAX_BYTES: usize = 128;
    /// Validate an opaque consumer key; no authority, uniqueness or remote idempotency is implied.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::OperationId;
    /// let id = OperationId::new("request:42:step:1")?;
    /// assert_eq!(id.as_str(), "request:42:step:1");
    /// assert!(OperationId::new("invalid\nkey").is_err());
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > Self::MAX_BYTES || value.chars().any(char::is_control)
        {
            return Err(Error::validation(
                "invocation.operation_id",
                "requires 1 to 128 UTF-8 bytes without control characters",
            ));
        }
        Ok(Self(value))
    }
    /// Borrow the exact unnormalized UTF-8 key.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::OperationId;
    /// let id = OperationId::new("request:42:step:1")?;
    /// assert_eq!(id.as_str(), "request:42:step:1");
    /// assert!(OperationId::new("invalid\nkey").is_err());
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for OperationId {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}
impl From<OperationId> for String {
    fn from(value: OperationId) -> Self {
        value.0
    }
}

pub(super) fn operation_id(ctx: &MessageContext) -> Option<OperationId> {
    ORIGIN
        .try_with(|origin| {
            (Arc::ptr_eq(&ctx.client, &origin.bot.inner.client)
                && ctx
                    .bot
                    .as_ref()
                    .is_some_and(|bot| Arc::ptr_eq(&bot.inner, &origin.bot.inner))
                && &ctx.event == origin.event.as_ref())
            .then(|| origin.operation_id.clone())
            .flatten()
        })
        .ok()
        .flatten()
}

/// Maximum number of command bodies/check chains active in one invocation lineage.
pub const MAX_INVOCATION_DEPTH: usize = 8;

pub(super) async fn with_origin<T>(
    bot: &BotHandle,
    event: &MessageEvent,
    future: impl Future<Output = T>,
) -> T {
    ORIGIN
        .scope(
            Origin {
                bot: bot.clone(),
                event: Arc::new(event.clone()),
                depth: 0,
                gate_running: false,
                operation_id: None,
            },
            future,
        )
        .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// Completion state of one explicit invocation; incomplete outcomes never imply replay safety.
///
/// # Example
///
/// ```rust
/// use rappin::{InvocationOutcome, InvocationStatus, Result};
/// fn propagate(outcome: InvocationOutcome) -> Result<()> {
///     if outcome.status != InvocationStatus::Completed {
///         eprintln!("Incomplete {}, operation {:?}, effects possible: {}", outcome.command, outcome.operation_id(), outcome.effects_possible);
///     }
///     // Preserve the original cause and leave outer reply/checkpoint ownership explicit.
///     outcome.into_result()
/// }
/// ```
pub enum InvocationStatus {
    /// No installed command matched the requested name/alias.
    NotFound,
    /// A parsing, scope or authorization requirement rejected the invocation.
    Denied,
    /// The installed handler returned successful completion.
    Completed,
    /// Preparation became stale, arbitrary code failed, or processing did not complete.
    Failed,
}

/// Result ownership belongs to the caller: invocation sends no automatic error
/// replies and calls no failure callbacks. Check/handler code can still reply.
#[derive(Debug)]
#[must_use = "propagate incomplete invocation or explicitly reconcile it"]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::{InvocationOutcome, InvocationStatus, Result};
/// fn propagate(outcome: InvocationOutcome) -> Result<()> {
///     if outcome.status != InvocationStatus::Completed {
///         eprintln!("Incomplete {}, operation {:?}, effects possible: {}", outcome.command, outcome.operation_id(), outcome.effects_possible);
///     }
///     // Preserve the original cause and leave outer reply/checkpoint ownership explicit.
///     outcome.into_result()
/// }
/// ```
pub struct InvocationOutcome {
    /// Explicit not-found, denied, completed or failed disposition.
    pub status: InvocationStatus,
    /// Requested or resolved installed command name.
    pub command: String,
    /// Conservative: arbitrary cog/check/handler code or the message gate ran.
    pub effects_possible: bool,
    /// Original structured cause when one is available.
    pub error: Option<Error>,
    operation_id: Option<OperationId>,
}
impl InvocationOutcome {
    /// Consumer operation key, including for rejected/stale/incomplete calls.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{InvocationOutcome, InvocationStatus, Result};
    /// fn propagate(outcome: InvocationOutcome) -> Result<()> {
    ///     if outcome.status != InvocationStatus::Completed {
    ///         eprintln!("Incomplete {}, operation {:?}, effects possible: {}", outcome.command, outcome.operation_id(), outcome.effects_possible);
    ///     }
    ///     // Preserve the original cause and leave outer reply/checkpoint ownership explicit.
    ///     outcome.into_result()
    /// }
    /// ```
    pub fn operation_id(&self) -> Option<&OperationId> {
        self.operation_id.as_ref()
    }
    fn with_operation_id(mut self, id: Option<OperationId>) -> Self {
        self.operation_id = id;
        self
    }
    /// Propagate an incomplete invocation without the normal typed-rejection
    /// auto-reply. The outer failure policy owns reconciliation and checkpointing.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{InvocationOutcome, InvocationStatus, Result};
    /// fn propagate(outcome: InvocationOutcome) -> Result<()> {
    ///     if outcome.status != InvocationStatus::Completed {
    ///         eprintln!("Incomplete {}, operation {:?}, effects possible: {}", outcome.command, outcome.operation_id(), outcome.effects_possible);
    ///     }
    ///     // Preserve the original cause and leave outer reply/checkpoint ownership explicit.
    ///     outcome.into_result()
    /// }
    /// ```
    pub fn into_result(self) -> Result<()> {
        if self.status == InvocationStatus::Completed {
            return Ok(());
        }
        let error = Error::Invocation {
            command: self.command,
            status: self.status,
            effects_possible: self.effects_possible,
            source: self.error.map(Box::new),
        };
        Err(match self.operation_id {
            Some(operation_id) => Error::IdentifiedInvocation {
                operation_id,
                source: Box::new(error),
            },
            None => error,
        })
    }
    fn failed(command: &str, error: Error, effects_possible: bool) -> Self {
        Self {
            status: if error.is_command_rejection() {
                InvocationStatus::Denied
            } else {
                InvocationStatus::Failed
            },
            command: command.into(),
            effects_possible,
            error: Some(error),
            operation_id: None,
        }
    }
}

/// Advisory assessment, never a reservation or an authorization grant.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
/// # Example
///
/// ```no_run
/// use rappin::{BotHandle, MessageContext, OperationId, Result};
/// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
///     // Must run in the original authenticated message task, not a synthetic context.
///     let plan = handle.prepare_invocation_with_operation_id(
///         ctx, "status", "", OperationId::new("request:42:step:1")?
///     ).await?;
///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
///     // Preparation grants no future authority. Execution rechecks the installed binding.
///     handle.invoke_prepared(plan).await.into_result()
/// }
/// ```
pub struct PreparationReport {
    /// Checks established by read-only preparation; these are rechecked during execution.
    pub assessed: Vec<String>,
    /// Checks or arbitrary application logic that cannot be established during preparation.
    pub deferred: Vec<String>,
}
/// Opaque, single-use plan bound to the original event and installed binding.
/// Execute within the same message task. Unload/replacement requires preparation
/// again, even if a replacement uses the same name and argument metadata.
/// # Example
///
/// ```no_run
/// use rappin::{BotHandle, MessageContext, OperationId, Result};
/// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
///     // Must run in the original authenticated message task, not a synthetic context.
///     let plan = handle.prepare_invocation_with_operation_id(
///         ctx, "status", "", OperationId::new("request:42:step:1")?
///     ).await?;
///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
///     // Preparation grants no future authority. Execution rechecks the installed binding.
///     handle.invoke_prepared(plan).await.into_result()
/// }
/// ```
pub struct PreparedInvocation {
    context: MessageContext,
    requested: String,
    arguments: String,
    snapshot: InvocationSnapshot,
    report: PreparationReport,
    operation_id: Option<OperationId>,
}
impl std::fmt::Debug for PreparedInvocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedInvocation")
            .field("command", &self.command())
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}
impl PreparedInvocation {
    /// Consumer key retained from preparation, separate from authenticated message identity.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     // Must run in the original authenticated message task, not a synthetic context.
    ///     let plan = handle.prepare_invocation_with_operation_id(
    ///         ctx, "status", "", OperationId::new("request:42:step:1")?
    ///     ).await?;
    ///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
    ///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
    ///     // Preparation grants no future authority. Execution rechecks the installed binding.
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub fn operation_id(&self) -> Option<&OperationId> {
        self.operation_id.as_ref()
    }
    /// Read-only assessment, including checks deferred until execution.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     // Must run in the original authenticated message task, not a synthetic context.
    ///     let plan = handle.prepare_invocation_with_operation_id(
    ///         ctx, "status", "", OperationId::new("request:42:step:1")?
    ///     ).await?;
    ///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
    ///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
    ///     // Preparation grants no future authority. Execution rechecks the installed binding.
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub fn report(&self) -> &PreparationReport {
        &self.report
    }
    /// Canonical installed command path captured by preparation.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     // Must run in the original authenticated message task, not a synthetic context.
    ///     let plan = handle.prepare_invocation_with_operation_id(
    ///         ctx, "status", "", OperationId::new("request:42:step:1")?
    ///     ).await?;
    ///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
    ///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
    ///     // Preparation grants no future authority. Execution rechecks the installed binding.
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub fn command(&self) -> &str {
        self.context.command.as_deref().expect("prepared command")
    }
    /// Original explicit argument string retained for execution.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     // Must run in the original authenticated message task, not a synthetic context.
    ///     let plan = handle.prepare_invocation_with_operation_id(
    ///         ctx, "status", "", OperationId::new("request:42:step:1")?
    ///     ).await?;
    ///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
    ///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
    ///     // Preparation grants no future authority. Execution rechecks the installed binding.
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub fn arguments(&self) -> &str {
        &self.context.args
    }
}

pub(super) struct InvocationSnapshot {
    pub(super) def: CommandDef,
    cog: Option<Arc<dyn Cog>>,
}
impl InvocationSnapshot {
    fn same_binding(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.def.handler, &other.def.handler)
            && match (&self.cog, &other.cog) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}
impl BotHandle {
    fn invocation_origin(&self, ctx: &MessageContext) -> Result<Origin> {
        let origin = ORIGIN.try_with(Clone::clone).map_err(|_| {
            Error::validation(
                "invocation.context",
                "requires an active authenticated message task",
            )
        })?;
        if origin.gate_running {
            return Err(Error::validation(
                "invocation.context",
                "message gates cannot recursively invoke commands",
            ));
        }
        if !Arc::ptr_eq(&origin.bot.inner, &self.inner)
            || !Arc::ptr_eq(&ctx.client, &self.inner.client)
            || !ctx
                .bot
                .as_ref()
                .is_some_and(|bot| Arc::ptr_eq(&bot.inner, &self.inner))
            || &ctx.event != origin.event.as_ref()
        {
            return Err(Error::validation(
                "invocation.context",
                "message identity or bot does not match authenticated dispatch",
            ));
        }
        Ok(origin)
    }
    pub(super) async fn resolve_invocation(
        &self,
        name: &str,
        arguments: &str,
    ) -> Option<(String, String)> {
        let commands = self.inner.commands.read().await;
        let mut path = name.to_lowercase();
        let mut best = commands.contains_key(&path).then_some((path.clone(), 0));
        let tokens = crate::commands::tokenize(arguments).unwrap_or_default();
        for token in tokens
            .iter()
            .take(8usize.saturating_sub(path.split(' ').count()))
        {
            path.push(' ');
            path.push_str(&token.value.to_lowercase());
            if commands.contains_key(&path) {
                best = Some((path.clone(), token.end));
            }
        }
        best.map(|(path, end)| (path, arguments[end..].trim_start().into()))
    }
    pub(super) async fn invocation_snapshot(&self, name: &str) -> Option<InvocationSnapshot> {
        // Installation acquires commands then cogs before publishing either.
        let commands = self.inner.commands.read().await;
        let def = commands.get(name)?.clone();
        let cog = match &def.cog_name {
            Some(name) => self.inner.cogs.read().await.get(name).cloned(),
            None => None,
        };
        Some(InvocationSnapshot { def, cog })
    }
    /// Assess an installed command without executing handlers, arbitrary parsing,
    /// arbitrary checks, cog checks, the message gate, or consuming cooldowns.
    /// Built-in declarative checks and dynamic schemas can be assessed. Custom
    /// CommandCheck::prepare implementations opt into the read-only contract.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     let plan = handle.prepare_invocation(ctx, "status", "").await?;
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub async fn prepare_invocation(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
    ) -> Result<PreparedInvocation> {
        self.prepare_invocation_inner(ctx, name, arguments, ctx.operation_id())
            .await
    }
    /// Prepare with an explicit consumer operation key before any handler runs.
    /// This key supplies no authentication, authorization or deduplication.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     // Must run in the original authenticated message task, not a synthetic context.
    ///     let plan = handle.prepare_invocation_with_operation_id(
    ///         ctx, "status", "", OperationId::new("request:42:step:1")?
    ///     ).await?;
    ///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
    ///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
    ///     // Preparation grants no future authority. Execution rechecks the installed binding.
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub async fn prepare_invocation_with_operation_id(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
        operation_id: OperationId,
    ) -> Result<PreparedInvocation> {
        self.prepare_invocation_inner(ctx, name, arguments, Some(operation_id))
            .await
    }
    async fn prepare_invocation_inner(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
        operation_id: Option<OperationId>,
    ) -> Result<PreparedInvocation> {
        let mut origin = self.invocation_origin(ctx)?;
        origin.operation_id = operation_id.clone();
        ORIGIN
            .scope(origin, async {
                let (path, args) = self
                    .resolve_invocation(name, arguments)
                    .await
                    .ok_or_else(|| Error::validation("invocation.command", "command not found"))?;
                let snapshot = self.invocation_snapshot(&path).await.ok_or_else(|| {
                    Error::validation("invocation.command", "command was unloaded")
                })?;
                if operation_id.is_some() && !snapshot.def.operation_identity_supported {
                    return Err(Error::validation(
                        "invocation.operation_id",
                        "worker requires protocol v8 operation identity support",
                    ));
                }
                let mut context = ctx.clone();
                context.command = Some(path);
                context.args = args;
                context.argv = crate::commands::tokenize(&context.args)?
                    .into_iter()
                    .map(|t| t.value)
                    .collect();
                let mut report = PreparationReport::default();
                if self.inner.message_gate.is_some() {
                    report.deferred.push("message gate".into());
                }
                if snapshot.cog.is_some() {
                    report.deferred.push("cog check".into());
                }
                if let Some(recipe) = &snapshot.def.preparation {
                    if recipe.worker_checks {
                        report.deferred.push("worker command checks".into());
                    }
                    if let Some(schema) = &recipe.schema {
                        schema.parse(&context.args)?;
                        report.assessed.push("dynamic argument schema".into());
                    } else {
                        report.deferred.push("typed argument parser".into());
                    }
                    for (index, check) in recipe.checks.iter().enumerate() {
                        if let Some(future) = check.prepare(&context) {
                            failure::catch(future).await?;
                            report.assessed.push(format!("registered check {index}"));
                        } else {
                            report.deferred.push(format!("registered check {index}"));
                        }
                    }
                } else {
                    report
                        .deferred
                        .push("opaque native/worker parsing, checks and cooldowns".into());
                }
                report
                    .deferred
                    .push("handler-owned validation and effects".into());
                Ok(PreparedInvocation {
                    context,
                    requested: name.into(),
                    arguments: arguments.into(),
                    snapshot,
                    report,
                    operation_id,
                })
            })
            .await
    }
    /// Invoke with fresh normal execution checks. Does not redispatch an event,
    /// run listeners, advance a cursor, retry, or send framework error replies.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     let outcome = handle.invoke_command(ctx, "status", "").await;
    ///     if let Some(error) = &outcome.error { eprintln!("{:?}: {}", outcome.status, error); }
    ///     // No framework error reply/callback was sent by this nested invocation.
    ///     outcome.into_result()
    /// }
    /// ```
    pub async fn invoke_command(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
    ) -> InvocationOutcome {
        self.invoke_command_inner(ctx, name, arguments, ctx.operation_id())
            .await
    }
    /// Invoke with a consumer-owned operation key; all normal checks still run.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     let outcome = handle.invoke_command_with_operation_id(ctx, "status", "", OperationId::new("request:42:step:1")?).await;
    ///     if let Some(error) = &outcome.error { eprintln!("{:?}: {}", outcome.status, error); }
    ///     // No framework error reply/callback was sent by this nested invocation.
    ///     outcome.into_result()
    /// }
    /// ```
    pub async fn invoke_command_with_operation_id(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
        operation_id: OperationId,
    ) -> InvocationOutcome {
        self.invoke_command_inner(ctx, name, arguments, Some(operation_id))
            .await
    }
    async fn invoke_command_inner(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
        operation_id: Option<OperationId>,
    ) -> InvocationOutcome {
        self.invoke_command_unlabelled(ctx, name, arguments, operation_id.clone())
            .await
            .with_operation_id(operation_id)
    }
    async fn invoke_command_unlabelled(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
        operation_id: Option<OperationId>,
    ) -> InvocationOutcome {
        if let Err(error) = self.invocation_origin(ctx) {
            return InvocationOutcome::failed(name, error, false);
        }
        let Some((path, args)) = self.resolve_invocation(name, arguments).await else {
            return InvocationOutcome {
                status: InvocationStatus::NotFound,
                command: name.into(),
                effects_possible: false,
                error: None,
                operation_id: None,
            };
        };
        let Some(snapshot) = self.invocation_snapshot(&path).await else {
            return InvocationOutcome {
                status: InvocationStatus::NotFound,
                command: path,
                effects_possible: false,
                error: None,
                operation_id: None,
            };
        };
        let context = MessageContext::new(
            ctx.client.clone(),
            ctx.event.clone(),
            Some(path),
            args,
            ctx.bot.clone(),
        );
        self.execute_checked_invocation(context, snapshot, operation_id)
            .await
    }
    /// Execute a single-use plan; reject stale bindings and recheck authorization.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, MessageContext, OperationId, Result};
    /// async fn invoke(handle: &BotHandle, ctx: &MessageContext) -> Result<()> {
    ///     // Must run in the original authenticated message task, not a synthetic context.
    ///     let plan = handle.prepare_invocation_with_operation_id(
    ///         ctx, "status", "", OperationId::new("request:42:step:1")?
    ///     ).await?;
    ///     println!("{} {}: deferred {:?}", plan.command(), plan.arguments(), plan.report().deferred);
    ///     assert_eq!(plan.operation_id().map(OperationId::as_str), Some("request:42:step:1"));
    ///     // Preparation grants no future authority. Execution rechecks the installed binding.
    ///     handle.invoke_prepared(plan).await.into_result()
    /// }
    /// ```
    pub async fn invoke_prepared(&self, plan: PreparedInvocation) -> InvocationOutcome {
        let operation_id = plan.operation_id.clone();
        self.invoke_prepared_inner(plan)
            .await
            .with_operation_id(operation_id)
    }
    async fn invoke_prepared_inner(&self, plan: PreparedInvocation) -> InvocationOutcome {
        let name = plan.command().to_owned();
        if let Err(error) = self.invocation_origin(&plan.context) {
            return InvocationOutcome::failed(&name, error, false);
        }
        let current = self
            .resolve_invocation(&plan.requested, &plan.arguments)
            .await;
        let snapshot = self.invocation_snapshot(&name).await;
        match (current, snapshot) {
            (Some((path, args)), Some(snapshot))
                if path == name
                    && args == plan.context.args
                    && snapshot.same_binding(&plan.snapshot) =>
            {
                self.execute_checked_invocation(plan.context, snapshot, plan.operation_id)
                    .await
            }
            _ => InvocationOutcome::failed(
                &name,
                Error::validation(
                    "invocation.binding",
                    "binding changed or was unloaded; prepare again",
                ),
                false,
            ),
        }
    }
    async fn execute_checked_invocation(
        &self,
        ctx: MessageContext,
        snapshot: InvocationSnapshot,
        operation_id: Option<OperationId>,
    ) -> InvocationOutcome {
        if operation_id.is_some() && !snapshot.def.operation_identity_supported {
            return InvocationOutcome::failed(
                ctx.command.as_deref().unwrap_or_default(),
                Error::validation(
                    "invocation.operation_id",
                    "worker requires protocol v8 operation identity support",
                ),
                false,
            );
        }
        let mut origin = ORIGIN.with(Clone::clone);
        origin.operation_id = operation_id;
        ORIGIN
            .scope(origin, self.execute_checked_invocation_inner(ctx, snapshot))
            .await
    }
    async fn execute_checked_invocation_inner(
        &self,
        ctx: MessageContext,
        snapshot: InvocationSnapshot,
    ) -> InvocationOutcome {
        let name = ctx.command.clone().unwrap_or_default();
        if ORIGIN.with(|o| o.depth) >= MAX_INVOCATION_DEPTH {
            return InvocationOutcome::failed(
                &name,
                Error::validation("invocation.depth", "nested command depth exceeded"),
                false,
            );
        }
        if let Some(gate) = &self.inner.message_gate {
            // The gate sees the original message, as in normal intake; no grants
            // from preparation or caller-supplied argument metadata are accepted.
            let gate_ctx = MessageContext::new(
                ctx.client.clone(),
                ctx.event.clone(),
                None,
                String::new(),
                ctx.bot.clone(),
            );
            let mut origin = ORIGIN.with(Clone::clone);
            origin.gate_running = true;
            match ORIGIN.scope(origin, failure::catch(gate(gate_ctx))).await {
                Ok(MessageDisposition::Continue) => (),
                Ok(MessageDisposition::Handled) => {
                    return InvocationOutcome::failed(
                        &name,
                        crate::commands::CommandError::Denied(
                            "message gate handled this request".into(),
                        )
                        .into(),
                        true,
                    )
                }
                Ok(MessageDisposition::Stop) => {
                    return InvocationOutcome::failed(
                        &name,
                        Error::HandlerInterrupted {
                            reason: "message gate stopped invocation".into(),
                            source: None,
                        },
                        true,
                    )
                }
                Err(error) => return InvocationOutcome::failed(&name, error, true),
            }
        }
        // A gate can await arbitrary application work. Reject a replacement
        // published while it ran instead of entering the retired binding.
        if !self
            .invocation_snapshot(&name)
            .await
            .is_some_and(|current| current.same_binding(&snapshot))
        {
            return InvocationOutcome::failed(
                &name,
                Error::validation(
                    "invocation.binding",
                    "binding changed before execution; prepare again",
                ),
                self.inner.message_gate.is_some(),
            );
        }
        self.dispatch_invocation(ctx, snapshot).await
    }
    pub(super) async fn dispatch_invocation(
        &self,
        ctx: MessageContext,
        snapshot: InvocationSnapshot,
    ) -> InvocationOutcome {
        let name = ctx.command.clone().unwrap_or_default();
        let mut origin = ORIGIN.with(Clone::clone);
        if origin.depth >= MAX_INVOCATION_DEPTH {
            return InvocationOutcome::failed(
                &name,
                Error::validation("invocation.depth", "nested command depth exceeded"),
                false,
            );
        }
        origin.depth += 1;
        ORIGIN
            .scope(origin, async {
                let result = failure::catch(async {
                    if let Some(cog) = snapshot.cog {
                        if !cog.check(&ctx).await {
                            return Ok(false);
                        }
                    }
                    (snapshot.def.handler)(ctx).await?;
                    Ok(true)
                })
                .await;
                match result {
                    Ok(true) => InvocationOutcome {
                        status: InvocationStatus::Completed,
                        command: name,
                        effects_possible: true,
                        error: None,
                        operation_id: None,
                    },
                    Ok(false) => InvocationOutcome {
                        status: InvocationStatus::Denied,
                        command: name,
                        effects_possible: true,
                        error: None,
                        operation_id: None,
                    },
                    Err(error) => InvocationOutcome::failed(&name, error, true),
                }
            })
            .await
    }
}
