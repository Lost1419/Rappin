//! Legacy observer and fallible listener contracts for dispatched events.
use async_trait::async_trait;

use crate::bot::context::{InteractionContext, MessageContext};
use crate::client::Client;
use crate::models::{BotProfile, MemberJoinEvent, MemberLeaveEvent, ReactionEvent, VoiceEvent};

/// Trait for receiving dispatched Yapin' events.
#[async_trait]
/// # Example
///
/// ```rust
/// use async_trait::async_trait;
/// use rappin::{EventHandler, MessageContext, MessageDisposition, Result};
/// struct Listener;
/// #[async_trait]
/// impl EventHandler for Listener {
///     async fn on_message_result(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
///         Ok(MessageDisposition::Continue)
///     }
/// }
/// ```
pub trait EventHandler: Send + Sync + 'static {
    /// An invite was created (Manage Server; test servers only).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_invite_create(&self, _client: &rappin::Client, _event: &rappin::InviteEvent) { tracing::debug!("on_invite_create received"); }
    /// }
    /// ```
    async fn on_invite_create(&self, _client: &Client, _event: &crate::InviteEvent) {}

    /// An invite was deleted (Manage Server; test servers only).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_invite_delete(&self, _client: &rappin::Client, _event: &rappin::InviteEvent) { tracing::debug!("on_invite_delete received"); }
    /// }
    /// ```
    async fn on_invite_delete(&self, _client: &Client, _event: &crate::InviteEvent) {}

    /// Fired when the bot has connected, verified its token, and synced commands.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_ready(&self, _client: &rappin::Client, profile: &rappin::BotProfile) { println!("Ready as {}", profile.id); }
    /// }
    /// ```
    async fn on_ready(&self, _client: &Client, _bot: &BotProfile) {}

    /// Fired on any incoming message that wasn't skipped (e.g. by bot filters).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_message(&self, _ctx: &rappin::MessageContext) { tracing::debug!("on_message received"); }
    /// }
    /// ```
    async fn on_message(&self, _ctx: &MessageContext) {}
    /// Fallible server-message listener, after command dispatch. The default calls
    /// the legacy on_message observer once. Handled skips remaining listeners;
    /// errors use BotBuilder::on_message_failure and retain checkpoint barriers.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::{EventHandler, MessageContext, MessageDisposition, Result};
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_message_result(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
    ///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
    ///         Ok(MessageDisposition::Continue)
    ///     }
    /// }
    /// ```
    async fn on_message_result(
        &self,
        ctx: &MessageContext,
    ) -> crate::Result<super::MessageDisposition> {
        self.on_message(ctx).await;
        Ok(super::MessageDisposition::Continue)
    }
    /// Incoming DM; separate from server message/command dispatch.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_dm(&self, ctx: &rappin::MessageContext) -> rappin::Result<()> { ctx.reply("Received your DM").await?; Ok(()) }
    /// }
    /// ```
    async fn on_dm(&self, _ctx: &MessageContext) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when an interaction (such as a button click) occurs.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_interaction(&self, _ctx: &rappin::InteractionContext) { tracing::debug!("on_interaction received"); }
    /// }
    /// ```
    async fn on_interaction(&self, _ctx: &InteractionContext) {}

    /// Fired when a new user joins a server the bot is in.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_member_join(&self, _client: &rappin::Client, _event: &rappin::MemberJoinEvent) { tracing::debug!("on_member_join received"); }
    /// }
    /// ```
    async fn on_member_join(&self, _client: &Client, _event: &MemberJoinEvent) {}

    /// Fired when a user leaves or is kicked/banned from a server.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_member_leave(&self, _client: &rappin::Client, _event: &rappin::MemberLeaveEvent) { tracing::debug!("on_member_leave received"); }
    /// }
    /// ```
    async fn on_member_leave(&self, _client: &Client, _event: &MemberLeaveEvent) {}

    /// Fired when a reaction is added to a message.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_reaction_add(&self, _client: &rappin::Client, _event: &rappin::ReactionEvent) { tracing::debug!("on_reaction_add received"); }
    /// }
    /// ```
    async fn on_reaction_add(&self, _client: &Client, _event: &ReactionEvent) {}

    /// Fired when a reaction is removed from a message.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_reaction_remove(&self, _client: &rappin::Client, _event: &rappin::ReactionEvent) { tracing::debug!("on_reaction_remove received"); }
    /// }
    /// ```
    async fn on_reaction_remove(&self, _client: &Client, _event: &ReactionEvent) {}

    /// Fired when a user connects to a voice channel.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_voice_join(&self, _client: &rappin::Client, _event: &rappin::VoiceEvent) { tracing::debug!("on_voice_join received"); }
    /// }
    /// ```
    async fn on_voice_join(&self, _client: &Client, _event: &VoiceEvent) {}

    /// Fired when a user leaves a voice channel.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_voice_leave(&self, _client: &rappin::Client, _event: &rappin::VoiceEvent) { tracing::debug!("on_voice_leave received"); }
    /// }
    /// ```
    async fn on_voice_leave(&self, _client: &Client, _event: &VoiceEvent) {}

    /// Fired when a developer dashboard test event is received.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_test(&self, _client: &rappin::Client, _data: &serde_json::Value) { tracing::debug!("on_test received"); }
    /// }
    /// ```
    async fn on_test(&self, _client: &Client, _data: &serde_json::Value) {}

    /// Fired when an unhandled, unrecognized, or newly added event type is received from Yapin'.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_unknown_event(&self, _client: &rappin::Client, kind: &str, _data: &serde_json::Value) { println!("Unknown event: {kind}"); }
    /// }
    /// ```
    async fn on_unknown_event(&self, _client: &Client, _kind: &str, _data: &serde_json::Value) {}
}

/// A default no-op event handler.
/// # Example
///
/// ```rust
/// use rappin::bot::handler::NoopEventHandler;
/// use rappin::EventHandler;
/// fn install<H: EventHandler>(handler: H) {}
/// install(NoopEventHandler);
/// ```
pub struct NoopEventHandler;

#[async_trait]
impl EventHandler for NoopEventHandler {}
