//! Shared command handlers over original message, interaction and worker contexts.
use crate::{Api, FromArgs, InteractionContext, MessageContext, Result};
use uuid::Uuid;

#[derive(Clone)]
enum Source {
    Message(MessageContext),
    Interaction(InteractionContext),
    #[cfg(feature = "hot-reload")]
    Worker(crate::hot_reload::WorkerContext),
}

/// A shared handler view retaining its original authenticated event and reply route.
/// Conversion creates no new authority or checked-invocation provenance.
/// Use `api()` for the common native/worker transport and the original context
/// for transport-specific features. Permission checks remain handler-owned.
///
/// # Example
/// ```no_run
/// async fn status(ctx: rappin::CommandContext) -> rappin::Result<()> {
///     ctx.parse::<()>()?;
///     ctx.reply("Ready").await
/// }
/// let bot = rappin::Bot::builder("token")
///     .command_spec(rappin::CommandSpec::<()>::new("status", "Show status"),
///         |ctx, ()| status(ctx)).build()?;
/// # Ok::<(), rappin::Error>(())
/// ```
#[derive(Clone)]
pub struct CommandContext {
    source: Source,
    resolved: Option<String>,
    arguments: Option<String>,
}
impl From<MessageContext> for CommandContext {
    fn from(ctx: MessageContext) -> Self {
        Self {
            source: Source::Message(ctx),
            resolved: None,
            arguments: None,
        }
    }
}
impl From<InteractionContext> for CommandContext {
    fn from(ctx: InteractionContext) -> Self {
        Self {
            source: Source::Interaction(ctx),
            resolved: None,
            arguments: None,
        }
    }
}
#[cfg(feature = "hot-reload")]
impl From<crate::hot_reload::WorkerContext> for CommandContext {
    fn from(ctx: crate::hot_reload::WorkerContext) -> Self {
        Self {
            source: Source::Worker(ctx),
            resolved: None,
            arguments: None,
        }
    }
}
impl CommandContext {
    pub(crate) fn resolved(ctx: InteractionContext, name: String, args: String) -> Self {
        Self {
            source: Source::Interaction(ctx),
            resolved: Some(name),
            arguments: Some(args),
        }
    }
    pub(crate) fn for_message(ctx: MessageContext, name: String) -> Self {
        Self {
            source: Source::Message(ctx),
            resolved: Some(name),
            arguments: None,
        }
    }
    pub(crate) fn native_client(&self) -> Option<&crate::Client> {
        match &self.source {
            Source::Message(ctx) => Some(&ctx.client),
            Source::Interaction(ctx) => Some(&ctx.client),
            #[cfg(feature = "hot-reload")]
            Source::Worker(_) => None,
        }
    }
    /// Access the native bot controller. Workers use their process-local state
    /// and parent API proxy instead of receiving an in-process bot handle.
    ///
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let bot = ctx.bot(); }
    /// ```
    pub fn bot(&self) -> Option<&crate::BotHandle> {
        match &self.source {
            Source::Message(ctx) => ctx.bot(),
            Source::Interaction(ctx) => ctx.bot(),
            #[cfg(feature = "hot-reload")]
            Source::Worker(_) => None,
        }
    }

    /// Canonical installed command path, preserving the original event separately.
    ///
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let command = ctx.command_name(); }
    /// ```
    pub fn command_name(&self) -> Option<&str> {
        if let Some(name) = &self.resolved {
            return Some(name);
        }
        match &self.source {
            Source::Message(ctx) => ctx.command.as_deref(),
            Source::Interaction(ctx) => ctx.command.as_deref(),
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => match &ctx.event {
                crate::CogEvent::Command { command, .. }
                | crate::CogEvent::InteractionCommand { command, .. } => Some(command),
                _ => None,
            },
        }
    }
    /// Retrieve application state from the owning bot or worker generation.
    ///
    /// ```rust
    /// fn state(ctx: &rappin::CommandContext) -> rappin::Result<std::sync::Arc<String>> {
    ///     ctx.state::<String>()
    /// }
    /// ```
    pub fn state<T: Send + Sync + 'static>(&self) -> Result<std::sync::Arc<T>> {
        match &self.source {
            Source::Message(ctx) => ctx.state(),
            Source::Interaction(ctx) => ctx.state(),
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx.state(),
        }
    }
    /// Borrow the original message context, when this is a native message command.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let original = ctx.message(); }
    /// ```
    pub fn message(&self) -> Option<&MessageContext> {
        match &self.source {
            Source::Message(ctx) => Some(ctx),
            _ => None,
        }
    }
    /// Borrow the original interaction, including its acknowledgement coordinator.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let original = ctx.interaction(); }
    /// ```
    pub fn interaction(&self) -> Option<&InteractionContext> {
        match &self.source {
            Source::Interaction(ctx) => Some(ctx),
            _ => None,
        }
    }
    /// Borrow the original worker proxy context.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let original = ctx.worker(); }
    /// ```
    #[cfg(feature = "hot-reload")]
    pub fn worker(&self) -> Option<&crate::hot_reload::WorkerContext> {
        match &self.source {
            Source::Worker(ctx) => Some(ctx),
            _ => None,
        }
    }
    /// Borrow the credential-confined transport. Its existing retry/permission
    /// semantics apply. Successful bot requests do not establish requester authority.
    /// # Example
    /// ```no_run
    /// use rappin::ApiExt;
    /// async fn inspect(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     let me = ctx.api().get_me().await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn api(&self) -> &dyn Api {
        match &self.source {
            Source::Message(ctx) => ctx,
            Source::Interaction(ctx) => ctx,
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx,
        }
    }
    /// Original requester, with absence preserved for incomplete interactions.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let requester = ctx.author_id(); }
    /// ```
    pub fn author_id(&self) -> Option<Uuid> {
        match &self.source {
            Source::Message(ctx) => Some(ctx.author_id()),
            Source::Interaction(ctx) => ctx.event.user_id,
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx.author_id(),
        }
    }
    /// Original channel; absence grants no scope.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let channel = ctx.channel_id(); }
    /// ```
    pub fn channel_id(&self) -> Option<Uuid> {
        match &self.source {
            Source::Message(ctx) => Some(ctx.channel_id()),
            Source::Interaction(ctx) => ctx.channel_id(),
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx.channel_id(),
        }
    }
    /// Original server; no synthetic message or server is manufactured.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let server = ctx.server_id(); }
    /// ```
    pub fn server_id(&self) -> Option<Uuid> {
        match &self.source {
            Source::Message(ctx) => ctx.server_id(),
            Source::Interaction(ctx) => ctx.server_id(),
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx
                .event
                .message()
                .and_then(|m| m.server_id)
                .or_else(|| ctx.event.interaction().and_then(|i| i.server_id)),
        }
    }
    /// Borrow the original raw argument string.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let text = ctx.args(); }
    /// ```
    pub fn args(&self) -> &str {
        if let Some(args) = &self.arguments {
            return args;
        }
        match &self.source {
            Source::Message(ctx) => &ctx.args,
            Source::Interaction(ctx) => ctx.args.as_deref().unwrap_or(""),
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx.args(),
        }
    }
    /// Parse with the existing typed parser and reject trailing arguments.
    /// # Example
    /// ```rust
    /// fn no_arguments(ctx: &rappin::CommandContext) -> rappin::Result<()> { ctx.parse::<()>() }
    /// ```
    pub fn parse<A: FromArgs>(&self) -> Result<A> {
        let mut arguments = crate::Arguments::new(self.args())?;
        let value = A::parse(&mut arguments)?;
        arguments.finish()?;
        Ok(value)
    }
    /// Preserve the consumer's local operation label. It grants no permission
    /// and does not make remote writes idempotent.
    /// # Example
    /// ```rust
    /// fn inspect(ctx: &rappin::CommandContext) { let operation = ctx.operation_id(); }
    /// ```
    pub fn operation_id(&self) -> Option<crate::OperationId> {
        match &self.source {
            Source::Message(ctx) => ctx.operation_id(),
            Source::Interaction(_) => None,
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx.operation_id().cloned(),
        }
    }
    /// Wait for a filtered event on the owning bot's hub or worker proxy.
    /// The caller chooses the filter. Use prompt_for_reply for requester-scoped prompts.
    ///
    /// ```no_run
    /// async fn observe(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     let event = ctx.wait_for_event(rappin::EventFilter::default(), std::time::Duration::from_secs(10)).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn wait_for_event(
        &self,
        filter: crate::EventFilter,
        timeout: std::time::Duration,
    ) -> Result<crate::Event> {
        #[cfg(feature = "hot-reload")]
        if let Some(worker) = self.worker() {
            return worker.wait_for_event(filter, timeout).await;
        }
        let client = self.native_client().ok_or_else(|| {
            crate::Error::validation(
                "command.context",
                "event collection requires an original context",
            )
        })?;
        Ok(client
            .events()
            .collect::<crate::Event>()
            .criteria(filter)
            .timeout(timeout)
            .subscribe()?
            .wait()
            .await?)
    }

    /// Register a requester/channel reply collector before sending its prompt.
    /// Uses the original reply route. Workers delegate registration and the
    /// prompt to the parent, preserving the same fast-answer ordering.
    ///
    /// ```no_run
    /// async fn ask(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     let answer = ctx.prompt_for_reply("Your choice?", std::time::Duration::from_secs(30)).await?;
    ///     ctx.reply(answer.content.unwrap_or_default()).await
    /// }
    /// ```
    pub async fn prompt_for_reply(
        &self,
        prompt: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<crate::MessageEvent> {
        #[cfg(feature = "hot-reload")]
        if let Some(worker) = self.worker() {
            return worker.prompt_for_reply(prompt, timeout).await;
        }
        let client = self.native_client().ok_or_else(|| {
            crate::Error::validation(
                "command.context",
                "reply collection requires an original context",
            )
        })?;
        let channel = self.channel_id().ok_or_else(|| {
            crate::Error::validation("command.channel", "reply collection requires a channel")
        })?;
        let author = self.author_id().ok_or_else(|| {
            crate::Error::validation("command.author", "reply collection requires a requester")
        })?;
        let waiter = client
            .events()
            .collect::<crate::MessageEvent>()
            .channel(channel)
            .author(author)
            .timeout(timeout)
            .subscribe()?;
        self.reply(prompt).await?;
        Ok(waiter.wait().await?)
    }

    /// Send a new message in the original channel and return the receipt.
    /// Missing channel identity returns a structured validation error.
    ///
    /// ```no_run
    /// async fn notify(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     let receipt = ctx.say("Status updated").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn say(&self, content: impl Into<String>) -> Result<crate::Message> {
        let channel = self.channel_id().ok_or_else(|| {
            crate::Error::validation("command.channel", "message requires the original channel")
        })?;
        crate::ApiExt::send_message(self, channel, &crate::CreateMessage::new(content)).await
    }

    /// Reply with rich content through the original delivery's reply coordinator.
    ///
    /// ```no_run
    /// async fn notify(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     ctx.reply_with(rappin::CreateMessage::new("Ready")).await
    /// }
    /// ```
    pub async fn reply_with(&self, body: crate::CreateMessage) -> Result<()> {
        match &self.source {
            Source::Message(ctx) => ctx.reply_with(body).await.map(|_| ()),
            Source::Interaction(ctx) => ctx.reply_with(body).await,
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) if ctx.event.message().is_some() => {
                ctx.reply_with(body).await.map(|_| ())
            }
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => {
                let event = ctx.event.interaction().ok_or_else(|| {
                    crate::Error::validation(
                        "command.interaction",
                        "reply requires an original interaction",
                    )
                })?;
                crate::ApiExt::respond(ctx, event, crate::InteractionCallbackType::Reply, &body)
                    .await
            }
        }
    }

    /// Reply with a paginator's first page and return its navigation helper.
    ///
    /// ```no_run
    /// async fn pages(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     let paginator = ctx.send_paginated([rappin::Embed::new().title("Help")]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_paginated(
        &self,
        embeds: impl IntoIterator<Item = crate::Embed>,
    ) -> Result<crate::paginator::Paginator> {
        let paginator = crate::paginator::Paginator::new(embeds);
        self.reply_with(paginator.build_message(0)).await?;
        Ok(paginator)
    }

    /// Reply with confirmation buttons and return their dialog helper.
    ///
    /// ```no_run
    /// async fn confirm(ctx: &rappin::CommandContext) -> rappin::Result<()> {
    ///     let dialog = ctx.send_confirm("Continue?").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_confirm(
        &self,
        content: impl Into<String>,
    ) -> Result<crate::paginator::ConfirmDialog> {
        let dialog = crate::paginator::ConfirmDialog::new(content);
        self.reply_with(dialog.build_message()).await?;
        Ok(dialog)
    }

    /// Consume an execution-time cooldown bucket using the original scope.
    /// Missing required actor/channel identity is an explicit error. Preparation
    /// never calls this method. Cloned cooldowns share their existing local buckets.
    ///
    /// ```no_run
    /// async fn admit(ctx: &rappin::CommandContext, cooldown: &rappin::Cooldown) -> rappin::Result<()> {
    ///     if let Some(wait) = ctx.check_cooldown(cooldown).await? {
    ///         return Err(rappin::CommandError::Denied(format!("Try again in {wait:?}")).into());
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn check_cooldown(
        &self,
        cooldown: &crate::Cooldown,
    ) -> Result<Option<std::time::Duration>> {
        let key = match cooldown.kind() {
            crate::CooldownType::User => self
                .author_id()
                .ok_or_else(|| {
                    crate::Error::validation(
                        "command.author",
                        "cooldown requires the original requester",
                    )
                })?
                .to_string(),
            crate::CooldownType::Channel => self
                .channel_id()
                .ok_or_else(|| {
                    crate::Error::validation(
                        "command.channel",
                        "cooldown requires the original channel",
                    )
                })?
                .to_string(),
            crate::CooldownType::Server => self
                .server_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "dm".into()),
            crate::CooldownType::Global => "global".into(),
        };
        Ok(cooldown.check_key(&key).await)
    }

    /// Reply through the original message target or interaction response
    /// coordinator. A failed/uncertain reply remains an error for the caller.
    /// # Example
    /// ```no_run
    /// async fn ping(ctx: rappin::CommandContext) -> rappin::Result<()> { ctx.reply("pong").await }
    /// ```
    pub async fn reply(&self, content: impl Into<String>) -> Result<()> {
        let content = content.into();
        match &self.source {
            Source::Message(ctx) => ctx.reply(content).await.map(|_| ()),
            Source::Interaction(ctx) => ctx.reply(content).await,
            #[cfg(feature = "hot-reload")]
            Source::Worker(ctx) => ctx.reply(content).await,
        }
    }
}
