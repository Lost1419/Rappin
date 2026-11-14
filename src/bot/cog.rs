//! Native modular commands, buttons and listeners with runtime installation/replacement.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::bot::context::{InteractionContext, MessageContext};
use crate::client::Client;
use crate::error::Result;
use crate::models::{MemberJoinEvent, MemberLeaveEvent};

/// Shared Send asynchronous native command handler returning explicit completion or a structured failure.
pub type CommandHandlerFn = Arc<
    dyn Fn(crate::CommandContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

/// Shared Send asynchronous native interaction handler returning explicit completion or a structured failure.
pub type ButtonHandlerFn = Arc<
    dyn Fn(InteractionContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

/// A command defined within a cog.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::bot::cog::{CogCommand, CogButton};
/// let command = CogCommand::new("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) });
/// let button = CogButton::new("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
/// assert_eq!(command.name, "ping");
/// assert_eq!(button.prefix, "confirm:");
/// ```
pub struct CogCommand {
    /// Canonical local command name.
    pub name: String,
    /// Description advertised with the command.
    pub description: String,
    /// Shared native asynchronous command handler.
    pub handler: CommandHandlerFn,
}

impl CogCommand {
    /// Bind a native handler to local command metadata.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::bot::cog::{CogCommand, CogButton};
    /// let command = CogCommand::new("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) });
    /// let button = CogButton::new("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
    /// assert_eq!(command.name, "ping");
    /// assert_eq!(button.prefix, "confirm:");
    /// ```
    pub fn new<F, Fut>(name: impl Into<String>, description: impl Into<String>, handler: F) -> Self
    where
        F: Fn(crate::CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            name: name.into().to_lowercase(),
            description: description.into(),
            handler: Arc::new(move |ctx| Box::pin(handler(ctx))),
        }
    }

    /// Creates a command with an automatic rate limit cooldown.
    /// # Example
    ///
    /// ```rust
    /// use rappin::bot::cog::CogCommand;
    /// use rappin::cooldown::Cooldown;
    /// let command = CogCommand::with_cooldown("ping", "Check connectivity", Cooldown::user(std::time::Duration::from_secs(5)), |ctx| async move { ctx.reply("pong").await?; Ok(()) });
    /// ```
    pub fn with_cooldown<F, Fut>(
        name: impl Into<String>,
        description: impl Into<String>,
        cooldown: crate::cooldown::Cooldown,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let wrapped = move |ctx: crate::CommandContext| {
            let cd = cooldown.clone();
            let handler = handler.clone();
            async move {
                if let Some(remaining) = ctx.check_cooldown(&cd).await? {
                    let secs = remaining.as_secs_f32();
                    return Err(crate::CommandError::Denied(format!(
                        "You are on cooldown; wait {secs:.1}s"
                    ))
                    .into());
                }
                handler(ctx).await
            }
        };
        Self::new(name, description, wrapped)
    }
}

/// A button handler defined within a cog.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::bot::cog::{CogCommand, CogButton};
/// let command = CogCommand::new("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) });
/// let button = CogButton::new("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
/// assert_eq!(command.name, "ping");
/// assert_eq!(button.prefix, "confirm:");
/// ```
pub struct CogButton {
    /// Custom-ID prefix selecting this button handler.
    pub prefix: String,
    /// Shared native asynchronous interaction handler.
    pub handler: ButtonHandlerFn,
}

impl CogButton {
    /// Bind a native interaction handler to a custom-ID prefix.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::bot::cog::{CogCommand, CogButton};
    /// let command = CogCommand::new("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) });
    /// let button = CogButton::new("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await });
    /// assert_eq!(command.name, "ping");
    /// assert_eq!(button.prefix, "confirm:");
    /// ```
    pub fn new<F, Fut>(prefix: impl Into<String>, handler: F) -> Self
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            prefix: prefix.into(),
            handler: Arc::new(move |ctx| Box::pin(handler(ctx))),
        }
    }
}

/// Trait representing a modular Cog.
///
/// Cogs allow splitting commands, button interactions, and event listeners into
/// distinct modules that can be added, removed, or reloaded while the bot is running.
/// Replacing a native Cog uses code already linked into the bot. For edited Rust source,
/// enable `hot-reload` and use `crate::hot_reload::WorkerCog` in a separate Rust binary.
#[async_trait]
/// # Example
///
/// ```rust
/// use async_trait::async_trait;
/// use rappin::{Cog, MessageContext, MessageDisposition, Result};
/// struct Greetings;
/// #[async_trait]
/// impl Cog for Greetings {
///     fn name(&self) -> &str { "greetings" }
///     async fn on_message(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
///         Ok(MessageDisposition::Continue)
///     }
/// }
/// ```
pub trait Cog: Send + Sync + 'static {
    /// Process a resource lifecycle event. Errors retain the event's checkpoint barrier.
    ///
    /// ```rust
    /// struct Observer;
    /// #[async_trait::async_trait]
    /// impl rappin::Cog for Observer {
    ///     async fn on_resource_event(&self, _client: &rappin::Client, event: &rappin::ResourceEvent) -> rappin::Result<()> {
    ///         println!("{} {}", event.kind.as_str(), event.id);
    ///         Ok(())
    ///     }
    ///     fn name(&self) -> &str { "observer" }
    /// }
    /// ```
    async fn on_resource_event(
        &self,
        client: &Client,
        event: &crate::ResourceEvent,
    ) -> crate::Result<()> {
        let _ = (client, event);
        Ok(())
    }

    /// Typed commands retain their parsing, checks and generated help when installed or reloaded.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     fn registered_commands(&self) -> Vec<rappin::RegisteredCommand> { vec![rappin::CommandSpec::<()>::new("status", "Show status").bind(|ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })] }
    /// }
    /// ```
    fn registered_commands(&self) -> Vec<crate::commands::RegisteredCommand> {
        vec![]
    }
    /// Unique identifier for this cog (e.g. "moderation", "fun", "music").
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::{Cog, MessageContext, MessageDisposition, Result};
    /// struct Greetings;
    /// #[async_trait]
    /// impl Cog for Greetings {
    ///     fn name(&self) -> &str { "greetings" }
    ///     async fn on_message(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
    ///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
    ///         Ok(MessageDisposition::Continue)
    ///     }
    /// }
    /// ```
    fn name(&self) -> &str;

    /// Commands exposed by this cog.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     fn commands(&self) -> Vec<rappin::bot::cog::CogCommand> { vec![rappin::bot::cog::CogCommand::new("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })] }
    /// }
    /// ```
    fn commands(&self) -> Vec<CogCommand> {
        Vec::new()
    }

    /// Button prefix handlers exposed by this cog.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     fn buttons(&self) -> Vec<rappin::bot::cog::CogButton> { vec![rappin::bot::cog::CogButton::new("confirm:", |ctx| async move { ctx.ephemeral("Confirmed").await })] }
    /// }
    /// ```
    fn buttons(&self) -> Vec<CogButton> {
        Vec::new()
    }

    /// Optional condition/permission check evaluated before any command in this cog runs.
    ///
    /// If this returns `false`, the command will not be executed.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     async fn check(&self, ctx: &rappin::CommandContext) -> rappin::Result<bool> { Ok(ctx.server_id().is_some()) }
    /// }
    /// ```
    async fn check(&self, _ctx: &crate::CommandContext) -> Result<bool> {
        Ok(true)
    }

    /// Hook executed when the cog is loaded into the bot.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     async fn on_load(&self, _client: &rappin::Client) -> rappin::Result<()> { tracing::info!("Cog loaded"); Ok(()) }
    /// }
    /// ```
    async fn on_load(&self, _client: &Client) -> Result<()> {
        Ok(())
    }

    /// Hook executed when the cog is unloaded from the bot.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     async fn on_unload(&self, _client: &rappin::Client) -> rappin::Result<()> { tracing::info!("Cog unloaded"); Ok(()) }
    /// }
    /// ```
    async fn on_unload(&self, _client: &Client) -> Result<()> {
        Ok(())
    }

    /// Fallible server-message listener. The default continues dispatch.
    /// Errors enter the bot's message failure policy.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::{Cog, MessageContext, MessageDisposition, Result};
    /// struct Greetings;
    /// #[async_trait]
    /// impl Cog for Greetings {
    ///     fn name(&self) -> &str { "greetings" }
    ///     async fn on_message(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
    ///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
    ///         Ok(MessageDisposition::Continue)
    ///     }
    /// }
    /// ```
    async fn on_message(&self, _ctx: &MessageContext) -> Result<super::MessageDisposition> {
        Ok(super::MessageDisposition::Continue)
    }
    /// Fallible reaction listener; errors retain event completion barriers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Observer;
    /// #[async_trait]
    /// impl Cog for Observer {
    ///     fn name(&self) -> &str { "observer" }
    ///     async fn on_reaction_add(&self, _client: &rappin::Client, _event: &rappin::ReactionEvent) -> rappin::Result<()> { tracing::debug!("Event observed"); Ok(()) }
    /// }
    /// ```
    async fn on_reaction_add(
        &self,
        _client: &Client,
        _event: &crate::ReactionEvent,
    ) -> crate::Result<()> {
        Ok(())
    }
    /// Fallible reaction listener; errors retain event completion barriers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Observer;
    /// #[async_trait]
    /// impl Cog for Observer {
    ///     fn name(&self) -> &str { "observer" }
    ///     async fn on_reaction_remove(&self, _client: &rappin::Client, _event: &rappin::ReactionEvent) -> rappin::Result<()> { tracing::debug!("Event observed"); Ok(()) }
    /// }
    /// ```
    async fn on_reaction_remove(
        &self,
        _client: &Client,
        _event: &crate::ReactionEvent,
    ) -> crate::Result<()> {
        Ok(())
    }
    /// Incoming DM; separate from server message/command dispatch.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     async fn on_dm(&self, ctx: &rappin::MessageContext) -> rappin::Result<()> { ctx.reply("DM received").await?; Ok(()) }
    /// }
    /// ```
    async fn on_dm(&self, _ctx: &MessageContext) -> crate::Result<()> {
        Ok(())
    }

    /// Optional interaction listener hook.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Module;
    /// #[async_trait]
    /// impl Cog for Module {
    ///     fn name(&self) -> &str { "module" }
    ///     async fn on_interaction(&self, ctx: &rappin::InteractionContext) -> rappin::Result<()> { tracing::debug!(interaction = ctx.interaction_id(), "Interaction observed");  Ok(()) }
    /// }
    /// ```
    async fn on_interaction(&self, _ctx: &InteractionContext) -> crate::Result<()> {
        Ok(())
    }

    /// Optional member join hook.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Observer;
    /// #[async_trait]
    /// impl Cog for Observer {
    ///     fn name(&self) -> &str { "observer" }
    ///     async fn on_member_join(&self, _client: &rappin::Client, _event: &rappin::MemberJoinEvent) -> rappin::Result<()> { tracing::debug!("Event observed");  Ok(()) }
    /// }
    /// ```
    async fn on_member_join(
        &self,
        _client: &Client,
        _event: &MemberJoinEvent,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// Optional member leave hook.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Observer;
    /// #[async_trait]
    /// impl Cog for Observer {
    ///     fn name(&self) -> &str { "observer" }
    ///     async fn on_member_leave(&self, _client: &rappin::Client, _event: &rappin::MemberLeaveEvent) -> rappin::Result<()> { tracing::debug!("Event observed");  Ok(()) }
    /// }
    /// ```
    async fn on_member_leave(
        &self,
        _client: &Client,
        _event: &MemberLeaveEvent,
    ) -> crate::Result<()> {
        Ok(())
    }
    /// Optional invite-create listener (Manage Server; test servers only).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Observer;
    /// #[async_trait]
    /// impl Cog for Observer {
    ///     fn name(&self) -> &str { "observer" }
    ///     async fn on_invite_create(&self, _client: &rappin::Client, _event: &rappin::InviteEvent) -> rappin::Result<()> { tracing::debug!("Event observed");  Ok(()) }
    /// }
    /// ```
    async fn on_invite_create(
        &self,
        _client: &Client,
        _event: &crate::InviteEvent,
    ) -> crate::Result<()> {
        Ok(())
    }
    /// Optional invite-delete listener (Manage Server; test servers only).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::Cog;
    /// struct Observer;
    /// #[async_trait]
    /// impl Cog for Observer {
    ///     fn name(&self) -> &str { "observer" }
    ///     async fn on_invite_delete(&self, _client: &rappin::Client, _event: &rappin::InviteEvent) -> rappin::Result<()> { tracing::debug!("Event observed");  Ok(()) }
    /// }
    /// ```
    async fn on_invite_delete(
        &self,
        _client: &Client,
        _event: &crate::InviteEvent,
    ) -> crate::Result<()> {
        Ok(())
    }
}

/// Builder for dynamically creating a [`Cog`] without implementing the full trait manually.
/// # Example
///
/// ```rust
/// use rappin::{Cog, CogBuilder, CommandSpec};
/// use rappin::cooldown::Cooldown;
/// use std::time::Duration;
/// let cog = CogBuilder::new("utilities")
///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
///     .build();
/// assert_eq!(cog.name(), "utilities");
/// ```
pub struct CogBuilder {
    name: String,
    commands: Vec<CogCommand>,
    buttons: Vec<CogButton>,
    registered: Vec<crate::commands::RegisteredCommand>,
}

impl CogBuilder {
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            commands: Vec::new(),
            buttons: Vec::new(),
            registered: Vec::new(),
        }
    }

    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn command<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.commands
            .push(CogCommand::new(name, description, handler));
        self
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn command_spec<A, F, Fut>(
        mut self,
        spec: crate::commands::CommandSpec<A>,
        handler: F,
    ) -> Self
    where
        A: crate::commands::FromArgs,
        F: Fn(crate::CommandContext, A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<()>> + Send + 'static,
    {
        self.registered.push(spec.bind(handler));
        self
    }
    /// Add a pre-bound typed or runtime command without modifying its parsing/check metadata.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn registered_command(mut self, command: crate::RegisteredCommand) -> Self {
        self.registered.push(command);
        self
    }

    /// Registers a cog command with an automatic rate limit cooldown.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn command_with_cooldown<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        cooldown: crate::cooldown::Cooldown,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.commands.push(CogCommand::with_cooldown(
            name,
            description,
            cooldown,
            handler,
        ));
        self
    }

    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn button<F, Fut>(mut self, prefix: impl Into<String>, handler: F) -> Self
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.buttons.push(CogButton::new(prefix, handler));
        self
    }

    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cog, CogBuilder, CommandSpec};
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// let cog = CogBuilder::new("utilities")
    ///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
    ///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
    ///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
    ///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
    ///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
    ///     .build();
    /// assert_eq!(cog.name(), "utilities");
    /// ```
    pub fn build(self) -> DynamicCog {
        DynamicCog {
            name: self.name,
            commands: self.commands,
            buttons: self.buttons,
            registered: self.registered,
        }
    }
}

/// A native cog built from registered commands and button routes.
///
/// # Example
///
/// ```rust
/// use rappin::{Cog, CogBuilder, CommandSpec};
/// use rappin::cooldown::Cooldown;
/// use std::time::Duration;
/// let cog = CogBuilder::new("utilities")
///     .command("ping", "Check connectivity", |ctx| async move { ctx.reply("pong").await?; Ok(()) })
///     .command_spec(CommandSpec::<()>::new("status", "Show status"), |ctx, ()| async move { ctx.reply("Ready").await?; Ok(()) })
///     .registered_command(CommandSpec::<()>::new("version", "Show version").bind(|ctx, ()| async move { ctx.reply("v1").await?; Ok(()) }))
///     .command_with_cooldown("limited", "Rate limited command", Cooldown::user(Duration::from_secs(5)), |ctx| async move { ctx.reply("Accepted").await?; Ok(()) })
///     .button("utilities:", |ctx| async move { ctx.ephemeral("Clicked").await })
///     .build();
/// assert_eq!(cog.name(), "utilities");
/// ```
pub struct DynamicCog {
    name: String,
    commands: Vec<CogCommand>,
    buttons: Vec<CogButton>,
    registered: Vec<crate::commands::RegisteredCommand>,
}

#[async_trait]
impl Cog for DynamicCog {
    fn registered_commands(&self) -> Vec<crate::commands::RegisteredCommand> {
        self.registered.clone()
    }
    fn name(&self) -> &str {
        &self.name
    }

    fn commands(&self) -> Vec<CogCommand> {
        self.commands.clone()
    }

    fn buttons(&self) -> Vec<CogButton> {
        self.buttons.clone()
    }
}
