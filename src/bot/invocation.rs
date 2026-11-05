//! Checked invocation within the lifetime of an authenticated message dispatch.
use super::{failure, BotHandle, Cog, CommandDef, MessageContext, MessageDisposition};
use crate::{MessageEvent, Result, YappinError};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc};

tokio::task_local! { static ORIGIN: Origin; }
#[derive(Clone)]
struct Origin {
    bot: BotHandle,
    event: Arc<MessageEvent>,
    depth: usize,
    gate_running: bool,
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
            },
            future,
        )
        .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InvocationStatus {
    NotFound,
    Denied,
    Completed,
    Failed,
}

/// Result ownership belongs to the caller: invocation sends no automatic error
/// replies and calls no failure callbacks. Check/handler code can still reply.
#[derive(Debug)]
#[must_use = "propagate incomplete invocation or explicitly reconcile it"]
#[non_exhaustive]
pub struct InvocationOutcome {
    pub status: InvocationStatus,
    pub command: String,
    /// Conservative: arbitrary cog/check/handler code or the message gate ran.
    pub effects_possible: bool,
    pub error: Option<YappinError>,
}
impl InvocationOutcome {
    /// Propagate an incomplete invocation without the normal typed-rejection
    /// auto-reply. The outer failure policy owns reconciliation and checkpointing.
    pub fn into_result(self) -> Result<()> {
        if self.status == InvocationStatus::Completed {
            return Ok(());
        }
        Err(YappinError::Invocation {
            command: self.command,
            status: self.status,
            effects_possible: self.effects_possible,
            source: self.error.map(Box::new),
        })
    }
    fn failed(command: &str, error: YappinError, effects_possible: bool) -> Self {
        Self {
            status: if error.is_command_rejection() {
                InvocationStatus::Denied
            } else {
                InvocationStatus::Failed
            },
            command: command.into(),
            effects_possible,
            error: Some(error),
        }
    }
}

/// Advisory assessment, never a reservation or an authorization grant.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PreparationReport {
    pub assessed: Vec<String>,
    pub deferred: Vec<String>,
}
/// Opaque, single-use plan bound to the original event and installed binding.
/// Execute within the same message task. Unload/replacement requires preparation
/// again, even if a replacement uses the same name and argument metadata.
pub struct PreparedInvocation {
    context: MessageContext,
    requested: String,
    arguments: String,
    snapshot: InvocationSnapshot,
    report: PreparationReport,
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
    pub fn report(&self) -> &PreparationReport {
        &self.report
    }
    pub fn command(&self) -> &str {
        self.context.command.as_deref().expect("prepared command")
    }
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
            YappinError::validation(
                "invocation.context",
                "requires an active authenticated message task",
            )
        })?;
        if origin.gate_running {
            return Err(YappinError::validation(
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
            return Err(YappinError::validation(
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
    pub async fn prepare_invocation(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
    ) -> Result<PreparedInvocation> {
        self.invocation_origin(ctx)?;
        let (path, args) = self
            .resolve_invocation(name, arguments)
            .await
            .ok_or_else(|| YappinError::validation("invocation.command", "command not found"))?;
        let snapshot = self
            .invocation_snapshot(&path)
            .await
            .ok_or_else(|| YappinError::validation("invocation.command", "command was unloaded"))?;
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
        })
    }
    /// Invoke with fresh normal execution checks. Does not redispatch an event,
    /// run listeners, advance a cursor, retry, or send framework error replies.
    pub async fn invoke_command(
        &self,
        ctx: &MessageContext,
        name: &str,
        arguments: &str,
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
            };
        };
        let Some(snapshot) = self.invocation_snapshot(&path).await else {
            return InvocationOutcome {
                status: InvocationStatus::NotFound,
                command: path,
                effects_possible: false,
                error: None,
            };
        };
        let context = MessageContext::new(
            ctx.client.clone(),
            ctx.event.clone(),
            Some(path),
            args,
            ctx.bot.clone(),
        );
        self.execute_checked_invocation(context, snapshot).await
    }
    /// Execute a single-use plan; reject stale bindings and recheck authorization.
    pub async fn invoke_prepared(&self, plan: PreparedInvocation) -> InvocationOutcome {
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
                    && Arc::ptr_eq(&snapshot.def.handler, &plan.snapshot.def.handler)
                    && match (&snapshot.cog, &plan.snapshot.cog) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        _ => false,
                    } =>
            {
                self.execute_checked_invocation(plan.context, snapshot)
                    .await
            }
            _ => InvocationOutcome::failed(
                &name,
                YappinError::validation(
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
    ) -> InvocationOutcome {
        let name = ctx.command.clone().unwrap_or_default();
        if ORIGIN.with(|o| o.depth) >= MAX_INVOCATION_DEPTH {
            return InvocationOutcome::failed(
                &name,
                YappinError::validation("invocation.depth", "nested command depth exceeded"),
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
                        YappinError::HandlerInterrupted {
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
                YappinError::validation(
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
                YappinError::validation("invocation.depth", "nested command depth exceeded"),
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
                    },
                    Ok(false) => InvocationOutcome {
                        status: InvocationStatus::Denied,
                        command: name,
                        effects_possible: true,
                        error: None,
                    },
                    Err(error) => InvocationOutcome::failed(&name, error, true),
                }
            })
            .await
    }
}
