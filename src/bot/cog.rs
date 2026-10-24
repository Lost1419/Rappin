use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::bot::context::{InteractionContext, MessageContext};
use crate::client::Client;
use crate::error::Result;
use crate::models::{MemberJoinEvent, MemberLeaveEvent};

pub type CommandHandlerFn =
    Arc<dyn Fn(MessageContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;

pub type ButtonHandlerFn = Arc<
    dyn Fn(InteractionContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

/// A command defined within a cog.
#[derive(Clone)]
pub struct CogCommand {
    pub name: String,
    pub description: String,
    pub handler: CommandHandlerFn,
}

impl CogCommand {
    pub fn new<F, Fut>(name: impl Into<String>, description: impl Into<String>, handler: F) -> Self
    where
        F: Fn(MessageContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            name: name.into().to_lowercase(),
            description: description.into(),
            handler: Arc::new(move |ctx| Box::pin(handler(ctx))),
        }
    }
}

/// A button handler defined within a cog.
#[derive(Clone)]
pub struct CogButton {
    pub prefix: String,
    pub handler: ButtonHandlerFn,
}

impl CogButton {
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
#[async_trait]
pub trait Cog: Send + Sync + 'static {
    /// Unique identifier for this cog (e.g. "moderation", "fun", "music").
    fn name(&self) -> &str;

    /// Commands exposed by this cog.
    fn commands(&self) -> Vec<CogCommand> {
        Vec::new()
    }

    /// Button prefix handlers exposed by this cog.
    fn buttons(&self) -> Vec<CogButton> {
        Vec::new()
    }

    /// Optional condition/permission check evaluated before any command in this cog runs.
    ///
    /// If this returns `false`, the command will not be executed.
    async fn check(&self, _ctx: &MessageContext) -> bool {
        true
    }

    /// Hook executed when the cog is loaded into the bot.
    async fn on_load(&self, _client: &Client) -> Result<()> {
        Ok(())
    }

    /// Hook executed when the cog is unloaded from the bot.
    async fn on_unload(&self, _client: &Client) -> Result<()> {
        Ok(())
    }

    /// Optional message listener hook.
    async fn on_message(&self, _ctx: &MessageContext) {}

    /// Optional interaction listener hook.
    async fn on_interaction(&self, _ctx: &InteractionContext) {}

    /// Optional member join hook.
    async fn on_member_join(&self, _client: &Client, _event: &MemberJoinEvent) {}

    /// Optional member leave hook.
    async fn on_member_leave(&self, _client: &Client, _event: &MemberLeaveEvent) {}
}

/// Builder for dynamically creating a [`Cog`] without implementing the full trait manually.
pub struct CogBuilder {
    name: String,
    commands: Vec<CogCommand>,
    buttons: Vec<CogButton>,
}

impl CogBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            commands: Vec::new(),
            buttons: Vec::new(),
        }
    }

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
        self.commands
            .push(CogCommand::new(name, description, handler));
        self
    }

    pub fn button<F, Fut>(mut self, prefix: impl Into<String>, handler: F) -> Self
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.buttons.push(CogButton::new(prefix, handler));
        self
    }

    pub fn build(self) -> DynamicCog {
        DynamicCog {
            name: self.name,
            commands: self.commands,
            buttons: self.buttons,
        }
    }
}

pub struct DynamicCog {
    name: String,
    commands: Vec<CogCommand>,
    buttons: Vec<CogButton>,
}

#[async_trait]
impl Cog for DynamicCog {
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
