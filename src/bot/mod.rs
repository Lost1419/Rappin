pub mod cog;
pub mod context;
pub mod handler;
mod pipeline;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::events::EventCursor;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

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

pub type CommandErrorHandlerFn = Arc<
    dyn Fn(MessageContext, YappinError) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

pub type ButtonErrorHandlerFn = Arc<
    dyn Fn(InteractionContext, YappinError) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

struct BotState {
    client: Arc<Client>,
    prefixes: Vec<String>,
    prefix_resolver: Option<PrefixResolver>,
    mention_prefix: bool,
    definition_error: Option<String>,
    commands: RwLock<HashMap<String, CommandDef>>,
    buttons: RwLock<Vec<ButtonDef>>,
    cogs: RwLock<HashMap<String, Arc<dyn Cog>>>,
    cog_factories: RwLock<HashMap<String, CogFactoryFn>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    command_error_handler: Option<CommandErrorHandlerFn>,
    button_error_handler: Option<ButtonErrorHandlerFn>,
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
    cursor: Option<EventCursor>,
    max_age: Option<u32>,
    handler_concurrency: usize,
    pending_batches: usize,
    auto_ack_after: Option<Duration>,
    telemetry: crate::diagnostics::Telemetry,
    #[cfg(feature = "gateway")]
    gateway_url: Option<String>,
    registry_update: tokio::sync::Mutex<()>,
    running: AtomicBool,
    stopped: tokio::sync::Notify,
}

/// A cloneable handle to control the bot and dynamically manage cogs while running.
#[derive(Clone)]
pub struct BotHandle {
    inner: Arc<BotState>,
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
    pub fn transport_status(&self) -> crate::diagnostics::TransportStatus {
        self.inner.telemetry.status()
    }
    pub fn subscribe_transport(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::diagnostics::TransportStatus> {
        self.inner.telemetry.subscribe()
    }
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
    pub fn client(&self) -> &Client {
        &self.inner.client
    }

    /// Returns a reference to the client's internal in-memory cache.
    pub fn cache(&self) -> &crate::cache::Cache {
        self.inner.client.cache()
    }

    /// Checks if a cog with the given name is currently loaded.
    pub async fn has_cog(&self, name: &str) -> bool {
        self.inner.cogs.read().await.contains_key(name)
    }

    /// Lists the names of all currently loaded cogs.
    pub async fn list_cogs(&self) -> Vec<String> {
        self.inner.cogs.read().await.keys().cloned().collect()
    }

    /// Registers a factory function to instantiate a cog by name dynamically.
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
    pub async fn add_cog<C: Cog>(&self, cog: C) -> Result<()> {
        self.install_cog(Arc::new(cog), true, None).await
    }

    async fn install_cog(
        &self,
        cog: Arc<dyn Cog>,
        synchronize: bool,
        expected: Option<bool>,
    ) -> Result<()> {
        let name = cog.name().to_owned();
        if name.is_empty() {
            return Err(YappinError::validation("cog.name", "must not be empty"));
        }
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
                    },
                );
            }
            for command in cog.registered_commands() {
                insert_registered(&mut commands, command, Some(name.clone()))?;
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
            planned.retain(|_, c| c.cog_name.as_deref() != Some(&name));
            planned.extend(commands.clone());
            let planned = advertised_commands(&planned)?;
            // Do not hold dispatch locks while waiting for the API. Old code stays available.
            if synchronize && self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst)
            {
                self.client().set_commands(&planned).await?;
            }
            // Acquire all routing locks before mutation; no await occurs during publication.
            let mut current = self.inner.commands.write().await;
            let mut routes = self.inner.buttons.write().await;
            let mut cogs = self.inner.cogs.write().await;
            current.retain(|_, c| c.cog_name.as_deref() != Some(&name));
            routes.retain(|b| b.cog_name.as_deref() != Some(&name));
            current.extend(commands);
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
    pub(crate) async fn install_worker<C: Cog>(&self, cog: C, replacing: bool) -> Result<()> {
        self.install_cog(Arc::new(cog), true, Some(replacing)).await
    }

    /// Remove routing atomically. Existing handler snapshots may finish in flight.
    pub async fn remove_cog(&self, name: &str) -> Result<Option<Arc<dyn Cog>>> {
        let old = {
            let _update = self.inner.registry_update.lock().await;
            if !self.inner.cogs.read().await.contains_key(name) {
                return Ok(None);
            }
            let mut planned = self.inner.commands.read().await.clone();
            planned.retain(|_, c| c.cog_name.as_deref() != Some(name));
            let planned = advertised_commands(&planned)?;
            if self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst) {
                self.client().set_commands(&planned).await?;
            }
            let mut commands = self.inner.commands.write().await;
            let mut buttons = self.inner.buttons.write().await;
            let mut cogs = self.inner.cogs.write().await;
            commands.retain(|_, c| c.cog_name.as_deref() != Some(name));
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
    pub async fn reload_cog<C: Cog>(&self, cog: C) -> Result<()> {
        self.install_cog(Arc::new(cog), true, None).await
    }

    /// Recreate a linked cog through its factory; this does not compile source files.
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
        self.install_cog(cog, true, None).await
    }

    /// Synchronize a consistent command snapshot without blocking normal dispatch.
    pub async fn sync_commands(&self) -> Result<Vec<Command>> {
        let _update = self.inner.registry_update.lock().await;
        let commands = advertised_commands(&*self.inner.commands.read().await)?;
        self.client().set_commands(&commands).await
    }

    /// Stops the bot's event loop.
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        self.inner.stopped.notify_one();
    }
}

/// Builder for configuring and starting a [`Bot`].
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
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
    cursor: Option<EventCursor>,
    max_age: Option<u32>,
    handler_concurrency: usize,
    pending_batches: usize,
    auto_ack_after: Option<Duration>,
    transport_mode: crate::diagnostics::TransportMode,
    lookup_policy: crate::cache::CachePolicy,
    #[cfg(feature = "gateway")]
    gateway_url: Option<String>,
}

impl BotBuilder {
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
            ignore_bots: true,
            sync_commands: true,
            log: true,
            cursor: None,
            max_age: None,
            handler_concurrency: 8,
            pending_batches: 4,
            auto_ack_after: None,
            transport_mode: crate::diagnostics::TransportMode::Polling,
            lookup_policy: crate::cache::CachePolicy::default(),
            #[cfg(feature = "gateway")]
            gateway_url: None,
        }
    }

    /// Sets the API base URL.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Adds a message command prefix (e.g. `!` or `yap!`).
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        let p = prefix.into();
        if !self.prefixes.contains(&p) {
            self.prefixes.push(p);
        }
        self
    }
    pub fn prefixes_with<F, Fut>(mut self, resolver: F) -> Self
    where
        F: Fn(MessageEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<String>>> + Send + 'static,
    {
        self.prefix_resolver = Some(Arc::new(move |event| Box::pin(resolver(event))));
        self
    }
    pub fn mention_prefix(mut self, enabled: bool) -> Self {
        self.mention_prefix = enabled;
        self
    }
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

    /// Registers a command with a name, description, and async handler.
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
            },
        );
        self
    }

    /// Registers a command with an automatic rate limit cooldown.
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
        let wrapped_handler: CommandHandlerFn = Arc::new(move |ctx| {
            let cd = cooldown.clone();
            let fut = handler(ctx.clone());
            Box::pin(async move {
                if let Some(remaining) = cd.check(&ctx).await {
                    let secs = remaining.as_secs_f32();
                    let _ = ctx
                        .reply(format!("⏳ You are on cooldown! Please wait {:.1}s.", secs))
                        .await;
                    return Ok(());
                }
                fut.await
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
            },
        );
        self
    }

    /// Registers a handler for button clicks whose `custom_id` starts with `prefix`.
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
    pub fn cog(mut self, cog: impl Cog) -> Self {
        self.cogs.push(Box::new(cog));
        self
    }

    /// Attaches a custom [`EventHandler`].
    pub fn event_handler(mut self, handler: impl EventHandler) -> Self {
        self.event_handler = Some(Arc::new(handler));
        self
    }

    /// Registers a custom error handler for commands that return an `Err`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// let bot = Bot::builder("yb_token")
    ///     .on_command_error(|ctx, err| async move {
    ///         let _ = ctx.reply(format!("⚠️ Command failed: {err}")).await;
    ///     });
    /// ```
    pub fn on_command_error<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(MessageContext, YappinError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.command_error_handler = Some(Arc::new(move |ctx, err| Box::pin(handler(ctx, err))));
        self
    }

    /// Registers a custom error handler for button interactions that return an `Err`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// let bot = Bot::builder("yb_token")
    ///     .on_button_error(|ctx, err| async move {
    ///         let _ = ctx.ephemeral(format!("⚠️ Button failed: {err}")).await;
    ///     });
    /// ```
    pub fn on_button_error<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(InteractionContext, YappinError) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.button_error_handler = Some(Arc::new(move |ctx, err| Box::pin(handler(ctx, err))));
        self
    }

    /// Whether to ignore messages authored by other bots (default `true`).
    pub fn ignore_bots(mut self, ignore: bool) -> Self {
        self.ignore_bots = ignore;
        self
    }

    /// Whether to automatically sync registered commands to Yapin' on startup (default `true`).
    pub fn sync_commands(mut self, sync: bool) -> Self {
        self.sync_commands = sync;
        self
    }

    /// Whether to log status messages to console (default `true`).
    pub fn log(mut self, enable_log: bool) -> Self {
        self.log = enable_log;
        self
    }

    /// Resume only after completed batches. A cursor file must have a single owner.
    pub fn cursor_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.cursor = Some(EventCursor::new(path));
        self
    }
    pub fn max_event_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }
    /// Maximum concurrent events. Use 1 for strict event ordering.
    pub fn handler_concurrency(mut self, limit: usize) -> Self {
        self.handler_concurrency = limit.clamp(1, 100);
        self
    }
    /// Bounds both prefetched and active event batches (each API batch has at most 100 events).
    pub fn pending_batches(mut self, limit: usize) -> Self {
        self.pending_batches = limit.clamp(1, 32);
        self
    }
    pub fn transport_mode(mut self, mode: crate::diagnostics::TransportMode) -> Self {
        self.transport_mode = mode;
        self
    }
    pub fn lookup_cache(mut self, policy: crate::cache::CachePolicy) -> Self {
        self.lookup_policy = policy;
        self
    }
    /// Opt-in early acknowledgement. Use a short delay (e.g. 250ms); already answered clicks are skipped.
    pub fn auto_ack_after(mut self, after: Duration) -> Self {
        self.auto_ack_after = Some(after);
        self
    }
    /// Gateway automatically resumes and falls back to polling when unavailable.
    #[cfg(feature = "gateway")]
    pub fn gateway(mut self, url: impl Into<String>) -> Self {
        self.gateway_url = Some(url.into());
        self.transport_mode = crate::diagnostics::TransportMode::GatewayPreferred;
        self
    }
    #[cfg(feature = "gateway")]
    pub fn gateway_only(self, url: impl Into<String>) -> Self {
        self.gateway(url)
            .transport_mode(crate::diagnostics::TransportMode::GatewayOnly)
    }

    /// Builds the configured [`Bot`].
    pub fn build(self) -> Bot {
        let client = Client::builder(self.token)
            .base_url(self.base_url)
            .lookup_cache(self.lookup_policy)
            .build();

        let state = Arc::new(BotState {
            client: Arc::new(client),
            prefixes: self.prefixes,
            prefix_resolver: self.prefix_resolver,
            mention_prefix: self.mention_prefix,
            definition_error: self.definition_error,
            commands: RwLock::new(self.commands),
            buttons: RwLock::new(self.buttons),
            cogs: RwLock::new(HashMap::new()),
            cog_factories: RwLock::new(HashMap::new()),
            event_handler: self.event_handler,
            command_error_handler: self.command_error_handler,
            button_error_handler: self.button_error_handler,
            ignore_bots: self.ignore_bots,
            sync_commands: self.sync_commands,
            log: self.log,
            cursor: self.cursor,
            max_age: self.max_age,
            handler_concurrency: self.handler_concurrency,
            pending_batches: self.pending_batches,
            auto_ack_after: self.auto_ack_after,
            telemetry: crate::diagnostics::Telemetry::new(self.transport_mode),
            #[cfg(feature = "gateway")]
            gateway_url: self.gateway_url,
            registry_update: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
            stopped: tokio::sync::Notify::new(),
        });

        Bot {
            handle: BotHandle { inner: state },
            initial_cogs: self.cogs,
        }
    }
}

/// The core bot runner that connects to Yapin', handles long-polling, and routes events.
pub struct Bot {
    handle: BotHandle,
    initial_cogs: Vec<Box<dyn Cog>>,
}

impl Bot {
    pub fn transport_status(&self) -> crate::diagnostics::TransportStatus {
        self.handle.transport_status()
    }
    pub fn events(&self) -> &crate::collectors::EventHub {
        self.client().events()
    }
    /// Creates a [`BotBuilder`] for the specified bot token.
    pub fn builder(token: impl Into<String>) -> BotBuilder {
        BotBuilder::new(token)
    }

    /// Returns a [`BotHandle`] for managing cogs and state at runtime.
    pub fn handle(&self) -> BotHandle {
        self.handle.clone()
    }

    /// Returns a reference to the underlying [`Client`].
    pub fn client(&self) -> &Client {
        self.handle.client()
    }

    /// Returns a reference to the bot's internal in-memory cache.
    pub fn cache(&self) -> &crate::cache::Cache {
        self.handle.cache()
    }

    /// Adds a cog to the bot. Can be called before or while the bot is running.
    pub async fn add_cog(&self, cog: impl Cog) -> Result<()> {
        self.handle.add_cog(cog).await
    }

    /// Removes a cog by name from the bot.
    pub async fn remove_cog(&self, name: &str) -> Result<Option<Arc<dyn Cog>>> {
        self.handle.remove_cog(name).await
    }

    /// Reloads a cog.
    pub async fn reload_cog(&self, cog: impl Cog) -> Result<()> {
        self.handle.reload_cog(cog).await
    }

    /// Registers a factory to instantiate a cog by name.
    pub async fn register_cog_factory<F>(&self, name: impl Into<String>, factory: F)
    where
        F: Fn() -> Box<dyn Cog> + Send + Sync + 'static,
    {
        self.handle.register_cog_factory(name, factory).await
    }

    /// Reloads a cog by name using its registered factory.
    pub async fn reload_cog_by_name(&self, name: &str) -> Result<()> {
        self.handle.reload_cog_by_name(name).await
    }

    /// Stops the bot long-polling loop gracefully.
    pub fn stop(&self) {
        self.handle.stop();
    }

    /// Runs the bot until stopped or Ctrl+C is received.
    pub async fn run(self) -> Result<()> {
        let handle = self.handle.clone();
        let result = self.run_inner().await;
        handle.inner.running.store(false, Ordering::SeqCst);
        result
    }

    async fn run_inner(mut self) -> Result<()> {
        if let Some(error) = &self.handle.inner.definition_error {
            return Err(YappinError::Config(error.clone()));
        }
        advertised_commands(&*self.handle.inner.commands.read().await)?;
        #[cfg(not(feature = "gateway"))]
        if self.transport_status().mode != crate::diagnostics::TransportMode::Polling {
            return Err(YappinError::Config(
                "enable the gateway Cargo feature for this transport mode".into(),
            ));
        }
        self.handle.inner.running.store(true, Ordering::SeqCst);

        // 1. Fetch bot user details to verify token
        let me: BotProfile = match self.handle.client().get_me().await {
            Ok(profile) => profile,
            Err(e) => {
                if let YappinError::Unauthorized = e {
                    eprintln!("Token rejected: generate a new one in Settings -> Developer");
                }
                return Err(e);
            }
        };

        // Initial cogs use the same validated, atomic registration path.
        for cog in self.initial_cogs.drain(..) {
            self.handle.install_cog(Arc::from(cog), false, None).await?;
        }

        // 3. Register slash commands if enabled
        if self.handle.inner.sync_commands {
            if let Err(e) = self.handle.sync_commands().await {
                warn!("Failed to synchronize commands: {}", e);
            }
        }

        let saved = match &self.handle.inner.cursor {
            Some(c) => c.load().await?,
            None => None,
        };
        let last = match saved {
            Some(id) => id,
            None if self.transport_status().mode
                == crate::diagnostics::TransportMode::GatewayOnly =>
            {
                0
            }
            None => self.client().get_events(Some(0), Some(0), None).await?.last,
        };
        if let Some(c) = &self.handle.inner.cursor {
            c.save(last).await?;
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
    pub async fn process_event(&self, event: crate::models::Event) -> Result<()> {
        if let Some(error) = &self.handle.inner.definition_error {
            return Err(YappinError::Config(error.clone()));
        }
        let me = match self.cache().get_me().await {
            Some(me) => me,
            None => self.client().get_me().await?,
        };
        self.client().receive_event(event.clone()).await;
        if let Some(after) = self.handle.inner.auto_ack_after {
            if let Ok(EventData::Interaction(mut interaction)) = event.parse_data() {
                interaction.created_at = interaction.created_at.or(event.created_at);
                self.client().auto_ack(interaction, after);
            }
        }
        self.dispatch_event(event, &me).await
    }

    async fn dispatch_event(&self, event: crate::models::Event, me: &BotProfile) -> Result<()> {
        let parsed = match event.parse_data() {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to deserialize event {}: {}", event.kind, e);
                return Err(e.into());
            }
        };

        match parsed {
            EventData::Message(msg) => self.handle_message(msg, me).await?,
            EventData::Interaction(inter) => self.handle_interaction(inter).await?,
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
            }
            EventData::ReactionRemove(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_reaction_remove(self.handle.client(), &ev).await;
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
            EventData::Unknown { kind, data } => {
                debug!("Received unhandled event type: {}", kind);
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_unknown_event(self.handle.client(), &kind, &data).await;
                }
            }
        }
        Ok(())
    }

    async fn handle_message(&self, msg: MessageEvent, me: &BotProfile) -> Result<()> {
        if self.handle.inner.ignore_bots && msg.author_bot == Some(true) {
            return Ok(());
        }

        // If command_for list is provided, verify whether our bot was targeted
        if let Some(ref targeted) = msg.command_for {
            if !targeted.contains(&me.id) {
                return Ok(());
            }
        }

        let mut matched_command: Option<String> = None;
        let mut matched_args = String::new();

        // Check if message comes pre-parsed as a slash command
        if let Some(ref cmd_name) = msg.command {
            matched_command = Some(cmd_name.to_lowercase());
            matched_args = msg.args.clone().unwrap_or_default();
        } else if let Some(ref content) = msg.content {
            // Check prefix commands
            let mut prefixes = if let Some(resolver) = &self.handle.inner.prefix_resolver {
                resolver(msg.clone()).await?
            } else {
                self.handle.inner.prefixes.clone()
            };
            if self.handle.inner.mention_prefix {
                prefixes.push(format!("<@{}>", me.id));
                prefixes.push(format!("<@!{}>", me.id));
            }
            prefixes.retain(|p| !p.is_empty());
            prefixes.sort_by_key(|p| std::cmp::Reverse(p.len()));

            let cmds_guard = self.handle.inner.commands.read().await;
            for p in prefixes {
                if content.starts_with(&p) {
                    let rest = content[p.len()..].trim_start();
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    if let Some(cmd_name) = parts.next() {
                        let cmd_lower = cmd_name.to_lowercase();
                        if cmds_guard.keys().any(|key| {
                            key == &cmd_lower || key.starts_with(&format!("{cmd_lower} "))
                        }) {
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
            let commands = self.handle.inner.commands.read().await;
            let tokens = crate::commands::tokenize(&matched_args).unwrap_or_default();
            let mut path = root.clone();
            let mut best = commands.contains_key(&path).then_some((path.clone(), 0));
            for token in tokens.iter().take(7) {
                path.push(' ');
                path.push_str(&token.value.to_lowercase());
                if commands.contains_key(&path) {
                    best = Some((path.clone(), token.end));
                }
            }
            if let Some((path, end)) = best {
                matched_command = Some(path);
                matched_args = matched_args[end..].trim_start().to_owned();
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
            let cmds_guard = self.handle.inner.commands.read().await;
            if let Some(def) = cmds_guard.get(&cmd_name) {
                let handler = def.handler.clone();
                let usage = def.info.as_ref().map(|info| info.usage());
                let cog_name = def.cog_name.clone();
                let cog = match cog_name {
                    Some(name) => self.handle.inner.cogs.read().await.get(&name).cloned(),
                    None => None,
                };
                drop(cmds_guard);
                let allow = match cog {
                    Some(cog) => cog.check(&ctx).await,
                    None => true,
                };
                if allow {
                    if let Err(e) = handler(ctx.clone()).await {
                        if matches!(e, YappinError::Command(_)) {
                            let text = format!(
                                "{}{}",
                                e,
                                usage.map(|u| format!("\nUsage: {u}")).unwrap_or_default()
                            );
                            for part in crate::split_message(&text) {
                                ctx.reply(part).await?;
                            }
                            return Ok(());
                        }
                        let message = e.to_string();
                        if let Some(on_err) = &self.handle.inner.command_error_handler {
                            on_err(ctx.clone(), e).await;
                        } else {
                            error!("Command /{cmd_name} failed: {message}");
                        }
                        return Err(YappinError::Config(format!(
                            "command /{cmd_name} failed: {message}"
                        )));
                    }
                }
            }
        }
        if let Some(h) = &self.handle.inner.event_handler {
            h.on_message(&ctx).await;
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
            cog.on_message(&ctx).await;
        }
        Ok(())
    }

    async fn handle_interaction(&self, inter: InteractionEvent) -> Result<()> {
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
            .map(|b| b.handler.clone())
            .collect();
        for handler in handlers {
            if let Err(e) = handler(ctx.clone()).await {
                let message = e.to_string();
                if let Some(on_err) = &self.handle.inner.button_error_handler {
                    on_err(ctx.clone(), e).await;
                } else {
                    error!("Button handler failed: {message}");
                }
                return Err(YappinError::Config(format!(
                    "button handler failed: {message}"
                )));
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
