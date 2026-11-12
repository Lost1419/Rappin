//! Bot configuration, scoped command routing, cog registration and ordered event processing.
pub mod cog;
pub mod context;
mod failure;
pub mod handler;
mod ingress;
mod invocation;
mod shared_pipeline;
pub use ingress::{
    canonical_event_fingerprint, prepare_event_retirement, prepare_timestamp_reconciliation,
    Admission, AdmissionDurability, AdmissionStatus, ConflictDisposition, EventIngress,
    EventReceipt, EventRetirement, EventRetirementAssessment, EventRetirementEvidence,
    EventRetirementOptions, EventRetirementRequest, EventRetirementState, IngressAdmissionPolicy,
    IngressConfig, IngressConflict, IngressConflictPolicy, IngressRecord, IngressRecovery,
    IngressSource, IngressStatus, IngressStore, PayloadDifference, PayloadType,
    PreparedEventRetirement, PreparedTimestampReconciliation, ReceiptStage, ReceiptStatus,
    TimestampReconciliation, TimestampReconciliationOptions, TimestampReconciliationRequest,
    TimestampReconciliationState,
};
mod message;
pub use invocation::{
    InvocationOutcome, InvocationStatus, OperationId, PreparationReport, PreparedInvocation,
    MAX_INVOCATION_DEPTH,
};
mod drain;
mod pipeline;
pub use failure::{
    EventFailure, EventProcessingFailure, FailureClass, FailureDisposition, HandlerFailure,
    HandlerKind,
};
pub use message::{MessageDisposition, MessageFailure, MessageStage};

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::events::EventCursor;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub use cog::{
    ButtonHandlerFn, Cog, CogBuilder, CogButton, CogCommand, CommandHandlerFn, DynamicCog,
};
pub use context::{InteractionContext, MessageContext};
pub use handler::{EventHandler, NoopEventHandler};

use crate::client::Client;
use crate::error::{Result, YappinError};
use crate::models::{BotProfile, Command, EventData, InteractionEvent, MessageEvent};
use crate::utils::DEFAULT_API_BASE;

#[derive(Clone)]
struct CommandDef {
    description: String,
    cog_name: Option<String>,
    handler: CommandHandlerFn,
    info: Option<crate::commands::CommandInfo>,
    preparation: Option<crate::commands::CommandPreparation>,
    operation_identity_supported: bool,
}

#[derive(Clone)]
struct CommandRegistry {
    routes: HashMap<String, CommandDef>,
    roots: std::collections::HashSet<String>,
}

/// An advisory plan, not a reservation. Installation revalidates under the
/// registry update lock because other tasks can change routes after preflight.
#[derive(Debug, Clone)]
/// # Example
///
/// ```rust
/// use rappin::{BotHandle, Cog, Result};
/// async fn assess(runner: &BotHandle, candidate: &dyn Cog) -> Result<()> {
///     let plan = runner.preflight_cog(candidate).await?;
///     println!("Advertised: {:?}; routes: {:?}", plan.advertised, plan.routes);
///     // Advisory only: installation rechecks conflicts and publication can still fail.
///     Ok(())
/// }
/// ```
pub struct RegistrationPlan {
    /// Root command definitions that would be published to Yapin.
    pub advertised: Vec<Command>,
    /// Canonical local command and alias paths checked for conflicts.
    pub routes: Vec<String>,
    /// Cog whose current routes may be replaced, if this is a replacement assessment.
    pub replacing_cog: Option<String>,
}
impl CommandRegistry {
    fn new(routes: HashMap<String, CommandDef>) -> Self {
        let mut registry = Self {
            routes,
            roots: Default::default(),
        };
        registry.reindex();
        registry
    }
    fn reindex(&mut self) {
        self.roots = self
            .routes
            .keys()
            .filter_map(|path| path.split(' ').next())
            .map(str::to_owned)
            .collect();
    }
}
impl std::ops::Deref for CommandRegistry {
    type Target = HashMap<String, CommandDef>;
    fn deref(&self) -> &Self::Target {
        &self.routes
    }
}

fn prepare_prefixes(prefixes: &mut Vec<String>) {
    prefixes.retain(|prefix| !prefix.is_empty());
    prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
    prefixes.dedup();
}

fn advertised_commands(commands: &HashMap<String, CommandDef>) -> Result<Vec<Command>> {
    let mut roots = std::collections::BTreeMap::new();
    let mut ordered: Vec<_> = commands.iter().collect();
    ordered.sort_by_key(|(name, _)| *name);
    for (name, def) in ordered {
        if def.info.as_ref().is_some_and(|info| info.name != *name) {
            continue;
        }
        let parts: Vec<_> = name.split(' ').collect();
        if parts.len() > 8 {
            return Err(crate::commands::CommandError::Definition(
                "subcommand depth exceeds eight".into(),
            )
            .into());
        }
        for part in &parts {
            crate::validation::validate_commands(&[Command::new(*part, &def.description)])?;
        }
        roots
            .entry(parts[0].to_string())
            .or_insert_with(|| def.description.clone());
    }
    let result: Vec<_> = roots
        .into_iter()
        .map(|(name, description)| Command::new(name, description))
        .collect();
    crate::validation::validate_commands(&result)?;
    Ok(result)
}
fn insert_registered(
    commands: &mut HashMap<String, CommandDef>,
    command: crate::commands::RegisteredCommand,
    cog_name: Option<String>,
) -> Result<()> {
    command.info.validate()?;
    let mut names = vec![command.info.name.clone()];
    names.extend(command.info.aliases.clone());
    let mut unique = std::collections::HashSet::new();
    for name in &names {
        if !unique.insert(name) || commands.contains_key(name) {
            return Err(crate::commands::CommandError::Definition(
                "command or alias already registered".into(),
            )
            .into());
        }
        if name.split(' ').count() > 8 {
            return Err(crate::commands::CommandError::Definition(
                "subcommand depth exceeds eight".into(),
            )
            .into());
        }
        for part in name.split(' ') {
            crate::validation::validate_commands(&[Command::new(part, &command.info.description)])?;
        }
    }
    for name in names {
        commands.insert(
            name,
            CommandDef {
                description: command.info.description.clone(),
                cog_name: cog_name.clone(),
                handler: command.handler.clone(),
                info: Some(command.info.clone()),
                preparation: command.preparation.clone(),
                operation_identity_supported: true,
            },
        );
    }
    Ok(())
}

type PrefixResolver = Arc<
    dyn Fn(MessageEvent) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send>>
        + Send
        + Sync,
>;

struct ButtonDef {
    prefix: String,
    cog_name: Option<String>,
    handler: ButtonHandlerFn,
}

type CogFactoryFn = Arc<dyn Fn() -> Box<dyn Cog> + Send + Sync>;

/// Legacy command failure observer; completion policy remains separate.
pub type CommandErrorHandlerFn = Arc<
    dyn Fn(MessageContext, YappinError) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Legacy button failure observer; completion policy remains separate.
pub type ButtonErrorHandlerFn = Arc<
    dyn Fn(InteractionContext, YappinError) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Fallible command callback returning an explicit completion disposition.
pub type CommandFailureHandlerFn = Arc<
    dyn Fn(
            MessageContext,
            HandlerFailure,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<FailureDisposition>> + Send>>
        + Send
        + Sync,
>;
/// Fallible button callback returning an explicit completion disposition.
pub type ButtonFailureHandlerFn = Arc<
    dyn Fn(
            InteractionContext,
            HandlerFailure,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<FailureDisposition>> + Send>>
        + Send
        + Sync,
>;

/// Fallible shared gate before server-message command dispatch.
pub type MessageGateFn = Arc<
    dyn Fn(
            MessageContext,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<MessageDisposition>> + Send>>
        + Send
        + Sync,
>;
/// Fallible gate/listener failure callback with explicit completion ownership.
pub type MessageFailureHandlerFn = Arc<
    dyn Fn(
            MessageContext,
            MessageFailure,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<FailureDisposition>> + Send>>
        + Send
        + Sync,
>;

type UiHandler = Arc<
    dyn Fn(
            crate::UiInteractionContext,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

pub(crate) struct BotState {
    client: Arc<Client>,
    prefixes: Vec<String>,
    prefix_resolver: Option<PrefixResolver>,
    mention_prefix: bool,
    definition_error: Option<String>,
    commands: RwLock<CommandRegistry>,
    buttons: RwLock<Vec<ButtonDef>>,
    cogs: RwLock<HashMap<String, Arc<dyn Cog>>>,
    cog_factories: RwLock<HashMap<String, CogFactoryFn>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    command_error_handler: Option<CommandErrorHandlerFn>,
    button_error_handler: Option<ButtonErrorHandlerFn>,
    command_failure_handler: Option<CommandFailureHandlerFn>,
    button_failure_handler: Option<ButtonFailureHandlerFn>,
    message_gate: Option<MessageGateFn>,
    message_failure_handler: Option<MessageFailureHandlerFn>,
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
    cursor: Option<EventCursor>,
    max_age: Option<u32>,
    availability_policy: Option<crate::AvailabilityPolicy>,
    ui_events: bool,
    interaction_commands: bool,
    ui_handlers: HashMap<String, UiHandler>,
    handler_concurrency: usize,
    pending_batches: usize,
    drain_timeout: Option<Duration>,
    auto_ack_after: Option<Duration>,
    ingress: Option<EventIngress>,
    telemetry: crate::diagnostics::Telemetry,
    pub(crate) availability_telemetry: crate::availability::Telemetry,
    #[cfg(feature = "gateway")]
    gateway_url: Option<String>,
    registry_update: tokio::sync::Mutex<()>,
    pub(crate) running: AtomicBool,
    draining: AtomicBool,
    pub(crate) stopped: tokio::sync::Notify,
}

/// A cloneable handle to control the bot and dynamically manage cogs while running.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::Bot;
/// let bot = Bot::builder("unused-documentation-token")
///     .command("ping", "Check connectivity", |ctx| async move {
///         ctx.reply("pong").await?; Ok(())
///     }).build();
/// // Call bot.run().await only after configuring a real token in the environment.
/// ```
pub struct BotHandle {
    pub(crate) inner: Arc<BotState>,
}

#[cfg(feature = "hot-reload")]
pub(crate) struct WeakBotHandle {
    inner: std::sync::Weak<BotState>,
}
#[cfg(feature = "hot-reload")]
impl WeakBotHandle {
    pub(crate) fn upgrade(&self) -> Option<BotHandle> {
        self.inner.upgrade().map(|inner| BotHandle { inner })
    }
}

impl BotHandle {
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// async fn inspect(runner: &BotHandle) {
    ///     let metadata = runner.command_info().await;
    ///     println!("{metadata:?}");
    /// }
    /// ```
    pub async fn command_info(&self) -> Vec<crate::commands::CommandInfo> {
        let commands = self.inner.commands.read().await;
        let mut result: Vec<_> = commands
            .iter()
            .filter(|(name, def)| def.info.as_ref().is_none_or(|info| info.name == **name))
            .map(|(name, def)| {
                def.info
                    .clone()
                    .unwrap_or_else(|| crate::commands::CommandInfo {
                        name: name.clone(),
                        description: def.description.clone(),
                        aliases: vec![],
                        arguments: vec![],
                    })
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// async fn inspect(runner: &BotHandle) {
    ///     let metadata = runner.help("status").await;
    ///     println!("{metadata:?}");
    /// }
    /// ```
    pub async fn help(&self, path: &str) -> String {
        let info = self.command_info().await;
        if path.is_empty() {
            return info
                .iter()
                .map(|info| format!("{} — {}", info.usage(), info.description))
                .collect::<Vec<_>>()
                .join("\n");
        }
        info.iter()
            .filter(|info| {
                info.name == path
                    || info.name.starts_with(&format!("{path} "))
                    || info.aliases.iter().any(|alias| alias == path)
            })
            .map(|info| info.help())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    /// Read the latest availability snapshot; watch updates may coalesce.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     let status = runner.availability_status();
    ///     println!("Availability: {:?}", status);
    /// }
    /// ```
    pub fn availability_status(&self) -> crate::AvailabilityStatus {
        self.inner.availability_telemetry.status()
    }
    /// Subscribe to the latest availability state; use cumulative counters for totals.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     let updates = runner.subscribe_availability();
    /// }
    /// ```
    pub fn subscribe_availability(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::AvailabilityStatus> {
        self.inner.availability_telemetry.subscribe()
    }
    /// Read current active event transport and fallback state.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     let status = runner.transport_status();
    ///     println!("Transport: {:?}", status);
    /// }
    /// ```
    pub fn transport_status(&self) -> crate::diagnostics::TransportStatus {
        self.inner.telemetry.status()
    }
    /// Subscribe to latest transport state; updates may coalesce.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// async fn observe(runner: &BotHandle) {
    ///     let mut updates = runner.subscribe_transport();
    ///     while updates.changed().await.is_ok() { println!("{:?}", *updates.borrow_and_update()); }
    /// }
    /// ```
    pub fn subscribe_transport(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::diagnostics::TransportStatus> {
        self.inner.telemetry.subscribe()
    }
    /// Access the event collector hub owned by this bot.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     let hub = runner.events();
    /// }
    /// ```
    pub fn events(&self) -> &crate::collectors::EventHub {
        self.client().events()
    }
    #[cfg(feature = "hot-reload")]
    pub(crate) fn downgrade(&self) -> WeakBotHandle {
        WeakBotHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }
    /// Returns a reference to the bot's REST client.
    /// True after intake closes and while the running pipeline drains accepted work.
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn status(runner: &BotHandle) -> bool { runner.is_draining() }
    /// ```
    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::SeqCst)
    }
    /// Borrow the authenticated client; application code remains responsible for operation authorization.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     let client = runner.client();
    /// }
    /// ```
    pub fn client(&self) -> &Client {
        &self.inner.client
    }

    /// Returns a reference to the client's internal in-memory cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     let cache = runner.cache();
    /// }
    /// ```
    pub fn cache(&self) -> &crate::cache::Cache {
        self.inner.client.cache()
    }

    /// Checks if a cog with the given name is currently loaded.
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// async fn inspect(runner: &BotHandle) {
    ///     let metadata = runner.has_cog("status").await;
    ///     println!("{metadata:?}");
    /// }
    /// ```
    pub async fn has_cog(&self, name: &str) -> bool {
        self.inner.cogs.read().await.contains_key(name)
    }

    /// Lists the names of all currently loaded cogs.
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// async fn inspect(runner: &BotHandle) {
    ///     let metadata = runner.list_cogs().await;
    ///     println!("{metadata:?}");
    /// }
    /// ```
    pub async fn list_cogs(&self) -> Vec<String> {
        self.inner.cogs.read().await.keys().cloned().collect()
    }

    /// Registers a factory function to instantiate a cog by name dynamically.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{BotHandle, CogBuilder, Result};
    /// async fn install(runner: &BotHandle) {
    ///     runner.register_cog_factory("status", || Box::new(CogBuilder::new("status").build())).await;
    /// }
    /// ```
    pub async fn register_cog_factory<F>(&self, name: impl Into<String>, factory: F)
    where
        F: Fn() -> Box<dyn Cog> + Send + Sync + 'static,
    {
        let name_str = name.into();
        self.inner
            .cog_factories
            .write()
            .await
            .insert(name_str, Arc::new(factory));
    }

    /// Install a cog without removing its current version until preparation succeeds.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, Result};
    /// async fn update(runner: &BotHandle, cog: impl rappin::Cog) -> Result<()> {
    ///     let result = runner.add_cog(cog).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn add_cog<C: Cog>(&self, cog: C) -> Result<()> {
        self.install_cog(Arc::new(cog), true, None, true).await
    }

    /// Validate runtime definitions and ownership before an application action
    /// (such as a database write). Does not run checks, handlers, lifecycle hooks or HTTP.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{BotHandle, CommandInfo, Result};
    /// async fn assess(runner: &BotHandle, commands: &[CommandInfo]) -> Result<()> {
    ///     let plan = runner.preflight_commands(commands, None).await?;
    ///     println!("{:?}", plan.routes);
    ///     Ok(())
    /// }
    /// ```
    pub async fn preflight_commands(
        &self,
        commands: &[crate::CommandInfo],
        replacing_cog: Option<&str>,
    ) -> Result<RegistrationPlan> {
        let _update = self.inner.registry_update.lock().await;
        let mut planned = self.inner.commands.read().await.routes.clone();
        if let Some(name) = replacing_cog {
            if name.is_empty() {
                return Err(YappinError::validation("cog.name", "must not be empty"));
            }
            planned.retain(|_, c| c.cog_name.as_deref() != Some(name));
        }
        for info in commands {
            insert_registered(
                &mut planned,
                crate::RegisteredCommand {
                    info: info.clone(),
                    handler: Arc::new(|_| Box::pin(async { Ok(()) })),
                    preparation: None,
                },
                replacing_cog.map(str::to_owned),
            )?;
        }
        let advertised = advertised_commands(&planned)?;
        let mut routes = planned.keys().cloned().collect::<Vec<_>>();
        routes.sort();
        Ok(RegistrationPlan {
            advertised,
            routes,
            replacing_cog: replacing_cog.map(str::to_owned),
        })
    }

    /// Check a cog's currently exposed definitions before its on_load hook.
    /// Cogs that build definitions in on_load are checked again before publication.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{BotHandle, Cog, Result};
    /// async fn assess(runner: &BotHandle, candidate: &dyn Cog) -> Result<()> {
    ///     let plan = runner.preflight_cog(candidate).await?;
    ///     println!("Advertised: {:?}; routes: {:?}", plan.advertised, plan.routes);
    ///     // Advisory only: installation rechecks conflicts and publication can still fail.
    ///     Ok(())
    /// }
    /// ```
    pub async fn preflight_cog(&self, cog: &dyn Cog) -> Result<RegistrationPlan> {
        if cog.name().is_empty() {
            return Err(YappinError::validation("cog.name", "must not be empty"));
        }
        let mut infos = cog
            .commands()
            .into_iter()
            .map(|c| crate::CommandInfo {
                name: c.name,
                description: c.description,
                aliases: vec![],
                arguments: vec![],
            })
            .collect::<Vec<_>>();
        infos.extend(cog.registered_commands().into_iter().map(|c| c.info));
        self.preflight_commands(&infos, Some(cog.name())).await
    }

    async fn install_cog(
        &self,
        cog: Arc<dyn Cog>,
        synchronize: bool,
        expected: Option<bool>,
        operation_identity_supported: bool,
    ) -> Result<()> {
        let name = cog.name().to_owned();
        if name.is_empty() {
            return Err(YappinError::validation("cog.name", "must not be empty"));
        }
        self.preflight_cog(cog.as_ref()).await?;
        cog.on_load(self.client()).await?;
        let result: Result<Option<Arc<dyn Cog>>> = async {
            // Preserve native cogs that initialize their route metadata in on_load.
            let mut commands = HashMap::new();
            for command in cog.commands() {
                if commands.contains_key(&command.name) {
                    return Err(YappinError::validation(
                        "commands.name",
                        "duplicate command",
                    ));
                }
                commands.insert(
                    command.name,
                    CommandDef {
                        description: command.description,
                        cog_name: Some(name.clone()),
                        handler: command.handler,
                        info: None,
                        preparation: None,
                        operation_identity_supported: true,
                    },
                );
            }
            for command in cog.registered_commands() {
                insert_registered(&mut commands, command, Some(name.clone()))?;
            }
            for command in commands.values_mut() {
                command.operation_identity_supported = operation_identity_supported;
            }
            let buttons = cog.buttons();
            let _update = self.inner.registry_update.lock().await;
            if let Some(present) = expected {
                if self.inner.cogs.read().await.contains_key(&name) != present {
                    return Err(YappinError::validation(
                        "cog.name",
                        "cog ownership changed before publication",
                    ));
                }
            }
            let mut planned = self.inner.commands.read().await.clone();
            for key in commands.keys() {
                if planned
                    .get(key)
                    .is_some_and(|c| c.cog_name.as_deref() != Some(&name))
                {
                    return Err(YappinError::validation(
                        "commands.name",
                        "command belongs to another cog or the bot",
                    ));
                }
            }
            planned
                .routes
                .retain(|_, c| c.cog_name.as_deref() != Some(&name));
            planned.routes.extend(commands.clone());
            let planned = advertised_commands(&planned)?;
            // Do not hold dispatch locks while waiting for the API. Old code stays available.
            if synchronize && self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst)
            {
                let publication = self.publish_commands(&planned).await;
                #[cfg(feature = "hot-reload")]
                let publication = publication.map_err(|cause| {
                    if expected.is_some() {
                        return crate::hot_reload::diagnostics::error(
                            crate::ReloadPhase::Publication,
                            "remote command publication failed; local routes were not replaced",
                            Some(&cause),
                            true,
                            true,
                            Some(self.client()),
                        );
                    }
                    cause
                });
                publication?;
            }
            // Acquire all routing locks before mutation; no await occurs during publication.
            let mut current = self.inner.commands.write().await;
            let mut routes = self.inner.buttons.write().await;
            let mut cogs = self.inner.cogs.write().await;
            current
                .routes
                .retain(|_, c| c.cog_name.as_deref() != Some(&name));
            routes.retain(|b| b.cog_name.as_deref() != Some(&name));
            current.routes.extend(commands);
            current.reindex();
            for button in buttons {
                routes.push(ButtonDef {
                    prefix: button.prefix,
                    cog_name: Some(name.clone()),
                    handler: button.handler,
                });
            }
            Ok(cogs.insert(name.clone(), cog.clone()))
        }
        .await;
        match result {
            Ok(old) => {
                if let Some(old) = old {
                    if let Err(error) = old.on_unload(self.client()).await {
                        warn!("Cog cleanup failed: {error}");
                    }
                }
                if self.inner.log {
                    info!("Cog '{}' installed", name);
                }
                Ok(())
            }
            Err(error) => {
                let _ = cog.on_unload(self.client()).await;
                Err(error)
            }
        }
    }

    #[cfg(feature = "hot-reload")]
    pub(crate) async fn install_worker<C: Cog>(
        &self,
        cog: C,
        replacing: bool,
        operation_identity_supported: bool,
    ) -> Result<()> {
        self.install_cog(
            Arc::new(cog),
            true,
            Some(replacing),
            operation_identity_supported,
        )
        .await
        .map_err(|cause| {
            if matches!(cause, YappinError::HotReload { .. }) {
                cause
            } else {
                crate::hot_reload::diagnostics::error(
                    crate::ReloadPhase::Registration,
                    "candidate registration failed",
                    Some(&cause),
                    true,
                    false,
                    Some(self.client()),
                )
            }
        })
    }

    /// Remove routing atomically. Existing handler snapshots may finish in flight.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, Result};
    /// async fn update(runner: &BotHandle) -> Result<()> {
    ///     let result = runner.remove_cog("status").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn remove_cog(&self, name: &str) -> Result<Option<Arc<dyn Cog>>> {
        let old = {
            let _update = self.inner.registry_update.lock().await;
            if !self.inner.cogs.read().await.contains_key(name) {
                return Ok(None);
            }
            let mut planned = self.inner.commands.read().await.clone();
            planned
                .routes
                .retain(|_, c| c.cog_name.as_deref() != Some(name));
            let planned = advertised_commands(&planned)?;
            if self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst) {
                self.publish_commands(&planned).await?;
            }
            let mut commands = self.inner.commands.write().await;
            let mut buttons = self.inner.buttons.write().await;
            let mut cogs = self.inner.cogs.write().await;
            commands
                .routes
                .retain(|_, c| c.cog_name.as_deref() != Some(name));
            commands.reindex();
            buttons.retain(|b| b.cog_name.as_deref() != Some(name));
            cogs.remove(name)
        };
        if let Some(cog) = &old {
            cog.on_unload(self.client()).await?;
        }
        Ok(old)
    }

    /// Replace an instance built from code already linked into this executable.
    /// For edited Rust source, use the optional hot_reload module.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, Result};
    /// async fn update(runner: &BotHandle, cog: impl rappin::Cog) -> Result<()> {
    ///     let result = runner.reload_cog(cog).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reload_cog<C: Cog>(&self, cog: C) -> Result<()> {
        self.install_cog(Arc::new(cog), true, None, true).await
    }

    /// Recreate a linked cog through its factory; this does not compile source files.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, Result};
    /// async fn update(runner: &BotHandle) -> Result<()> {
    ///     let result = runner.reload_cog_by_name("status").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reload_cog_by_name(&self, name: &str) -> Result<()> {
        let factory = self
            .inner
            .cog_factories
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| {
                YappinError::Config(format!("No factory registered for cog '{name}'"))
            })?;
        let cog: Arc<dyn Cog> = Arc::from(factory());
        if cog.name() != name {
            return Err(YappinError::validation(
                "cog.name",
                "factory returned a different cog name",
            ));
        }
        self.install_cog(cog, true, None, true).await
    }

    async fn publish_commands(&self, commands: &[Command]) -> Result<Vec<Command>> {
        if self.inner.interaction_commands {
            crate::UiApiExt::set_interaction_commands(self.client(), commands, true).await
        } else {
            self.client().set_commands(commands).await
        }
    }
    /// Synchronize a consistent command snapshot without blocking normal dispatch.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{BotHandle, Result};
    /// async fn publish(runner: &BotHandle) -> Result<()> {
    ///     let advertised = runner.sync_commands().await?;
    ///     println!("Published {} roots", advertised.len());
    ///     Ok(())
    /// }
    /// ```
    pub async fn sync_commands(&self) -> Result<Vec<Command>> {
        let _update = self.inner.registry_update.lock().await;
        let commands = advertised_commands(&*self.inner.commands.read().await)?;
        self.publish_commands(&commands).await
    }

    /// Stops the bot's event loop.
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotHandle;
    /// fn inspect(runner: &BotHandle) {
    ///     runner.stop();
    /// }
    /// ```
    pub fn stop(&self) {
        if let Some(ingress) = &self.inner.ingress {
            ingress.close();
        }
        self.inner.running.store(false, Ordering::SeqCst);
        self.inner.stopped.notify_waiters();
        self.inner.stopped.notify_one();
    }
}

/// Builder for configuring and starting a [`Bot`].
/// # Example
///
/// ```rust
/// use rappin::Bot;
/// let bot = Bot::builder("unused-documentation-token")
///     .command("ping", "Check connectivity", |ctx| async move {
///         ctx.reply("pong").await?; Ok(())
///     }).build();
/// // Call bot.run().await only after configuring a real token in the environment.
/// ```
pub struct BotBuilder {
    token: String,
    base_url: String,
    prefixes: Vec<String>,
    prefix_resolver: Option<PrefixResolver>,
    mention_prefix: bool,
    definition_error: Option<String>,
    commands: HashMap<String, CommandDef>,
    buttons: Vec<ButtonDef>,
    cogs: Vec<Box<dyn Cog>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    command_error_handler: Option<CommandErrorHandlerFn>,
    button_error_handler: Option<ButtonErrorHandlerFn>,
    command_failure_handler: Option<CommandFailureHandlerFn>,
    button_failure_handler: Option<ButtonFailureHandlerFn>,
    message_gate: Option<MessageGateFn>,
    message_failure_handler: Option<MessageFailureHandlerFn>,
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
    cursor: Option<EventCursor>,
    max_age: Option<u32>,
    availability_policy: Option<crate::AvailabilityPolicy>,
    ui_events: bool,
    interaction_commands: bool,
    ui_handlers: HashMap<String, UiHandler>,
    handler_concurrency: usize,
    pending_batches: usize,
    drain_timeout: Option<Duration>,
    auto_ack_after: Option<Duration>,
    ingress_config: Option<IngressConfig>,
    ingress_store: Option<Arc<dyn IngressStore>>,
    transport_mode: crate::diagnostics::TransportMode,
    lookup_policy: crate::cache::CachePolicy,
    lookup_backend: Option<Arc<dyn crate::cache::LookupCache>>,
    api_fallback: bool,
    max_retry_wait: Duration,
    #[cfg(feature = "gateway")]
    gateway_url: Option<String>,
}

impl BotBuilder {
    /// # Example
    ///
    /// ```rust
    /// use rappin::BotBuilder;
    /// let builder = BotBuilder::new("unused-documentation-token");
    /// ```
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base_url: DEFAULT_API_BASE.to_string(),
            prefixes: vec!["/".to_string(), "!".to_string()],
            prefix_resolver: None,
            mention_prefix: false,
            definition_error: None,
            commands: HashMap::new(),
            buttons: Vec::new(),
            cogs: Vec::new(),
            event_handler: None,
            command_error_handler: None,
            button_error_handler: None,
            command_failure_handler: None,
            button_failure_handler: None,
            message_gate: None,
            message_failure_handler: None,
            ignore_bots: true,
            sync_commands: true,
            log: true,
            cursor: None,
            max_age: None,
            availability_policy: None,
            ui_events: false,
            interaction_commands: false,
            ui_handlers: HashMap::new(),
            handler_concurrency: 8,
            pending_batches: 4,
            drain_timeout: None,
            auto_ack_after: None,
            ingress_config: None,
            ingress_store: None,
            transport_mode: crate::diagnostics::TransportMode::Polling,
            lookup_policy: crate::cache::CachePolicy::default(),
            lookup_backend: None,
            api_fallback: true,
            max_retry_wait: Duration::from_secs(30),
            #[cfg(feature = "gateway")]
            gateway_url: None,
        }
    }

    /// Sets the API base URL.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").base_url("https://bots.yapper.website/api/bot/v1").build();
    /// ```
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Adds a message command prefix (e.g. `!` or `yap!`).
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").prefix("?").build();
    /// ```
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        let p = prefix.into();
        if !self.prefixes.contains(&p) {
            self.prefixes.push(p);
        }
        self
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .prefixes_with(|ctx| async move {
    ///         // Query an application-owned store by ctx.server_id here.
    ///         Ok(vec!["?".to_owned()])
    ///     }).build();
    /// ```
    pub fn prefixes_with<F, Fut>(mut self, resolver: F) -> Self
    where
        F: Fn(MessageEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<String>>> + Send + 'static,
    {
        self.prefix_resolver = Some(Arc::new(move |event| Box::pin(resolver(event))));
        self
    }
    /// Enable bot-mention command prefixes in addition to configured string prefixes.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").mention_prefix(true).build();
    /// ```
    pub fn mention_prefix(mut self, enabled: bool) -> Self {
        self.mention_prefix = enabled;
        self
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, CommandSpec};
    /// let spec = CommandSpec::<()>::new("status", "Show status");
    /// let bot = Bot::builder("unused-documentation-token").command_spec(spec, |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) }).build();
    /// ```
    pub fn command_spec<A, F, Fut>(
        mut self,
        spec: crate::commands::CommandSpec<A>,
        handler: F,
    ) -> Self
    where
        A: crate::commands::FromArgs,
        F: Fn(MessageContext, A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        if let Err(error) = insert_registered(&mut self.commands, spec.bind(handler), None) {
            self.definition_error = Some(error.to_string());
        }
        self
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").help_command("help").build();
    /// ```
    pub fn help_command(self, name: impl Into<String>) -> Self {
        self.command(name, "Show command usage", |ctx| async move {
            let text = ctx
                .bot()
                .ok_or_else(|| YappinError::Config("help requires a bot handle".into()))?
                .help(ctx.args.trim())
                .await;
            for part in crate::split_message(if text.is_empty() {
                "No matching command"
            } else {
                &text
            }) {
                ctx.reply(part).await?;
            }
            Ok(())
        })
    }

    /// Installs an erased typed or runtime-schema command on the builder.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, CommandSpec};
    /// let spec = CommandSpec::<()>::new("status", "Show status");
    /// let bot = Bot::builder("unused-documentation-token").registered_command(spec.bind(|ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })).build();
    /// ```
    pub fn registered_command(mut self, command: crate::RegisteredCommand) -> Self {
        if let Err(error) = insert_registered(&mut self.commands, command, None) {
            self.definition_error = Some(error.to_string());
        }
        self
    }

    /// Registers a command with a name, description, and async handler.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .command("ping", "Check connectivity", |ctx| async move {
    ///         ctx.reply("pong").await?; Ok(())
    ///     }).build();
    /// // Call bot.run().await only after configuring a real token in the environment.
    /// ```
    pub fn command<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(MessageContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let name_str = name.into().to_lowercase();
        let desc_str = description.into();
        let wrapped_handler: CommandHandlerFn = Arc::new(move |ctx| Box::pin(handler(ctx)));
        if self
            .commands
            .get(&name_str)
            .is_some_and(|command| command.info.is_some())
        {
            self.definition_error = Some("command conflicts with a typed command or alias".into());
            return self;
        }
        self.commands.insert(
            name_str,
            CommandDef {
                description: desc_str,
                cog_name: None,
                handler: wrapped_handler,
                info: None,
                preparation: None,
                operation_identity_supported: true,
            },
        );
        self
    }

    /// Registers a command with an automatic rate limit cooldown.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, cooldown::Cooldown};
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .command_with_cooldown("ping", "Check connectivity", Cooldown::user(std::time::Duration::from_secs(5)),
    ///         |ctx| async move { ctx.reply("pong").await?; Ok(()) }).build();
    /// ```
    pub fn command_with_cooldown<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        cooldown: crate::cooldown::Cooldown,
        handler: F,
    ) -> Self
    where
        F: Fn(MessageContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let name_str = name.into().to_lowercase();
        let desc_str = description.into();
        let handler = Arc::new(handler);
        let wrapped_handler: CommandHandlerFn = Arc::new(move |ctx| {
            let cd = cooldown.clone();
            let handler = handler.clone();
            Box::pin(async move {
                if let Some(remaining) = cd.check(&ctx).await {
                    let secs = remaining.as_secs_f32();
                    return Err(crate::CommandError::Denied(format!(
                        "You are on cooldown; wait {secs:.1}s"
                    ))
                    .into());
                }
                handler(ctx).await
            })
        });
        if self
            .commands
            .get(&name_str)
            .is_some_and(|command| command.info.is_some())
        {
            self.definition_error = Some("command conflicts with a typed command or alias".into());
            return self;
        }
        self.commands.insert(
            name_str,
            CommandDef {
                description: desc_str,
                cog_name: None,
                handler: wrapped_handler,
                info: None,
                preparation: None,
                operation_identity_supported: true,
            },
        );
        self
    }

    /// Enable extended interaction events for native/cog/worker UI hooks.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").interactive_events(true).build();
    /// ```
    pub fn interactive_events(mut self, enabled: bool) -> Self {
        self.ui_events = enabled;
        self
    }
    /// Opt into slash command interaction events, leaving message commands unchanged.
    /// Register an on_command_interaction handler or cog UI hook to answer them.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").interaction_commands(true).build();
    /// ```
    pub fn interaction_commands(mut self, enabled: bool) -> Self {
        self.interaction_commands = enabled;
        if enabled {
            self.ui_events = true;
        }
        self
    }
    fn ui_handler<F, Fut>(mut self, kind: &str, handler: F) -> Self
    where
        F: Fn(crate::UiInteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.ui_events = true;
        self.ui_handlers
            .insert(kind.into(), Arc::new(move |ctx| Box::pin(handler(ctx))));
        self
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").interactive_events(true)
    ///     .on_select(|ctx| async move { ctx.respond_ui(rappin::InteractionCallbackType::Ephemeral, &rappin::UiMessageBody::new("Received")).await }).build();
    /// ```
    pub fn on_select<F, Fut>(self, handler: F) -> Self
    where
        F: Fn(crate::UiInteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.ui_handler("select", handler)
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").interactive_events(true)
    ///     .on_modal(|ctx| async move { ctx.respond_ui(rappin::InteractionCallbackType::Ephemeral, &rappin::UiMessageBody::new("Received")).await }).build();
    /// ```
    pub fn on_modal<F, Fut>(self, handler: F) -> Self
    where
        F: Fn(crate::UiInteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.ui_handler("modal_submit", handler)
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").interactive_events(true)
    ///     .on_command_interaction(|ctx| async move { ctx.respond_ui(rappin::InteractionCallbackType::Ephemeral, &rappin::UiMessageBody::new("Received")).await }).build();
    /// ```
    pub fn on_command_interaction<F, Fut>(self, handler: F) -> Self
    where
        F: Fn(crate::UiInteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.ui_handler("command", handler)
    }

    /// Registers a handler for button clicks whose `custom_id` starts with `prefix`.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .button("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await }).build();
    /// ```
    pub fn button<F, Fut>(mut self, prefix: impl Into<String>, handler: F) -> Self
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let p = prefix.into();
        let wrapped: ButtonHandlerFn = Arc::new(move |ctx| Box::pin(handler(ctx)));
        self.buttons.push(ButtonDef {
            prefix: p,
            cog_name: None,
            handler: wrapped,
        });
        self
    }

    /// Adds an initial cog to be loaded at startup.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, CogBuilder};
    /// let cog = CogBuilder::new("status").command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) }).build();
    /// let bot = Bot::builder("unused-documentation-token").cog(cog).build();
    /// ```
    pub fn cog(mut self, cog: impl Cog) -> Self {
        self.cogs.push(Box::new(cog));
        self
    }

    /// Attaches a custom [`EventHandler`].
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, bot::NoopEventHandler};
    /// let bot = Bot::builder("unused-documentation-token").event_handler(NoopEventHandler).build();
    /// ```
    pub fn event_handler(mut self, handler: impl EventHandler) -> Self {
        self.event_handler = Some(Arc::new(handler));
        self
    }

    /// Observes unexpected command failures. Returning `()` does not complete them;
    /// use `on_command_failure` for explicit completion. Last registration wins.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// let bot = Bot::builder("yb_token")
    ///     .on_command_error(|_ctx, err| async move {
    ///         eprintln!("Command failed: {err}");
    ///     });
    /// ```
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .on_command_error(|_ctx, error| async move { tracing::error!("Handler failed: {error}"); }).build();
    /// // Legacy callbacks observe failures; returning () does not mark unexpected work handled.
    /// ```
    pub fn on_command_error<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(MessageContext, YappinError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.command_failure_handler = None;
        self.command_error_handler = Some(Arc::new(move |ctx, err| Box::pin(handler(ctx, err))));
        self
    }

    /// Observes unexpected button failures without completing them.
    /// Use `on_button_failure` for explicit completion. Last registration wins.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// let bot = Bot::builder("yb_token")
    ///     .on_button_error(|_ctx, err| async move {
    ///         eprintln!("Button failed: {err}");
    ///     });
    /// ```
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .on_button_error(|_ctx, error| async move { tracing::error!("Handler failed: {error}"); }).build();
    /// // Legacy callbacks observe failures; returning () does not mark unexpected work handled.
    /// ```
    pub fn on_button_error<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(InteractionContext, YappinError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.button_failure_handler = None;
        self.button_error_handler = Some(Arc::new(move |ctx, err| Box::pin(handler(ctx, err))));
        self
    }

    /// Explicitly decide whether a failed invocation completed. Callback errors
    /// remain unprocessed and retain both the original and handling causes.
    /// Replaces `on_command_error`; the last registration selects the callback.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, FailureDisposition};
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .on_command_failure(|_ctx, failure| async move {
    ///         tracing::error!("Incomplete handler: {}", failure.error);
    ///         Ok(FailureDisposition::Stop)
    ///     }).build();
    /// ```
    pub fn on_command_failure<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(MessageContext, HandlerFailure) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<FailureDisposition>> + Send + 'static,
    {
        self.command_error_handler = None;
        self.command_failure_handler = Some(Arc::new(move |ctx, failure| {
            Box::pin(handler(ctx, failure))
        }));
        self
    }
    /// Button counterpart of `on_command_failure`. `Handled` completes only the
    /// failed invocation; remaining matching handlers/listeners still run.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, FailureDisposition};
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .on_button_failure(|_ctx, failure| async move {
    ///         tracing::error!("Incomplete handler: {}", failure.error);
    ///         Ok(FailureDisposition::Stop)
    ///     }).build();
    /// ```
    pub fn on_button_failure<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(InteractionContext, HandlerFailure) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<FailureDisposition>> + Send + 'static,
    {
        self.button_error_handler = None;
        self.button_failure_handler = Some(Arc::new(move |ctx, failure| {
            Box::pin(handler(ctx, failure))
        }));
        self
    }

    /// Whether to ignore messages authored by other bots (default `true`).
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").ignore_bots(true).build();
    /// ```
    pub fn ignore_bots(mut self, ignore: bool) -> Self {
        self.ignore_bots = ignore;
        self
    }

    /// Whether to automatically sync registered commands to Yapin' on startup (default `true`).
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").sync_commands(false).build();
    /// ```
    pub fn sync_commands(mut self, sync: bool) -> Self {
        self.sync_commands = sync;
        self
    }

    /// Whether to log status messages to console (default `true`).
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").log(false).build();
    /// ```
    pub fn log(mut self, enable_log: bool) -> Self {
        self.log = enable_log;
        self
    }

    /// Opt-in gate for server-message events, after bot/target filters and before
    /// prefix resolution, help replies, cog checks, commands and message listeners.
    /// The context has no resolved command/args yet. This does not filter EventHub.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, MessageDisposition};
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .message_gate(|ctx| async move {
    ///         // Evaluate current application policy before prefix parsing or command checks.
    ///         Ok(MessageDisposition::Continue)
    ///     }).build();
    /// ```
    pub fn message_gate<F, Fut>(mut self, gate: F) -> Self
    where
        F: Fn(MessageContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<MessageDisposition>> + Send + 'static,
    {
        self.message_gate = Some(Arc::new(move |ctx| Box::pin(gate(ctx))));
        self
    }

    /// Complete/reconcile a gate or fallible message-listener failure explicitly.
    /// Default replies to typed rejections; unexpected errors remain incomplete.
    /// Handled completes a gate rejection without dispatch, or a listener invocation
    /// before remaining listeners. Callback errors preserve both structured causes.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, FailureDisposition};
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .on_message_failure(|_ctx, failure| async move {
    ///         tracing::error!("Incomplete handler: {}", failure.error);
    ///         Ok(FailureDisposition::Stop)
    ///     }).build();
    /// ```
    pub fn on_message_failure<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(MessageContext, MessageFailure) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<FailureDisposition>> + Send + 'static,
    {
        self.message_failure_handler = Some(Arc::new(move |ctx, failure| {
            Box::pin(handler(ctx, failure))
        }));
        self
    }

    /// Resume after the ordered completed event prefix. A cursor file has one owner.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").cursor_file("./state/events.json").build();
    /// ```
    pub fn cursor_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.cursor = Some(EventCursor::new(path));
        self
    }
    /// Opt in to bounded, cancellable startup/intake read recovery. This policy
    /// owns retry attempts; client retries are suppressed only inside these reads.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").availability_policy(rappin::AvailabilityPolicy::default()).build();
    /// ```
    pub fn availability_policy(mut self, policy: crate::AvailabilityPolicy) -> Self {
        self.availability_policy = Some(policy);
        self
    }
    /// Set the maximum event age in seconds requested during polling.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").max_event_age(900).build();
    /// ```
    pub fn max_event_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }
    /// Maximum concurrent events. Use 1 for strict event ordering.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").handler_concurrency(4).build();
    /// ```
    pub fn handler_concurrency(mut self, limit: usize) -> Self {
        self.handler_concurrency = limit.clamp(1, 100);
        self
    }
    /// Enable bounded shared webhook/polling admission. Existing intake remains
    /// unchanged unless explicitly enabled. GatewayOnly requires polling recovery
    /// and is therefore rejected with shared ingress.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").shared_ingress(rappin::IngressConfig::default()).build();
    /// ```
    pub fn shared_ingress(mut self, config: IngressConfig) -> Self {
        self.ingress_config = Some(config);
        self
    }
    /// Application-owned durable receipts; enables shared ingress with defaults.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, IngressStore};
    /// use std::sync::Arc;
    /// fn configured(storage: Arc<dyn IngressStore>) {
    ///     let bot = Bot::builder("unused-documentation-token").ingress_store(storage).build();
    /// }
    /// ```
    pub fn ingress_store(mut self, store: Arc<dyn IngressStore>) -> Self {
        self.ingress_config.get_or_insert_with(Default::default);
        self.ingress_store = Some(store);
        self
    }

    /// Bound shutdown draining of already admitted handlers. Intake stops first;
    /// gateway API responses/heartbeats stay alive until completion or this limit.
    /// None (the default) preserves legacy unbounded draining. Hard task abort is
    /// cancellation, not graceful draining; uncertain effects must be reconciled.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").drain_timeout(Some(std::time::Duration::from_secs(30))).build();
    /// ```
    pub fn drain_timeout(mut self, timeout: Option<Duration>) -> Self {
        if timeout.is_some_and(|value| value.is_zero() || value > Duration::from_secs(3600)) {
            self.definition_error =
                Some("drain timeout must be greater than zero and at most one hour".into());
        }
        self.drain_timeout = timeout;
        self
    }
    /// Bounds prepared intake and active batches. Shared polling may hold one
    /// additional raw response while awaiting admission, before any intake effects.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").pending_batches(4).build();
    /// ```
    pub fn pending_batches(mut self, limit: usize) -> Self {
        self.pending_batches = limit.clamp(1, 32);
        self
    }
    /// Choose polling, preferred gateway or gateway-only intake; non-polling modes require gateway support and configuration.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").transport_mode(rappin::diagnostics::TransportMode::Polling).build();
    /// ```
    pub fn transport_mode(mut self, mode: crate::diagnostics::TransportMode) -> Self {
        self.transport_mode = mode;
        self
    }
    /// Opt in to bounded in-memory member/role snapshots; authorization freshness remains explicit.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").lookup_cache(rappin::CachePolicy::new(128, std::time::Duration::from_secs(15))).build();
    /// ```
    pub fn lookup_cache(mut self, policy: crate::cache::CachePolicy) -> Self {
        self.lookup_policy = policy;
        self
    }
    /// Use your own asynchronous member/role cache (e.g. Redis or SQL).
    /// The backend controls storage, expiration and failures. No database is required.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, LookupCache};
    /// use std::sync::Arc;
    /// fn configured(storage: Arc<dyn LookupCache>) {
    ///     let bot = Bot::builder("unused-documentation-token").lookup_cache_backend(storage).build();
    /// }
    /// ```
    pub fn lookup_cache_backend(mut self, backend: Arc<dyn crate::cache::LookupCache>) -> Self {
        self.lookup_backend = Some(backend);
        self
    }
    /// Configure known-host REST fallback; custom base URLs never fail over.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").api_fallback(true).build();
    /// ```
    pub fn api_fallback(mut self, enabled: bool) -> Self {
        self.api_fallback = enabled;
        self
    }
    /// Opt-in early acknowledgement. Use a short delay (e.g. 250ms); already answered clicks are skipped.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").auto_ack_after(std::time::Duration::from_millis(500)).build();
    /// ```
    pub fn auto_ack_after(mut self, after: Duration) -> Self {
        self.auto_ack_after = Some(after);
        self
    }
    /// Gateway automatically resumes and falls back to polling when unavailable.
    #[cfg(feature = "gateway")]
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").gateway(rappin::gateway::DEFAULT_GATEWAY).build();
    /// ```
    pub fn gateway(mut self, url: impl Into<String>) -> Self {
        self.gateway_url = Some(url.into());
        self.transport_mode = crate::diagnostics::TransportMode::GatewayPreferred;
        self
    }
    #[cfg(feature = "gateway")]
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").gateway_only(rappin::gateway::DEFAULT_GATEWAY).build();
    /// ```
    pub fn gateway_only(self, url: impl Into<String>) -> Self {
        self.gateway(url)
            .transport_mode(crate::diagnostics::TransportMode::GatewayOnly)
    }

    /// Total server-advised rate-limit waiting per API request, default 30 seconds.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token").max_retry_wait(std::time::Duration::from_secs(10)).build();
    /// ```
    pub fn max_retry_wait(mut self, budget: Duration) -> Self {
        self.max_retry_wait = budget;
        self
    }

    /// Builds the configured [`Bot`].
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .command("ping", "Check connectivity", |ctx| async move {
    ///         ctx.reply("pong").await?; Ok(())
    ///     }).build();
    /// // Call bot.run().await only after configuring a real token in the environment.
    /// ```
    pub fn build(mut self) -> Bot {
        prepare_prefixes(&mut self.prefixes);
        let mut client = Client::builder(self.token)
            .base_url(self.base_url)
            .lookup_cache(self.lookup_policy)
            .api_fallback(self.api_fallback)
            .max_retry_wait(self.max_retry_wait);
        if let Some(backend) = self.lookup_backend {
            client = client.lookup_cache_backend(backend);
        }
        let client = client.build();

        let state = Arc::new(BotState {
            client: Arc::new(client),
            prefixes: self.prefixes,
            prefix_resolver: self.prefix_resolver,
            mention_prefix: self.mention_prefix,
            definition_error: self.definition_error,
            commands: RwLock::new(CommandRegistry::new(self.commands)),
            buttons: RwLock::new(self.buttons),
            cogs: RwLock::new(HashMap::new()),
            cog_factories: RwLock::new(HashMap::new()),
            event_handler: self.event_handler,
            command_error_handler: self.command_error_handler,
            button_error_handler: self.button_error_handler,
            command_failure_handler: self.command_failure_handler,
            button_failure_handler: self.button_failure_handler,
            message_gate: self.message_gate,
            message_failure_handler: self.message_failure_handler,
            ignore_bots: self.ignore_bots,
            sync_commands: self.sync_commands,
            log: self.log,
            cursor: self.cursor,
            max_age: self.max_age,
            availability_policy: self.availability_policy,
            ui_events: self.ui_events,
            interaction_commands: self.interaction_commands,
            ui_handlers: self.ui_handlers,
            handler_concurrency: self.handler_concurrency,
            pending_batches: self.pending_batches,
            drain_timeout: self.drain_timeout,
            auto_ack_after: self.auto_ack_after,
            ingress: self
                .ingress_config
                .map(|config| EventIngress::new(config, self.ingress_store)),
            telemetry: crate::diagnostics::Telemetry::new(self.transport_mode),
            availability_telemetry: Default::default(),
            #[cfg(feature = "gateway")]
            gateway_url: self.gateway_url,
            registry_update: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
            draining: AtomicBool::new(false),
            stopped: tokio::sync::Notify::new(),
        });

        Bot {
            handle: BotHandle { inner: state },
            initial_cogs: self.cogs,
        }
    }
}

/// The core bot runner that connects to Yapin', handles long-polling, and routes events.
/// # Example
///
/// ```rust
/// use rappin::Bot;
/// let bot = Bot::builder("unused-documentation-token")
///     .command("ping", "Check connectivity", |ctx| async move {
///         ctx.reply("pong").await?; Ok(())
///     }).build();
/// // Call bot.run().await only after configuring a real token in the environment.
/// ```
pub struct Bot {
    handle: BotHandle,
    initial_cogs: Vec<Box<dyn Cog>>,
}

impl Bot {
    /// Read the latest availability snapshot; watch updates may coalesce.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     let status = runner.availability_status();
    ///     println!("Availability: {:?}", status);
    /// }
    /// ```
    pub fn availability_status(&self) -> crate::AvailabilityStatus {
        self.handle.availability_status()
    }
    /// Subscribe to the latest availability state; use cumulative counters for totals.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     let updates = runner.subscribe_availability();
    /// }
    /// ```
    pub fn subscribe_availability(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::AvailabilityStatus> {
        self.handle.subscribe_availability()
    }
    async fn available_read<T, F, Fut>(
        &self,
        phase: crate::AvailabilityPhase,
        mut read: F,
    ) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let Some(policy) = &self.handle.inner.availability_policy else {
            return read().await;
        };
        let mut episode =
            crate::availability::Episode::new(self.handle.clone(), policy.clone(), phase);
        loop {
            match episode.attempt(read()).await {
                Ok(value) => {
                    episode.success();
                    return Ok(value);
                }
                Err(error) => episode.retry(error).await?,
            }
        }
    }

    /// Read current active event transport and fallback state.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     let status = runner.transport_status();
    ///     println!("Transport: {:?}", status);
    /// }
    /// ```
    pub fn transport_status(&self) -> crate::diagnostics::TransportStatus {
        self.handle.transport_status()
    }
    /// Access the event collector hub owned by this bot.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     let hub = runner.events();
    /// }
    /// ```
    pub fn events(&self) -> &crate::collectors::EventHub {
        self.client().events()
    }
    /// Creates a [`BotBuilder`] for the specified bot token.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .command("ping", "Check connectivity", |ctx| async move {
    ///         ctx.reply("pong").await?; Ok(())
    ///     }).build();
    /// // Call bot.run().await only after configuring a real token in the environment.
    /// ```
    pub fn builder(token: impl Into<String>) -> BotBuilder {
        BotBuilder::new(token)
    }

    /// Returns a [`BotHandle`] for managing cogs and state at runtime.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// let bot = Bot::builder("unused-documentation-token")
    ///     .command("ping", "Check connectivity", |ctx| async move {
    ///         ctx.reply("pong").await?; Ok(())
    ///     }).build();
    /// // Call bot.run().await only after configuring a real token in the environment.
    /// let handle = bot.handle();
    /// ```
    pub fn handle(&self) -> BotHandle {
        self.handle.clone()
    }

    /// Returns a reference to the underlying [`Client`].
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     let client = runner.client();
    /// }
    /// ```
    pub fn client(&self) -> &Client {
        self.handle.client()
    }

    /// Returns a reference to the bot's internal in-memory cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     let cache = runner.cache();
    /// }
    /// ```
    pub fn cache(&self) -> &crate::cache::Cache {
        self.handle.cache()
    }

    /// Adds a cog to the bot. Can be called before or while the bot is running.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Bot, Result};
    /// async fn update(runner: &Bot, cog: impl rappin::Cog) -> Result<()> {
    ///     let result = runner.add_cog(cog).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn add_cog(&self, cog: impl Cog) -> Result<()> {
        self.handle.add_cog(cog).await
    }

    /// Removes a cog by name from the bot.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Bot, Result};
    /// async fn update(runner: &Bot) -> Result<()> {
    ///     let result = runner.remove_cog("status").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn remove_cog(&self, name: &str) -> Result<Option<Arc<dyn Cog>>> {
        self.handle.remove_cog(name).await
    }

    /// Reloads a cog.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Bot, Result};
    /// async fn update(runner: &Bot, cog: impl rappin::Cog) -> Result<()> {
    ///     let result = runner.reload_cog(cog).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reload_cog(&self, cog: impl Cog) -> Result<()> {
        self.handle.reload_cog(cog).await
    }

    /// Registers a factory to instantiate a cog by name.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Bot, CogBuilder, Result};
    /// async fn install(runner: &Bot) {
    ///     runner.register_cog_factory("status", || Box::new(CogBuilder::new("status").build())).await;
    /// }
    /// ```
    pub async fn register_cog_factory<F>(&self, name: impl Into<String>, factory: F)
    where
        F: Fn() -> Box<dyn Cog> + Send + Sync + 'static,
    {
        self.handle.register_cog_factory(name, factory).await
    }

    /// Reloads a cog by name using its registered factory.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Bot, Result};
    /// async fn update(runner: &Bot) -> Result<()> {
    ///     let result = runner.reload_cog_by_name("status").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reload_cog_by_name(&self, name: &str) -> Result<()> {
        self.handle.reload_cog_by_name(name).await
    }

    /// Stops the bot long-polling loop gracefully.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Bot;
    /// fn inspect(runner: &Bot) {
    ///     runner.stop();
    /// }
    /// ```
    pub fn stop(&self) {
        self.handle.stop();
    }

    /// Runs the bot until stopped or Ctrl+C is received.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Bot, Result, YappinError};
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let token = std::env::var("YAPPER_BOT_TOKEN").map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
    ///     Bot::builder(token).build().run().await
    /// }
    /// ```
    pub async fn run(self) -> Result<()> {
        struct Running(BotHandle);
        impl Drop for Running {
            fn drop(&mut self) {
                self.0.stop();
                self.0.inner.client.events().close();
                self.0.inner.client.cancel_auto_ack();
                self.0.inner.telemetry.update(|s| {
                    s.active = None;
                    s.heartbeat_rtt = None;
                    s.pending_batches = 0;
                });
            }
        }
        let _running = Running(self.handle.clone());
        self.run_inner().await
    }

    async fn run_inner(mut self) -> Result<()> {
        if let Some(error) = &self.handle.inner.definition_error {
            return Err(YappinError::Config(error.clone()));
        }
        advertised_commands(&*self.handle.inner.commands.read().await)?;
        if let Some(ingress) = &self.handle.inner.ingress {
            ingress.validate()?;
            if self.transport_status().mode == crate::TransportMode::GatewayOnly {
                return Err(YappinError::Config("shared ingress requires authoritative polling recovery; use Polling or GatewayPreferred".into()));
            }
        }
        #[cfg(not(feature = "gateway"))]
        if self.transport_status().mode != crate::diagnostics::TransportMode::Polling {
            return Err(YappinError::Config(
                "enable the gateway Cargo feature for this transport mode".into(),
            ));
        }
        self.handle.inner.running.store(true, Ordering::SeqCst);

        // 1. Fetch bot user details to verify token
        let me: BotProfile = match self
            .available_read(crate::AvailabilityPhase::StartupIdentity, || {
                self.client().get_me()
            })
            .await
        {
            Ok(profile) => profile,
            Err(e) => {
                if let YappinError::Unauthorized = e {
                    eprintln!("Token rejected: generate a new one in Settings -> Developer");
                }
                return Err(e);
            }
        };

        // Resolve durable recovery before lifecycle hooks or command publication
        // can perform application/API effects. The default intake order is kept.
        let shared_last = if let Some(ingress) = &self.handle.inner.ingress {
            let saved = match &self.handle.inner.cursor {
                Some(cursor) => cursor.load().await?,
                None => None,
            };
            Some(ingress.restore(me.id, saved).await?.or(saved).unwrap_or(0))
        } else {
            None
        };

        // Initial cogs use the same validated, atomic registration path.
        for cog in self.initial_cogs.drain(..) {
            self.handle
                .install_cog(Arc::from(cog), false, None, true)
                .await?;
        }

        // 3. Register slash commands if enabled
        if self.handle.inner.sync_commands {
            if let Err(e) = self.handle.sync_commands().await {
                warn!("Failed to synchronize commands: {}", e);
            }
        }

        let saved = match shared_last {
            Some(last) => Some(last),
            None => match &self.handle.inner.cursor {
                Some(cursor) => cursor.load().await?,
                None => None,
            },
        };
        let last = match saved {
            Some(id) => id,
            None if self.transport_status().mode
                == crate::diagnostics::TransportMode::GatewayOnly =>
            {
                0
            }
            None => {
                self.available_read(crate::AvailabilityPhase::StartupCursor, || {
                    self.client().get_events(Some(0), Some(0), None)
                })
                .await?
                .last
            }
        };
        if saved != Some(last) {
            if let Some(c) = &self.handle.inner.cursor {
                c.save(last).await?;
            }
        }
        if self.handle.inner.log {
            println!("[rappin] Logged in as {} ({})", me.username, me.id);
        }

        // Emit ready to event handler
        if let Some(ref handler) = self.handle.inner.event_handler {
            handler.on_ready(self.handle.client(), &me).await;
        }

        self.run_pipeline(&me, last).await
    }

    /// Dispatch an already authenticated event from a custom transport.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Bot, Event, Result};
    /// async fn verified_delivery(bot: &Bot, authenticated_event: Event) -> Result<()> {
    ///     // The caller must authenticate delivery and own deduplication/checkpointing.
    ///     // Prefer shared ingress for mixed webhook/polling intake.
    ///     bot.process_event(authenticated_event).await
    /// }
    /// ```
    pub async fn process_event(&self, event: crate::models::Event) -> Result<()> {
        if let Some(error) = &self.handle.inner.definition_error {
            return Err(YappinError::Config(error.clone()));
        }
        let me = match self.cache().get_me().await {
            Some(me) => me,
            None => self.client().get_me().await?,
        };
        let mut prepared = None;
        if event.kind == "interaction" {
            if let Ok(EventData::Interaction(mut interaction)) = event.parse_data() {
                interaction.created_at = interaction.created_at.or(event.created_at);
                self.client().track_interaction(&interaction).await?;
                if let Some(after) = self.handle.inner.auto_ack_after {
                    self.client().auto_ack(interaction.clone(), after);
                }
                prepared = Some(Box::new(interaction));
            }
        }
        self.client().receive_event(event.clone()).await;
        failure::catch(self.dispatch_event(event, &me, prepared)).await
    }

    async fn dispatch_event(
        &self,
        event: crate::models::Event,
        me: &BotProfile,
        prepared: Option<Box<InteractionEvent>>,
    ) -> Result<()> {
        if self.handle.inner.ui_events
            && (event.kind == "command"
                || event.kind == "interaction"
                    && matches!(event.data["type"].as_str(), Some("select" | "modal_submit")))
        {
            let data = crate::UiInteraction::from_event(&event)?;
            self.client().track_interaction(&data.interaction).await?;
            return self.handle_ui_interaction(data, event.id).await;
        }
        let parsed = match prepared
            .map(|interaction| Ok(EventData::Interaction(*interaction)))
            .unwrap_or_else(|| event.parse_data())
        {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to deserialize event {}: {}", event.kind, e);
                return Err(e.into());
            }
        };

        match parsed {
            EventData::Message(msg) => self.handle_message(msg, me, event.id).await?,
            EventData::DmMessage(message) => {
                let message = message.0;
                if self.handle.inner.ignore_bots && message.author_bot == Some(true) {
                    return Ok(());
                }
                let ctx = MessageContext::new(
                    self.handle.inner.client.clone(),
                    message,
                    None,
                    String::new(),
                    Some(self.handle.clone()),
                );
                if let Some(handler) = &self.handle.inner.event_handler {
                    handler.on_dm(&ctx).await?;
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    cog.on_dm(&ctx).await?;
                }
            }
            EventData::Interaction(inter) => self.handle_interaction(inter, event.id).await?,
            EventData::MemberJoin(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_member_join(self.handle.client(), &ev).await;
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    cog.on_member_join(self.handle.client(), &ev).await;
                }
            }
            EventData::MemberLeave(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_member_leave(self.handle.client(), &ev).await;
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    cog.on_member_leave(self.handle.client(), &ev).await;
                }
            }
            EventData::ReactionAdd(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_reaction_add(self.handle.client(), &ev).await;
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    cog.on_reaction_add(self.handle.client(), &ev).await?;
                }
            }
            EventData::ReactionRemove(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_reaction_remove(self.handle.client(), &ev).await;
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    cog.on_reaction_remove(self.handle.client(), &ev).await?;
                }
            }
            EventData::VoiceJoin(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_voice_join(self.handle.client(), &ev).await;
                }
            }
            EventData::VoiceLeave(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_voice_leave(self.handle.client(), &ev).await;
                }
            }
            EventData::Test(val) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_test(self.handle.client(), &val).await;
                }
            }
            EventData::InviteCreate(ev) | EventData::InviteDelete(ev) => {
                let created = event.kind == "invite_create";
                if let Some(ref h) = self.handle.inner.event_handler {
                    if created {
                        h.on_invite_create(self.handle.client(), &ev).await;
                    } else {
                        h.on_invite_delete(self.handle.client(), &ev).await;
                    }
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    if created {
                        cog.on_invite_create(self.handle.client(), &ev).await;
                    } else {
                        cog.on_invite_delete(self.handle.client(), &ev).await;
                    }
                }
            }
            EventData::Resource(resource) => {
                self.handle
                    .client()
                    .invalidate_server_lookups(resource.server_id)
                    .await;
                self.handle
                    .client()
                    .cache()
                    .invalidate_resource(&resource)
                    .await;
                if let Some(ref handler) = self.handle.inner.event_handler {
                    failure::catch(handler.on_resource_event(self.handle.client(), &resource))
                        .await?;
                }
                let cogs: Vec<_> = self
                    .handle
                    .inner
                    .cogs
                    .read()
                    .await
                    .values()
                    .cloned()
                    .collect();
                for cog in cogs {
                    failure::catch(cog.on_resource_event(self.handle.client(), &resource)).await?;
                }
            }
            EventData::Unknown { kind, data } => {
                debug!("Received unhandled event type: {}", kind);
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_unknown_event(self.handle.client(), &kind, &data).await;
                }
            }
        }
        Ok(())
    }

    async fn handle_message(
        &self,
        msg: MessageEvent,
        me: &BotProfile,
        event_id: u64,
    ) -> Result<()> {
        if self.handle.inner.ignore_bots && msg.author_bot == Some(true) {
            return Ok(());
        }

        // If command_for list is provided, verify whether our bot was targeted
        if let Some(ref targeted) = msg.command_for {
            if !targeted.contains(&me.id) {
                return Ok(());
            }
        }

        if let Some(gate) = &self.handle.inner.message_gate {
            let ctx = MessageContext::new(
                self.handle.inner.client.clone(),
                msg.clone(),
                None,
                String::new(),
                Some(self.handle.clone()),
            );
            let result = failure::catch(async { gate(ctx.clone()).await }).await;
            if self
                .message_result(&ctx, event_id, MessageStage::Gate, "gate", result)
                .await?
            {
                return Ok(());
            }
        }

        invocation::with_origin(
            &self.handle,
            &msg,
            self.handle_allowed_message(msg.clone(), me, event_id),
        )
        .await
    }

    async fn handle_allowed_message(
        &self,
        msg: MessageEvent,
        me: &BotProfile,
        event_id: u64,
    ) -> Result<()> {
        let mut matched_command: Option<String> = None;
        let mut matched_args = String::new();

        // Check if message comes pre-parsed as a slash command
        if let Some(ref cmd_name) = msg.command {
            matched_command = Some(cmd_name.to_lowercase());
            matched_args = msg.args.clone().unwrap_or_default();
        } else if let Some(ref content) = msg.content {
            // Check prefix commands
            let mut prefixes = if let Some(resolver) = &self.handle.inner.prefix_resolver {
                let mut prefixes = resolver(msg.clone()).await?;
                prepare_prefixes(&mut prefixes);
                std::borrow::Cow::Owned(prefixes)
            } else {
                std::borrow::Cow::Borrowed(self.handle.inner.prefixes.as_slice())
            };
            if self.handle.inner.mention_prefix {
                let prefixes = prefixes.to_mut();
                prefixes.push(format!("<@{}>", me.id));
                prefixes.push(format!("<@!{}>", me.id));
                prepare_prefixes(prefixes);
            }

            let cmds_guard = self.handle.inner.commands.read().await;
            for p in prefixes.iter() {
                if content.starts_with(p) {
                    let rest = content[p.len()..].trim_start();
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    if let Some(cmd_name) = parts.next() {
                        let cmd_lower = cmd_name.to_lowercase();
                        if cmds_guard.roots.contains(&cmd_lower) {
                            matched_command = Some(cmd_lower);
                            matched_args = parts.next().unwrap_or_default().trim().to_string();
                            break;
                        }
                    }
                }
            }
        }

        let mut unmatched_group = None;
        if let Some(root) = matched_command.clone() {
            if let Some((path, args)) = self.handle.resolve_invocation(&root, &matched_args).await {
                matched_command = Some(path);
                matched_args = args;
            } else {
                unmatched_group = Some(root);
                matched_command = None;
            }
        }

        let ctx = MessageContext::new(
            self.handle.inner.client.clone(),
            msg,
            matched_command.clone(),
            matched_args,
            Some(self.handle.clone()),
        );

        if let Some(group) = unmatched_group {
            let text = self.handle.help(&group).await;
            for part in crate::split_message(&text) {
                ctx.reply(part).await?;
            }
        }
        if let Some(cmd_name) = matched_command {
            let snapshot = self.handle.invocation_snapshot(&cmd_name).await;
            if let Some(snapshot) = snapshot {
                let usage = snapshot.def.info.as_ref().map(|info| info.usage());
                if let Some(error) = self
                    .handle
                    .dispatch_invocation(ctx.clone(), snapshot)
                    .await
                    .error
                {
                    self.command_failed(ctx.clone(), event_id, &cmd_name, usage, error)
                        .await?;
                }
            }
        }
        if let Some(h) = &self.handle.inner.event_handler {
            let result = failure::catch(h.on_message_result(&ctx)).await;
            if self
                .message_result(
                    &ctx,
                    event_id,
                    MessageStage::Listener,
                    "event_handler",
                    result,
                )
                .await?
            {
                return Ok(());
            }
        }
        let cogs: Vec<_> = self
            .handle
            .inner
            .cogs
            .read()
            .await
            .values()
            .cloned()
            .collect();
        for cog in cogs {
            let name = cog.name();
            let result = failure::catch(cog.on_message_result(&ctx)).await;
            if self
                .message_result(&ctx, event_id, MessageStage::Listener, name, result)
                .await?
            {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn handle_ui_interaction(&self, data: crate::UiInteraction, event_id: u64) -> Result<()> {
        let kind = data.interaction.kind.clone().unwrap_or_default();
        let ctx = crate::UiInteractionContext::new(
            self.handle.inner.client.clone(),
            data,
            Some(self.handle.clone()),
        );
        if let Some(handler) = self.handle.inner.ui_handlers.get(&kind) {
            if let Err(error) = failure::catch(handler(ctx.clone())).await {
                self.button_failed(ctx.context.clone(), event_id, &format!("ui:{kind}"), error)
                    .await?;
            }
        }
        let cogs: Vec<_> = self
            .handle
            .inner
            .cogs
            .read()
            .await
            .values()
            .cloned()
            .collect();
        for cog in cogs {
            if let Err(error) = failure::catch(cog.on_ui_interaction(&ctx)).await {
                self.button_failed(ctx.context.clone(), event_id, cog.name(), error)
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_interaction(&self, inter: InteractionEvent, event_id: u64) -> Result<()> {
        let custom_id = inter.custom_id.clone().unwrap_or_default();
        let ctx = InteractionContext::new(
            self.handle.inner.client.clone(),
            inter,
            Some(self.handle.clone()),
        );
        let handlers: Vec<_> = self
            .handle
            .inner
            .buttons
            .read()
            .await
            .iter()
            .filter(|b| custom_id.starts_with(&b.prefix))
            .map(|b| (b.prefix.clone(), b.handler.clone()))
            .collect();
        for (prefix, handler) in handlers {
            if let Err(error) = failure::catch(async { handler(ctx.clone()).await }).await {
                self.button_failed(ctx.clone(), event_id, &prefix, error)
                    .await?;
            }
        }
        if let Some(h) = &self.handle.inner.event_handler {
            h.on_interaction(&ctx).await;
        }
        let cogs: Vec<_> = self
            .handle
            .inner
            .cogs
            .read()
            .await
            .values()
            .cloned()
            .collect();
        for cog in cogs {
            cog.on_interaction(&ctx).await;
        }
        Ok(())
    }
}
