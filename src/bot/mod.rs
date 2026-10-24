pub mod context;
pub mod handler;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use tracing::{debug, error, info, warn};

pub use context::{InteractionContext, MessageContext};
pub use handler::{EventHandler, NoopEventHandler};

use crate::client::Client;
use crate::error::{Result, YappinError};
use crate::models::{
    BotProfile, Command, EventData, InteractionEvent, MessageEvent,
};
use crate::utils::DEFAULT_API_BASE;

type CommandHandlerFn = Arc<
    dyn Fn(MessageContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;
type ButtonHandlerFn = Arc<
    dyn Fn(InteractionContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

struct CommandDef {
    description: String,
    handler: CommandHandlerFn,
}

struct ButtonDef {
    prefix: String,
    handler: ButtonHandlerFn,
}

/// Builder for configuring and starting a [`Bot`].
pub struct BotBuilder {
    token: String,
    base_url: String,
    prefixes: Vec<String>,
    commands: HashMap<String, CommandDef>,
    buttons: Vec<ButtonDef>,
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
    pub fn command<F, Fut>(mut self, name: impl Into<String>, description: impl Into<String>, handler: F) -> Self
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
            handler: wrapped,
        });
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
        let client = Client::builder(self.token)
            .base_url(self.base_url)
            .build();

        Bot {
            client: Arc::new(client),
            prefixes: self.prefixes,
            commands: Arc::new(self.commands),
            buttons: Arc::new(self.buttons),
            event_handler: self.event_handler,
            ignore_bots: self.ignore_bots,
            sync_commands: self.sync_commands,
            log: self.log,
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// The core bot runner that connects to Yapin', handles long-polling, and routes events.
pub struct Bot {
    client: Arc<Client>,
    prefixes: Vec<String>,
    commands: Arc<HashMap<String, CommandDef>>,
    buttons: Arc<Vec<ButtonDef>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    ignore_bots: bool,
    sync_commands: bool,
    log: bool,
    running: Arc<AtomicBool>,
}

impl Bot {
    /// Creates a [`BotBuilder`] for the specified bot token.
    pub fn builder(token: impl Into<String>) -> BotBuilder {
        BotBuilder::new(token)
    }

    /// Returns a reference to the underlying [`Client`].
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Stops the bot long-polling loop gracefully.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Runs the bot until stopped or Ctrl+C is received.
    pub async fn run(&self) -> Result<()> {
        self.running.store(true, Ordering::SeqCst);

        // 1. Fetch bot user details to verify token
        let me: BotProfile = match self.client.get_me().await {
            Ok(profile) => profile,
            Err(e) => {
                if let YappinError::Unauthorized = e {
                    eprintln!("Token rejected: generate a new one in Settings -> Developer");
                }
                return Err(e);
            }
        };

        // 2. Register slash commands if enabled
        if self.sync_commands && !self.commands.is_empty() {
            let cmd_list: Vec<Command> = self
                .commands
                .iter()
                .map(|(name, def)| Command::new(name, &def.description))
                .collect();

            if let Err(e) = self.client.set_commands(&cmd_list).await {
                warn!("Failed to synchronize commands: {}", e);
            } else if self.log {
                info!("Registered {} slash command(s)", cmd_list.len());
            }
        }

        // 3. Obtain initial event offset (after=0 returns no events, newest id as last)
        let mut last: u64 = match self.client.get_events(Some(0), Some(0), None).await {
            Ok(resp) => resp.last,
            Err(e) => {
                warn!("Could not fetch initial event offset, starting at 0: {}", e);
                0
            }
        };

        if self.log {
            println!(
                "[yappin] Logged in as {} ({})",
                me.username, me.id
            );
        }

        // Emit ready to event handler
        if let Some(ref handler) = self.event_handler {
            handler.on_ready(&self.client, &me).await;
        }

        let mut fail_count = 0u32;

        while self.running.load(Ordering::SeqCst) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    if self.log {
                        println!("\n[yappin] Shutting down...");
                    }
                    self.stop();
                    break;
                }
                poll_result = self.client.get_events(Some(last), Some(25), None) => {
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
                if let Some(ref h) = self.event_handler {
                    h.on_member_join(&self.client, &ev).await;
                }
            }
            EventData::MemberLeave(ev) => {
                if let Some(ref h) = self.event_handler {
                    h.on_member_leave(&self.client, &ev).await;
                }
            }
            EventData::ReactionAdd(ev) => {
                if let Some(ref h) = self.event_handler {
                    h.on_reaction_add(&self.client, &ev).await;
                }
            }
            EventData::ReactionRemove(ev) => {
                if let Some(ref h) = self.event_handler {
                    h.on_reaction_remove(&self.client, &ev).await;
                }
            }
            EventData::VoiceJoin(ev) => {
                if let Some(ref h) = self.event_handler {
                    h.on_voice_join(&self.client, &ev).await;
                }
            }
            EventData::VoiceLeave(ev) => {
                if let Some(ref h) = self.event_handler {
                    h.on_voice_leave(&self.client, &ev).await;
                }
            }
            EventData::Test(val) => {
                if let Some(ref h) = self.event_handler {
                    h.on_test(&self.client, &val).await;
                }
            }
            EventData::Unknown { kind, data: _ } => {
                debug!("Received unhandled event type: {}", kind);
            }
        }
    }

    async fn handle_message(&self, msg: MessageEvent, me: &BotProfile) {
        if self.ignore_bots && msg.author_bot == Some(true) {
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
            let mut prefixes = self.prefixes.clone();
            prefixes.sort_by_key(|p| std::cmp::Reverse(p.len()));

            for p in prefixes {
                if content.starts_with(&p) {
                    let rest = &content[p.len()..];
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    if let Some(cmd_name) = parts.next() {
                        let cmd_lower = cmd_name.to_lowercase();
                        if self.commands.contains_key(&cmd_lower) {
                            matched_command = Some(cmd_lower);
                            matched_args = parts.next().unwrap_or_default().trim().to_string();
                            break;
                        }
                    }
                }
            }
        }

        let ctx = MessageContext::new(
            self.client.clone(),
            msg,
            matched_command.clone(),
            matched_args,
        );

        if let Some(cmd_name) = matched_command {
            if let Some(def) = self.commands.get(&cmd_name) {
                let handler = def.handler.clone();
                let cmd_ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handler(cmd_ctx).await {
                        error!("Command /{} error: {}", cmd_name, e);
                    }
                });
            }
        }

        if let Some(ref h) = self.event_handler {
            h.on_message(&ctx).await;
        }
    }

    async fn handle_interaction(&self, inter: InteractionEvent) {
        let custom_id = inter.custom_id.clone().unwrap_or_default();
        let ctx = InteractionContext::new(self.client.clone(), inter);

        for btn in self.buttons.iter() {
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

        if let Some(ref h) = self.event_handler {
            h.on_interaction(&ctx).await;
        }
    }
}
