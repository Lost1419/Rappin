pub mod cog;
pub mod context;
pub mod handler;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::sleep;
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

struct CommandDef {
    description: String,
    cog_name: Option<String>,
    handler: CommandHandlerFn,
}

struct ButtonDef {
    prefix: String,
    cog_name: Option<String>,
    handler: ButtonHandlerFn,
}

type CogFactoryFn = Arc<dyn Fn() -> Box<dyn Cog> + Send + Sync>;

struct BotState {
    client: Arc<Client>,
    prefixes: Vec<String>,
    commands: RwLock<HashMap<String, CommandDef>>,
    buttons: RwLock<Vec<ButtonDef>>,
    cogs: RwLock<HashMap<String, Arc<dyn Cog>>>,
    cog_factories: RwLock<HashMap<String, CogFactoryFn>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
    running: AtomicBool,
}

/// A cloneable handle to control the bot and dynamically manage cogs while running.
#[derive(Clone)]
pub struct BotHandle {
    inner: Arc<BotState>,
}

impl BotHandle {
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

    /// Dynamically loads a new cog into the bot while it's running.
    ///
    /// The cog's `on_load()` is called, its commands and buttons are registered,
    /// and `sync_commands()` is automatically called to update Yapin's slash command menu.
    pub async fn add_cog<C: Cog>(&self, cog: C) -> Result<()> {
        let name = cog.name().to_string();

        // If cog already exists, unload it first
        self.remove_cog(&name).await?;

        // 1. Run on_load
        cog.on_load(&self.inner.client).await?;

        let cog_commands = cog.commands();
        let cog_buttons = cog.buttons();
        let cog_arc: Arc<dyn Cog> = Arc::new(cog);

        // 2. Register cog commands
        {
            let mut cmds = self.inner.commands.write().await;
            for c in cog_commands {
                cmds.insert(
                    c.name,
                    CommandDef {
                        description: c.description,
                        cog_name: Some(name.clone()),
                        handler: c.handler,
                    },
                );
            }
        }

        // 3. Register cog buttons
        {
            let mut btns = self.inner.buttons.write().await;
            for b in cog_buttons {
                btns.push(ButtonDef {
                    prefix: b.prefix,
                    cog_name: Some(name.clone()),
                    handler: b.handler,
                });
            }
        }

        // 4. Save cog instance
        self.inner.cogs.write().await.insert(name.clone(), cog_arc);

        if self.inner.log {
            info!("Cog '{}' loaded", name);
        }

        // 5. Sync updated command list to Yapin' if running
        if self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst) {
            self.sync_commands().await?;
        }

        Ok(())
    }

    /// Dynamically unloads a cog by name while the bot is running.
    ///
    /// The cog's `on_unload()` is called, its commands and buttons are removed,
    /// and `sync_commands()` updates Yapin's live slash commands.
    pub async fn remove_cog(&self, name: &str) -> Result<Option<Arc<dyn Cog>>> {
        let cog_opt = self.inner.cogs.write().await.remove(name);

        if let Some(ref cog) = cog_opt {
            // 1. Run on_unload
            cog.on_unload(&self.inner.client).await?;

            // 2. Remove commands
            {
                let mut cmds = self.inner.commands.write().await;
                cmds.retain(|_, def| def.cog_name.as_deref() != Some(name));
            }

            // 3. Remove buttons
            {
                let mut btns = self.inner.buttons.write().await;
                btns.retain(|b| b.cog_name.as_deref() != Some(name));
            }

            if self.inner.log {
                info!("Cog '{}' unloaded", name);
            }

            // 4. Sync commands to Yapin' if running
            if self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst) {
                self.sync_commands().await?;
            }
        }

        Ok(cog_opt)
    }

    /// Reloads an existing cog instance with a new instance of the cog.
    pub async fn reload_cog<C: Cog>(&self, cog: C) -> Result<()> {
        let name = cog.name().to_string();
        self.remove_cog(&name).await?;
        self.add_cog(cog).await?;
        Ok(())
    }

    /// Reloads a cog by name using its registered cog factory.
    pub async fn reload_cog_by_name(&self, name: &str) -> Result<()> {
        let factory = {
            let factories = self.inner.cog_factories.read().await;
            factories.get(name).cloned().ok_or_else(|| {
                YappinError::Config(format!("No factory registered for cog '{}'", name))
            })?
        };

        let new_cog = factory();
        self.remove_cog(name).await?;

        // Load new cog
        new_cog.on_load(&self.inner.client).await?;
        let cog_commands = new_cog.commands();
        let cog_buttons = new_cog.buttons();
        let cog_arc: Arc<dyn Cog> = Arc::from(new_cog);

        {
            let mut cmds = self.inner.commands.write().await;
            for c in cog_commands {
                cmds.insert(
                    c.name,
                    CommandDef {
                        description: c.description,
                        cog_name: Some(name.to_string()),
                        handler: c.handler,
                    },
                );
            }
        }

        {
            let mut btns = self.inner.buttons.write().await;
            for b in cog_buttons {
                btns.push(ButtonDef {
                    prefix: b.prefix,
                    cog_name: Some(name.to_string()),
                    handler: b.handler,
                });
            }
        }

        self.inner
            .cogs
            .write()
            .await
            .insert(name.to_string(), cog_arc);

        if self.inner.log {
            info!("Cog '{}' reloaded", name);
        }

        if self.inner.sync_commands && self.inner.running.load(Ordering::SeqCst) {
            self.sync_commands().await?;
        }

        Ok(())
    }

    /// Synchronizes registered commands to Yapin' via `PUT /commands`.
    pub async fn sync_commands(&self) -> Result<Vec<Command>> {
        let cmds_snapshot = self.inner.commands.read().await;
        let cmd_list: Vec<Command> = cmds_snapshot
            .iter()
            .map(|(name, def)| Command::new(name, &def.description))
            .collect();
        drop(cmds_snapshot);

        let synced = self.inner.client.set_commands(&cmd_list).await?;
        if self.inner.log {
            info!("Synchronized {} command(s) with Yapin'", synced.len());
        }
        Ok(synced)
    }

    /// Stops the bot's event loop.
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
    }
}

/// Builder for configuring and starting a [`Bot`].
pub struct BotBuilder {
    token: String,
    base_url: String,
    prefixes: Vec<String>,
    commands: HashMap<String, CommandDef>,
    buttons: Vec<ButtonDef>,
    cogs: Vec<Box<dyn Cog>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
}

impl BotBuilder {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base_url: DEFAULT_API_BASE.to_string(),
            prefixes: vec!["/".to_string(), "!".to_string()],
            commands: HashMap::new(),
            buttons: Vec::new(),
            cogs: Vec::new(),
            event_handler: None,
            ignore_bots: true,
            sync_commands: true,
            log: true,
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
        self.commands.insert(
            name_str,
            CommandDef {
                description: desc_str,
                cog_name: None,
                handler: wrapped_handler,
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
        self.commands.insert(
            name_str,
            CommandDef {
                description: desc_str,
                cog_name: None,
                handler: wrapped_handler,
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

    /// Builds the configured [`Bot`].
    pub fn build(self) -> Bot {
        let client = Client::builder(self.token).base_url(self.base_url).build();

        let state = Arc::new(BotState {
            client: Arc::new(client),
            prefixes: self.prefixes,
            commands: RwLock::new(self.commands),
            buttons: RwLock::new(self.buttons),
            cogs: RwLock::new(HashMap::new()),
            cog_factories: RwLock::new(HashMap::new()),
            event_handler: self.event_handler,
            ignore_bots: self.ignore_bots,
            sync_commands: self.sync_commands,
            log: self.log,
            running: AtomicBool::new(false),
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
    pub async fn run(mut self) -> Result<()> {
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

        // 2. Load any initial cogs
        for cog in self.initial_cogs.drain(..) {
            let name = cog.name().to_string();
            cog.on_load(self.handle.client()).await?;

            let cog_commands = cog.commands();
            let cog_buttons = cog.buttons();
            let cog_arc: Arc<dyn Cog> = Arc::from(cog);

            {
                let mut cmds = self.handle.inner.commands.write().await;
                for c in cog_commands {
                    cmds.insert(
                        c.name,
                        CommandDef {
                            description: c.description,
                            cog_name: Some(name.clone()),
                            handler: c.handler,
                        },
                    );
                }
            }

            {
                let mut btns = self.handle.inner.buttons.write().await;
                for b in cog_buttons {
                    btns.push(ButtonDef {
                        prefix: b.prefix,
                        cog_name: Some(name.clone()),
                        handler: b.handler,
                    });
                }
            }

            self.handle
                .inner
                .cogs
                .write()
                .await
                .insert(name.clone(), cog_arc);

            if self.handle.inner.log {
                info!("Cog '{}' loaded", name);
            }
        }

        // 3. Register slash commands if enabled
        if self.handle.inner.sync_commands {
            if let Err(e) = self.handle.sync_commands().await {
                warn!("Failed to synchronize commands: {}", e);
            }
        }

        // 4. Obtain initial event offset (after=0 returns no events, newest id as last)
        let mut last: u64 = match self
            .handle
            .client()
            .get_events(Some(0), Some(0), None)
            .await
        {
            Ok(resp) => resp.last,
            Err(e) => {
                warn!("Could not fetch initial event offset, starting at 0: {}", e);
                0
            }
        };

        if self.handle.inner.log {
            println!("[rappin] Logged in as {} ({})", me.username, me.id);
        }

        // Emit ready to event handler
        if let Some(ref handler) = self.handle.inner.event_handler {
            handler.on_ready(self.handle.client(), &me).await;
        }

        let mut fail_count = 0u32;

        while self.handle.inner.running.load(Ordering::SeqCst) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    if self.handle.inner.log {
                        println!("\n[rappin] Shutting down...");
                    }
                    self.stop();
                    break;
                }
                poll_result = self.handle.client().get_events(Some(last), Some(25), None) => {
                    match poll_result {
                        Ok(resp) => {
                            fail_count = 0;
                            for event in resp.events {
                                if event.id <= last {
                                    continue;
                                }
                                last = event.id;
                                self.dispatch_event(event, &me).await;
                            }
                            last = last.max(resp.last);
                        }
                        Err(YappinError::Unauthorized) => {
                            eprintln!("Token rejected: generate a new one in Settings -> Developer");
                            return Err(YappinError::Unauthorized);
                        }
                        Err(e) => {
                            fail_count += 1;
                            let backoff_secs = (1 << fail_count.min(5)).min(30);
                            warn!("Event poll error (attempt {}): {}. Retrying in {}s...", fail_count, e, backoff_secs);
                            sleep(Duration::from_secs(backoff_secs)).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn dispatch_event(&self, event: crate::models::Event, me: &BotProfile) {
        let parsed = match event.parse_data() {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to deserialize event {}: {}", event.kind, e);
                return;
            }
        };

        match parsed {
            EventData::Message(msg) => self.handle_message(msg, me).await,
            EventData::Interaction(inter) => self.handle_interaction(inter).await,
            EventData::MemberJoin(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_member_join(self.handle.client(), &ev).await;
                }
                let cogs = self.handle.inner.cogs.read().await;
                for cog in cogs.values() {
                    cog.on_member_join(self.handle.client(), &ev).await;
                }
            }
            EventData::MemberLeave(ev) => {
                if let Some(ref h) = self.handle.inner.event_handler {
                    h.on_member_leave(self.handle.client(), &ev).await;
                }
                let cogs = self.handle.inner.cogs.read().await;
                for cog in cogs.values() {
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
    }

    async fn handle_message(&self, msg: MessageEvent, me: &BotProfile) {
        if self.handle.inner.ignore_bots && msg.author_bot == Some(true) {
            return;
        }

        // If command_for list is provided, verify whether our bot was targeted
        if let Some(ref targeted) = msg.command_for {
            if !targeted.contains(&me.id) {
                return;
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
            let mut prefixes = self.handle.inner.prefixes.clone();
            prefixes.sort_by_key(|p| std::cmp::Reverse(p.len()));

            let cmds_guard = self.handle.inner.commands.read().await;
            for p in prefixes {
                if content.starts_with(&p) {
                    let rest = &content[p.len()..];
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    if let Some(cmd_name) = parts.next() {
                        let cmd_lower = cmd_name.to_lowercase();
                        if cmds_guard.contains_key(&cmd_lower) {
                            matched_command = Some(cmd_lower);
                            matched_args = parts.next().unwrap_or_default().trim().to_string();
                            break;
                        }
                    }
                }
            }
        }

        let ctx = MessageContext::new(
            self.handle.inner.client.clone(),
            msg,
            matched_command.clone(),
            matched_args,
            Some(self.handle.clone()),
        );

        if let Some(cmd_name) = matched_command {
            let cmds_guard = self.handle.inner.commands.read().await;
            if let Some(def) = cmds_guard.get(&cmd_name) {
                let handler = def.handler.clone();
                let cog_name = def.cog_name.clone();
                drop(cmds_guard);

                // Run cog check if belongs to a cog
                let mut allow = true;
                if let Some(ref cname) = cog_name {
                    let cogs_guard = self.handle.inner.cogs.read().await;
                    if let Some(cog) = cogs_guard.get(cname) {
                        allow = cog.check(&ctx).await;
                    }
                }

                if allow {
                    let cmd_ctx = ctx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler(cmd_ctx).await {
                            error!("Command /{} error: {}", cmd_name, e);
                        }
                    });
                }
            }
        }

        // Notify event handler and cogs
        if let Some(ref h) = self.handle.inner.event_handler {
            h.on_message(&ctx).await;
        }
        let cogs = self.handle.inner.cogs.read().await;
        for cog in cogs.values() {
            cog.on_message(&ctx).await;
        }
    }

    async fn handle_interaction(&self, inter: InteractionEvent) {
        let custom_id = inter.custom_id.clone().unwrap_or_default();
        let ctx = InteractionContext::new(
            self.handle.inner.client.clone(),
            inter,
            Some(self.handle.clone()),
        );

        let btns_guard = self.handle.inner.buttons.read().await;
        for btn in btns_guard.iter() {
            if custom_id.starts_with(&btn.prefix) {
                let handler = btn.handler.clone();
                let btn_ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handler(btn_ctx).await {
                        error!("Button interaction error: {}", e);
                    }
                });
            }
        }
        drop(btns_guard);

        if let Some(ref h) = self.handle.inner.event_handler {
            h.on_interaction(&ctx).await;
        }
        let cogs = self.handle.inner.cogs.read().await;
        for cog in cogs.values() {
            cog.on_interaction(&ctx).await;
        }
    }
}
