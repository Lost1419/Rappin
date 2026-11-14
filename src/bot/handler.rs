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
///     async fn on_message(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
///         Ok(MessageDisposition::Continue)
///     }
/// }
/// ```
pub trait EventHandler: Send + Sync + 'static {
    /// Process a resource lifecycle event. Errors retain the event's checkpoint barrier.
    ///
    /// ```rust
    /// struct Observer;
    /// #[async_trait::async_trait]
    /// impl rappin::EventHandler for Observer {
    ///     async fn on_resource_event(&self, _client: &rappin::Client, event: &rappin::ResourceEvent) -> rappin::Result<()> {
    ///         println!("{} {}", event.kind.as_str(), event.id);
    ///         Ok(())
    ///     }
    /// }
    /// ```
    async fn on_resource_event(
        &self,
        client: &Client,
        event: &crate::ResourceEvent,
    ) -> crate::Result<()> {
        self.on_unknown_event(client, event.kind.as_str(), &event.data)
            .await?;
        Ok(())
    }

    /// An invite was created (Manage Server; test servers only).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_invite_create(&self, _client: &rappin::Client, _event: &rappin::InviteEvent) -> rappin::Result<()> { tracing::debug!("on_invite_create received");  Ok(()) }
    /// }
    /// ```
    async fn on_invite_create(
        &self,
        _client: &Client,
        _event: &crate::InviteEvent,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// An invite was deleted (Manage Server; test servers only).
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_invite_delete(&self, _client: &rappin::Client, _event: &rappin::InviteEvent) -> rappin::Result<()> { tracing::debug!("on_invite_delete received");  Ok(()) }
    /// }
    /// ```
    async fn on_invite_delete(
        &self,
        _client: &Client,
        _event: &crate::InviteEvent,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when the bot has connected, verified its token, and synced commands.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_ready(&self, _client: &rappin::Client, profile: &rappin::BotProfile) -> rappin::Result<()> { println!("Ready as {}", profile.id);  Ok(()) }
    /// }
    /// ```
    async fn on_ready(&self, _client: &Client, _bot: &BotProfile) -> crate::Result<()> {
        Ok(())
    }

    /// Fallible server-message listener, after command dispatch. Returning Continue allows subsequent listeners. Handled skips remaining listeners;
    /// errors use BotBuilder::on_message_failure and retain checkpoint barriers.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::{EventHandler, MessageContext, MessageDisposition, Result};
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_message(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
    ///         if ctx.content() == "hello" { ctx.reply("Hello!").await?; }
    ///         Ok(MessageDisposition::Continue)
    ///     }
    /// }
    /// ```
    async fn on_message(&self, _ctx: &MessageContext) -> crate::Result<super::MessageDisposition> {
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
    ///     async fn on_interaction(&self, _ctx: &rappin::InteractionContext) -> rappin::Result<()> { tracing::debug!("on_interaction received");  Ok(()) }
    /// }
    /// ```
    async fn on_interaction(&self, _ctx: &InteractionContext) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a new user joins a server the bot is in.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_member_join(&self, _client: &rappin::Client, _event: &rappin::MemberJoinEvent) -> rappin::Result<()> { tracing::debug!("on_member_join received");  Ok(()) }
    /// }
    /// ```
    async fn on_member_join(
        &self,
        _client: &Client,
        _event: &MemberJoinEvent,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a user leaves or is kicked/banned from a server.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_member_leave(&self, _client: &rappin::Client, _event: &rappin::MemberLeaveEvent) -> rappin::Result<()> { tracing::debug!("on_member_leave received");  Ok(()) }
    /// }
    /// ```
    async fn on_member_leave(
        &self,
        _client: &Client,
        _event: &MemberLeaveEvent,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a reaction is added to a message.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_reaction_add(&self, _client: &rappin::Client, _event: &rappin::ReactionEvent) -> rappin::Result<()> { tracing::debug!("on_reaction_add received");  Ok(()) }
    /// }
    /// ```
    async fn on_reaction_add(&self, _client: &Client, _event: &ReactionEvent) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a reaction is removed from a message.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_reaction_remove(&self, _client: &rappin::Client, _event: &rappin::ReactionEvent) -> rappin::Result<()> { tracing::debug!("on_reaction_remove received");  Ok(()) }
    /// }
    /// ```
    async fn on_reaction_remove(
        &self,
        _client: &Client,
        _event: &ReactionEvent,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a user connects to a voice channel.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_voice_join(&self, _client: &rappin::Client, _event: &rappin::VoiceEvent) -> rappin::Result<()> { tracing::debug!("on_voice_join received");  Ok(()) }
    /// }
    /// ```
    async fn on_voice_join(&self, _client: &Client, _event: &VoiceEvent) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a user leaves a voice channel.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_voice_leave(&self, _client: &rappin::Client, _event: &rappin::VoiceEvent) -> rappin::Result<()> { tracing::debug!("on_voice_leave received");  Ok(()) }
    /// }
    /// ```
    async fn on_voice_leave(&self, _client: &Client, _event: &VoiceEvent) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when a developer dashboard test event is received.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_test(&self, _client: &rappin::Client, _data: &serde_json::Value) -> rappin::Result<()> { tracing::debug!("on_test received");  Ok(()) }
    /// }
    /// ```
    async fn on_test(&self, _client: &Client, _data: &serde_json::Value) -> crate::Result<()> {
        Ok(())
    }

    /// Fired when an unhandled, unrecognized, or newly added event type is received from Yapin'.
    /// # Example
    ///
    /// ```rust
    /// use async_trait::async_trait;
    /// use rappin::EventHandler;
    /// struct Listener;
    /// #[async_trait]
    /// impl EventHandler for Listener {
    ///     async fn on_unknown_event(&self, _client: &rappin::Client, kind: &str, _data: &serde_json::Value) -> rappin::Result<()> { println!("Unknown event: {kind}");  Ok(()) }
    /// }
    /// ```
    async fn on_unknown_event(
        &self,
        _client: &Client,
        _kind: &str,
        _data: &serde_json::Value,
    ) -> crate::Result<()> {
        Ok(())
    }
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
