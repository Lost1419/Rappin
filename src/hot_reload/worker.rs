use super::protocol::{self, CogEvent, Listener, Manifest, Wire};
use crate::{Command, CreateMessage, InteractionCallback, Message, Result, YappinError};
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{
    io::{BufReader, Stdin, Stdout},
    sync::Mutex,
};

type Handler = Arc<
    dyn Fn(WorkerContext) -> Pin<Box<dyn Future<Output = Result<crate::MessageDisposition>> + Send>>
        + Send
        + Sync,
>;
struct Connection {
    input: BufReader<Stdin>,
    output: Stdout,
    poisoned: bool,
    allow_single_attempt: bool,
    allow_ui: bool,
}

/// Rust handler context inside a worker. Authenticated JSON API requests go through the parent.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::hot_reload::WorkerCog;
/// let worker = WorkerCog::new("utilities")
///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await })
///     .button("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
/// // Reserve stdout for IPC. Call run only in the executable started by the parent.
/// ```
pub struct WorkerContext {
    /// Original parent-supplied handler input; operation identity is separate metadata, not a replacement message ID.
    pub event: CogEvent,
    connection: Arc<Mutex<Connection>>,
    active: Arc<AtomicBool>,
    operation_id: Option<crate::OperationId>,
}
impl WorkerContext {
    /// Consumer operation key supplied by the parent, separate from CogEvent identity.
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerContext;
    /// fn inspect(ctx: &WorkerContext) {
    ///     let journal_key = ctx.operation_id();
    ///     eprintln!("Operation: {journal_key:?}");
    /// }
    /// ```
    pub fn operation_id(&self) -> Option<&crate::OperationId> {
        self.operation_id.as_ref()
    }

    /// Parse and finish a FromArgs argument cursor; invalid/extra tokens return typed command errors.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// fn no_arguments(ctx: &WorkerContext) -> Result<()> { ctx.parse::<()>() }
    /// ```
    pub fn parse<A: crate::commands::FromArgs>(&self) -> Result<A> {
        let mut args = crate::commands::Arguments::new(self.args())?;
        let value = A::parse(&mut args)?;
        args.finish()?;
        Ok(value)
    }
    async fn wait_request(
        &self,
        filter: crate::collectors::EventFilter,
        timeout: std::time::Duration,
        prompt: Option<CreateMessage>,
    ) -> Result<crate::Event> {
        self.ensure_active()?;
        let mut connection = self.connection.lock().await;
        self.ensure_active()?;
        if connection.poisoned {
            return Err(protocol::protocol_error());
        }
        let timeout_ms = u64::try_from(timeout.as_millis())
            .map_err(|_| YappinError::validation("timeout", "timeout is too large"))?;
        connection.poisoned = true;
        protocol::write(
            &mut connection.output,
            &Wire::Wait {
                filter,
                timeout_ms,
                prompt,
                channel: self.channel_id(),
            },
        )
        .await?;
        match protocol::read(&mut connection.input).await? {
            Wire::EventResult {
                event,
                error,
                structured_error,
            } => {
                connection.poisoned = false;
                if let Some(error) = structured_error {
                    return Err(error.into());
                }
                if let Some(error) = error {
                    return Err(YappinError::Config(error));
                }
                event.map(|e| *e).ok_or_else(protocol::protocol_error)
            }
            _ => Err(protocol::protocol_error()),
        }
    }
    /// Register a bounded parent-owned event waiter; timeout/loss remain structured errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, collectors::EventFilter, hot_reload::WorkerContext};
    /// async fn wait(ctx: &WorkerContext, filter: EventFilter) -> Result<()> {
    ///     let event = ctx.wait_for_event(filter, std::time::Duration::from_secs(10)).await?;
    ///     eprintln!("Collected {}", event.id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn wait_for_event(
        &self,
        filter: crate::collectors::EventFilter,
        timeout: std::time::Duration,
    ) -> Result<crate::Event> {
        self.wait_request(filter, timeout, None).await
    }
    /// The parent registers the waiter before posting the prompt, preventing fast-answer races.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn ask(ctx: &WorkerContext) -> Result<()> {
    ///     let answer = ctx.prompt_for_reply("Your choice?", std::time::Duration::from_secs(30)).await?;
    ///     eprintln!("Collected message {}", answer.id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn prompt_for_reply(
        &self,
        prompt: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<crate::MessageEvent> {
        let channel = self
            .channel_id()
            .ok_or_else(|| YappinError::Config("event has no channel".into()))?;
        let filter = crate::collectors::EventFilter {
            kind: Some(
                if matches!(self.event, CogEvent::DmMessage { .. }) {
                    "dm_message"
                } else {
                    "message"
                }
                .into(),
            ),
            channel_id: Some(channel),
            author_id: Some(
                self.author_id()
                    .ok_or_else(|| YappinError::Config("event has no author".into()))?,
            ),
            ..Default::default()
        };
        let message = if let Some(event) = self.event.message() {
            CreateMessage::reply(prompt, event.id)
        } else {
            CreateMessage::new(prompt)
        };
        let event = self.wait_request(filter, timeout, Some(message)).await?;
        serde_json::from_value(event.data).map_err(Into::into)
    }
    pub(crate) async fn require_ui_api(&self) -> Result<()> {
        self.ensure_active()?;
        if self.connection.lock().await.allow_ui {
            Ok(())
        } else {
            Err(YappinError::validation("ui_api","worker UI callbacks require WorkerCog::ui_api() or on_ui_interaction and a protocol v7 parent"))
        }
    }
    /// Borrow extended select/modal/command values for a UI listener event.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerContext;
    /// fn inspect(ctx: &WorkerContext) {
    ///     if let Some(event) = ctx.ui_interaction() { eprintln!("UI kind: {:?}", event.interaction.kind); }
    /// }
    /// ```
    pub fn ui_interaction(&self) -> Option<&crate::UiInteraction> {
        match &self.event {
            CogEvent::UiInteraction { event } => Some(event),
            _ => None,
        }
    }
    /// Borrow explicit raw command arguments; other event kinds may supply no arguments.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerContext;
    /// fn inspect(ctx: &WorkerContext) {
    ///     let text = ctx.args();
    ///     eprintln!("Argument bytes: {}", text.len());
    /// }
    /// ```
    pub fn args(&self) -> &str {
        self.event.args()
    }
    /// Borrow parent-supplied command tokens, empty for non-command events.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerContext;
    /// fn inspect(ctx: &WorkerContext) {
    ///     let tokens = ctx.argv();
    ///     eprintln!("Tokens: {}", tokens.len());
    /// }
    /// ```
    pub fn argv(&self) -> &[String] {
        self.event.argv()
    }
    /// Original message/reaction/interaction actor, when available.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerContext;
    /// fn inspect(ctx: &WorkerContext) {
    ///     let requester = ctx.author_id();
    /// }
    /// ```
    pub fn author_id(&self) -> Option<uuid::Uuid> {
        self.event.author_id()
    }
    /// Original event channel, when available.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerContext;
    /// fn inspect(ctx: &WorkerContext) {
    ///     let channel = ctx.channel_id();
    /// }
    /// ```
    pub fn channel_id(&self) -> Option<uuid::Uuid> {
        self.event.channel_id()
    }
    /// JSON-only API access; paths are checked by the parent Client. No bot token is exposed.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.api_once("GET", "/me", None).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn api_once(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        self.ensure_active()?;
        if !self.connection.lock().await.allow_single_attempt {
            return Err(YappinError::validation(
                "request_once",
                "worker requires WorkerCog::single_attempt_api() and a protocol v6 parent",
            ));
        }
        self.proxy(Wire::ApiOnce {
            method: method.into(),
            path: path.into(),
            body,
        })
        .await
    }
    /// Request an origin-confined JSON operation through the authenticated parent; existing client request retries remain.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.api("GET", "/me", None).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        self.proxy(Wire::Api {
            method: method.into(),
            path: path.into(),
            body,
        })
        .await
    }
    pub(crate) async fn proxy_permissions(
        &self,
        server: uuid::Uuid,
        actor: uuid::Uuid,
        freshness: crate::Freshness,
    ) -> Result<crate::PermissionSnapshot> {
        let value = self
            .proxy(Wire::Permissions {
                server,
                actor,
                prefer_cache: freshness == crate::Freshness::PreferCache,
            })
            .await?;
        Ok(serde_json::from_value(value)?)
    }
    pub(crate) async fn public_request(&self, path: &str) -> Result<Value> {
        self.proxy(Wire::Public { path: path.into() }).await
    }
    pub(crate) async fn upload(
        &self,
        path: &str,
        body: Value,
        files: &[crate::UploadFile],
    ) -> Result<Value> {
        use base64::Engine;
        crate::validate_uploads(files)?;
        self.ensure_active()?;
        let mut connection = self.connection.lock().await;
        self.ensure_active()?;
        if connection.poisoned {
            return Err(protocol::protocol_error());
        }
        connection.poisoned = true;
        protocol::write(
            &mut connection.output,
            &Wire::Upload {
                path: path.into(),
                body,
                files: files
                    .iter()
                    .map(|f| protocol::UploadDescriptor {
                        name: f.name.clone(),
                        size: f.data.len(),
                    })
                    .collect(),
            },
        )
        .await?;
        for (index, file) in files.iter().enumerate() {
            for chunk in file.data.chunks(65536) {
                protocol::write(
                    &mut connection.output,
                    &Wire::UploadChunk {
                        file: index,
                        data: base64::engine::general_purpose::STANDARD.encode(chunk),
                    },
                )
                .await?;
            }
        }
        protocol::write(&mut connection.output, &Wire::UploadFinish).await?;
        Self::api_result(&mut connection).await
    }
    async fn api_result(connection: &mut Connection) -> Result<Value> {
        match protocol::read(&mut connection.input).await? {
            Wire::ApiResult {
                value,
                error,
                structured_error,
            } => {
                connection.poisoned = false;
                if let Some(error) = structured_error {
                    return Err(error.into());
                }
                match error {
                    Some(error) => Err(YappinError::Config(error)),
                    None => Ok(value.unwrap_or(Value::Null)),
                }
            }
            _ => Err(protocol::protocol_error()),
        }
    }
    async fn proxy(&self, request: Wire) -> Result<Value> {
        self.ensure_active()?;
        let mut connection = self.connection.lock().await;
        self.ensure_active()?;
        if connection.poisoned {
            return Err(protocol::protocol_error());
        }
        // Cancellation during an exchange must not let a later call consume its response.
        connection.poisoned = true;
        protocol::write(&mut connection.output, &request).await?;
        Self::api_result(&mut connection).await
    }

    fn ensure_active(&self) -> Result<()> {
        if self.active.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(YappinError::Config(
                "worker handler has already returned".into(),
            ))
        }
    }
    /// Send a message to an explicitly selected channel using the parent API.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.send(channel, rappin::CreateMessage::new("Notice")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn send(&self, channel: uuid::Uuid, message: CreateMessage) -> Result<Message> {
        let clean = message.for_request();
        let message = clean.as_ref();
        message.validate()?;
        let response = self
            .api(
                "POST",
                &format!("/channels/{channel}/messages"),
                Some(serde_json::to_value(message)?),
            )
            .await?;
        Ok(serde_json::from_value(response["message"].clone())?)
    }
    /// Post in the original event channel without a reply reference.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.say("Channel notice").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn say(&self, content: impl Into<String>) -> Result<Message> {
        let channel = self
            .channel_id()
            .ok_or_else(|| YappinError::Config("event has no channel".into()))?;
        self.send(channel, CreateMessage::new(content)).await
    }
    /// Reply in the original message/interaction context.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.reply("Reply to the original message").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reply(&self, content: impl Into<String>) -> Result<()> {
        let content = content.into();
        if let Some(message) = self.event.message() {
            self.send(
                message.channel_id,
                CreateMessage::reply(content, message.id),
            )
            .await?;
            Ok(())
        } else {
            self.respond(InteractionCallback::reply(content)).await
        }
    }
    /// Send a customized reply preserving the original message target.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.reply_with(rappin::CreateMessage::new("Rich reply")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reply_with(&self, mut message: CreateMessage) -> Result<Message> {
        let event = self
            .event
            .message()
            .ok_or_else(|| YappinError::Config("reply_with requires a message event".into()))?;
        if message.reply_to.is_none() {
            message.reply_to = Some(event.id);
        }
        self.send(event.channel_id, message).await
    }
    /// Submit a callback for the original interaction.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.respond(rappin::InteractionCallback::ephemeral("Received")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn respond(&self, callback: InteractionCallback) -> Result<()> {
        callback.validate()?;
        let event = self
            .event
            .interaction()
            .ok_or_else(|| YappinError::Config("event is not an interaction".into()))?;
        let encode = |s: &str| {
            url::form_urlencoded::byte_serialize(s.as_bytes())
                .collect::<String>()
                .replace('+', "%20")
        };
        self.api(
            "POST",
            &format!(
                "/interactions/{}/{}/callback",
                encode(&event.id),
                encode(&event.token)
            ),
            Some(serde_json::to_value(callback)?),
        )
        .await?;
        Ok(())
    }
    /// Send an ephemeral callback reply.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.ephemeral("Only you can see this").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn ephemeral(&self, content: impl Into<String>) -> Result<()> {
        self.respond(InteractionCallback::ephemeral(content)).await
    }
    /// Update the original component message.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.update("Updated").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn update(&self, content: impl Into<String>) -> Result<()> {
        self.respond(InteractionCallback::update(content)).await
    }
    /// Silently acknowledge the original interaction.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerContext};
    /// async fn handle(ctx: &WorkerContext, channel: uuid::Uuid) -> Result<()> {
    ///     let receipt = ctx.ack().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn ack(&self) -> Result<()> {
        self.respond(InteractionCallback::ack()).await
    }
}

/// A small executable cog. Keep stdout exclusively for the protocol; log with eprintln!.
/// Handler futures must finish their API requests before returning (do not detach them).
/// # Example
///
/// ```rust
/// use rappin::hot_reload::WorkerCog;
/// let worker = WorkerCog::new("utilities")
///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await })
///     .button("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
/// // Reserve stdout for IPC. Call run only in the executable started by the parent.
/// ```
pub struct WorkerCog {
    manifest: Manifest,
    commands: HashMap<String, Handler>,
    buttons: HashMap<String, Handler>,
    listeners: HashMap<Listener, Handler>,
    single_attempt_api: bool,
    ui_api: bool,
    operation_identity: bool,
}
impl WorkerCog {
    /// Bind a FromArgs parser and help metadata to a single-name worker route.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities").command_typed("status", "Show status", |ctx, (): ()| async move { ctx.reply("Ready").await });
    /// ```
    pub fn command_typed<A, F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        A: crate::commands::FromArgs,
        F: Fn(WorkerContext, A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        let description = description.into();
        self.manifest.help.push(crate::commands::CommandInfo {
            name: name.clone(),
            description: description.clone(),
            aliases: vec![],
            arguments: A::describe(),
        });
        let handler = Arc::new(handler);
        self.command(name, description, move |ctx| {
            let handler = handler.clone();
            async move {
                let args = ctx.parse::<A>()?;
                handler(ctx, args).await
            }
        })
    }
    /// Create a worker definition without starting IPC or executing handlers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await })
    ///     .button("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
    /// // Reserve stdout for IPC. Call run only in the executable started by the parent.
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            manifest: Manifest {
                name: name.into(),
                commands: vec![],
                buttons: vec![],
                listeners: vec![],
                help: vec![],
            },
            commands: HashMap::new(),
            buttons: HashMap::new(),
            listeners: HashMap::new(),
            single_attempt_api: false,
            ui_api: false,
            operation_identity: false,
        }
    }
    /// Runtime argument schemas on workers use the same parser as native commands.
    /// Worker routes are single command names; use native commands for aliases/subcommands.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{CommandSchema, hot_reload::WorkerCog};
    /// fn configured(schema: CommandSchema) -> rappin::Result<WorkerCog> {
    ///     WorkerCog::new("utilities").dynamic_command(schema, |ctx, values| async move {
    ///         ctx.reply(format!("Parsed {} arguments", values.len())).await
    ///     })
    /// }
    /// ```
    pub fn dynamic_command<F, Fut>(
        mut self,
        schema: crate::CommandSchema,
        handler: F,
    ) -> Result<Self>
    where
        F: Fn(WorkerContext, crate::CommandValues) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        schema.validate()?;
        if schema.name.contains(' ') || !schema.aliases.is_empty() {
            return Err(crate::CommandError::Definition(
                "workers require a single command name without local aliases".into(),
            )
            .into());
        }
        self.manifest.help.push(schema.info());
        let name = schema.name.clone();
        let description = schema.description.clone();
        let schema = Arc::new(schema);
        let handler = Arc::new(handler);
        self = self.command(name, description, move |ctx| {
            let schema = schema.clone();
            let handler = handler.clone();
            async move {
                let values = schema.parse(ctx.args())?;
                handler(ctx, values).await
            }
        });
        Ok(self)
    }
    /// Bind a single-name worker command using parent-authenticated context.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await })
    ///     .button("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
    /// // Reserve stdout for IPC. Call run only in the executable started by the parent.
    /// ```
    pub fn command<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        self.manifest
            .commands
            .push(Command::new(&name, description));
        self.commands.insert(
            name,
            Arc::new(move |ctx| {
                let future = handler(ctx);
                Box::pin(async move { future.await.map(|()| crate::MessageDisposition::Continue) })
            }),
        );
        self
    }
    /// Bind a custom-ID prefix to a fallible interaction handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await })
    ///     .button("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
    /// // Reserve stdout for IPC. Call run only in the executable started by the parent.
    /// ```
    pub fn button<F, Fut>(mut self, prefix: impl Into<String>, handler: F) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let prefix = prefix.into();
        self.manifest.buttons.push(prefix.clone());
        self.buttons.insert(
            prefix,
            Arc::new(move |ctx| {
                let future = handler(ctx);
                Box::pin(async move { future.await.map(|()| crate::MessageDisposition::Continue) })
            }),
        );
        self
    }
    /// Register a fallible event listener; finish required work before returning, and keep stdout for IPC.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::{WorkerCog, Listener};
    /// let worker = WorkerCog::new("observer").listen(Listener::ReactionAdd, |_ctx| async move {
    ///     eprintln!("Reaction received"); Ok(())
    /// });
    /// ```
    pub fn listen<F, Fut>(mut self, kind: Listener, handler: F) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.manifest.listeners.push(kind);
        self.listeners.insert(
            kind,
            Arc::new(move |ctx| {
                let future = handler(ctx);
                Box::pin(async move { future.await.map(|()| crate::MessageDisposition::Continue) })
            }),
        );
        self
    }
    /// Opt-in fallible server-message listener, after command dispatch. Required
    /// I/O must finish before returning. Errors use the parent's message policy.
    /// Do not also register Listener::Message for the same worker.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{MessageDisposition, hot_reload::WorkerCog};
    /// let worker = WorkerCog::new("greetings").on_message(|ctx| async move {
    ///     if let Some(message) = ctx.event.message() {
    ///         if message.content.as_deref() == Some("hello") { ctx.reply("Hello!").await?; }
    ///     }
    ///     Ok(MessageDisposition::Continue)
    /// });
    /// ```
    pub fn on_message<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<crate::MessageDisposition>> + Send + 'static,
    {
        self.manifest.listeners.push(Listener::ServerMessage);
        self.listeners.insert(
            Listener::ServerMessage,
            Arc::new(move |ctx| Box::pin(handler(ctx))),
        );
        self
    }
    /// Extended select/modal/command event values. Parent must opt into interactive_events.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{UiApiExt, InteractionCallbackType, UiMessageBody, hot_reload::WorkerCog};
    /// let worker = WorkerCog::new("forms").on_ui_interaction(|ctx| async move {
    ///     let event = ctx.ui_interaction().expect("UI listener event");
    ///     ctx.respond_ui(&event.interaction, InteractionCallbackType::Ephemeral, &UiMessageBody::new("Received")).await
    /// });
    /// ```
    pub fn on_ui_interaction<F, Fut>(self, handler: F) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.listen(Listener::UiInteraction, handler)
    }
    /// Require a protocol v7 parent for UI callbacks from existing button/command routes.
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities").ui_api();
    /// // Upgrade the parent before enabling the required protocol capability.
    /// ```
    pub fn ui_api(mut self) -> Self {
        self.ui_api = true;
        self
    }
    /// Require parent protocol v6 before starting handlers that use request_once.
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities").single_attempt_api();
    /// // Upgrade the parent before enabling the required protocol capability.
    /// ```
    pub fn single_attempt_api(mut self) -> Self {
        self.single_attempt_api = true;
        self
    }
    /// Opt into protocol v8 operation metadata. Upgrade the parent first.
    /// This does not enable single-attempt API or UI helpers implicitly.
    /// # Example
    ///
    /// ```rust
    /// use rappin::hot_reload::WorkerCog;
    /// let worker = WorkerCog::new("utilities").operation_identity();
    /// // Upgrade the parent before enabling the required protocol capability.
    /// ```
    pub fn operation_identity(mut self) -> Self {
        self.operation_identity = true;
        self
    }
    /// Serve the parent-owned stdio protocol until shutdown; execute only trusted worker code and await all required effects.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Result, hot_reload::WorkerCog};
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     WorkerCog::new("utilities").command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await }).run().await
    /// }
    /// ```
    pub async fn run(self) -> Result<()> {
        self.manifest.validate()?;
        let connection = Arc::new(Mutex::new(Connection {
            input: BufReader::new(tokio::io::stdin()),
            output: tokio::io::stdout(),
            poisoned: false,
            allow_single_attempt: self.single_attempt_api,
            allow_ui: self.ui_api || self.manifest.listeners.contains(&Listener::UiInteraction),
        }));
        protocol::write(
            &mut connection.lock().await.output,
            &Wire::Ready {
                // Existing handlers retain protocol v4 compatibility. Opting into
                // message outcomes requires a parent that understands protocol v5.
                protocol: if self.operation_identity {
                    protocol::VERSION
                } else if self.ui_api || self.manifest.listeners.contains(&Listener::UiInteraction)
                {
                    7
                } else if self.single_attempt_api {
                    6
                } else if self.manifest.listeners.contains(&Listener::ServerMessage) {
                    5
                } else {
                    4
                },
                manifest: self.manifest,
            },
        )
        .await?;
        loop {
            let (event, operation_id) =
                match protocol::read(&mut connection.lock().await.input).await? {
                    Wire::Invoke {
                        event,
                        operation_id,
                    } => (*event, operation_id),
                    Wire::Shutdown => return Ok(()),
                    _ => return Err(protocol::protocol_error()),
                };
            if operation_id.is_some()
                && (!self.operation_identity || !matches!(&event, CogEvent::Command { .. }))
            {
                return Err(protocol::protocol_error());
            }
            let handler = match &event {
                CogEvent::Command { command, .. } => self.commands.get(command),
                CogEvent::Button { prefix, .. } => self.buttons.get(prefix),
                CogEvent::Message { .. } => self
                    .listeners
                    .get(&Listener::ServerMessage)
                    .or_else(|| self.listeners.get(&Listener::Message)),
                CogEvent::DmMessage { .. } => self.listeners.get(&Listener::DmMessage),
                CogEvent::ReactionAdd { .. } => self.listeners.get(&Listener::ReactionAdd),
                CogEvent::ReactionRemove { .. } => self.listeners.get(&Listener::ReactionRemove),
                CogEvent::Interaction { .. } => self.listeners.get(&Listener::Interaction),
                CogEvent::UiInteraction { .. } => self.listeners.get(&Listener::UiInteraction),
                CogEvent::MemberJoin { .. } => self.listeners.get(&Listener::MemberJoin),
                CogEvent::MemberLeave { .. } => self.listeners.get(&Listener::MemberLeave),
                CogEvent::InviteCreate { .. } => self.listeners.get(&Listener::InviteCreate),
                CogEvent::InviteDelete { .. } => self.listeners.get(&Listener::InviteDelete),
            };
            let active = Arc::new(AtomicBool::new(true));
            let result = match handler {
                Some(handler) => {
                    handler(WorkerContext {
                        event,
                        connection: connection.clone(),
                        active: active.clone(),
                        operation_id,
                    })
                    .await
                }
                None => Err(YappinError::Config(
                    "worker has no handler for this event".into(),
                )),
            };
            let mut connection = connection.lock().await;
            active.store(false, Ordering::SeqCst);
            if connection.poisoned {
                return Err(protocol::protocol_error());
            }
            protocol::write(
                &mut connection.output,
                &Wire::Done {
                    disposition: result.as_ref().ok().copied(),
                    structured_error: result.as_ref().err().map(|e| e.to_remote()),
                    error: result.err().map(|e| e.to_string()),
                },
            )
            .await?;
        }
    }
}
