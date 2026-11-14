//! Shared command handlers over original message, interaction and worker contexts.
use crate::{Api, FromArgs, MessageContext, Result, UiInteractionContext};
use uuid::Uuid;

#[derive(Clone)]
enum Source {
    Message(MessageContext),
    Interaction(UiInteractionContext),
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
///     .command_shared("status", "Show status", status).build();
/// ```
#[derive(Clone)]
pub struct CommandContext {
    source: Source,
}
impl From<MessageContext> for CommandContext {
    fn from(ctx: MessageContext) -> Self {
        Self {
            source: Source::Message(ctx),
        }
    }
}
impl From<UiInteractionContext> for CommandContext {
    fn from(ctx: UiInteractionContext) -> Self {
        Self {
            source: Source::Interaction(ctx),
        }
    }
}
#[cfg(feature = "hot-reload")]
impl From<crate::hot_reload::WorkerContext> for CommandContext {
    fn from(ctx: crate::hot_reload::WorkerContext) -> Self {
        Self {
            source: Source::Worker(ctx),
        }
    }
}
impl CommandContext {
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
    pub fn interaction(&self) -> Option<&UiInteractionContext> {
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
        match &self.source {
            Source::Message(ctx) => &ctx.args,
            Source::Interaction(ctx) => ctx.data.args.as_deref().unwrap_or(""),
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
