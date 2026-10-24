use async_trait::async_trait;

use crate::bot::context::{InteractionContext, MessageContext};
use crate::client::Client;
use crate::models::{
    BotProfile, MemberJoinEvent, MemberLeaveEvent, ReactionEvent, VoiceEvent,
};

/// Trait for receiving dispatched Yapin' events.
#[async_trait]
pub trait EventHandler: Send + Sync + 'static {
    /// Fired when the bot has connected, verified its token, and synced commands.
    async fn on_ready(&self, _client: &Client, _bot: &BotProfile) {}

    /// Fired on any incoming message that wasn't skipped (e.g. by bot filters).
    async fn on_message(&self, _ctx: &MessageContext) {}

    /// Fired when an interaction (such as a button click) occurs.
    async fn on_interaction(&self, _ctx: &InteractionContext) {}

    /// Fired when a new user joins a server the bot is in.
    async fn on_member_join(&self, _client: &Client, _event: &MemberJoinEvent) {}

    /// Fired when a user leaves or is kicked/banned from a server.
    async fn on_member_leave(&self, _client: &Client, _event: &MemberLeaveEvent) {}

    /// Fired when a reaction is added to a message.
    async fn on_reaction_add(&self, _client: &Client, _event: &ReactionEvent) {}

    /// Fired when a reaction is removed from a message.
    async fn on_reaction_remove(&self, _client: &Client, _event: &ReactionEvent) {}

    /// Fired when a user connects to a voice channel.
    async fn on_voice_join(&self, _client: &Client, _event: &VoiceEvent) {}

    /// Fired when a user leaves a voice channel.
    async fn on_voice_leave(&self, _client: &Client, _event: &VoiceEvent) {}

    /// Fired when a developer dashboard test event is received.
    async fn on_test(&self, _client: &Client, _data: &serde_json::Value) {}
}

/// A default no-op event handler.
pub struct NoopEventHandler;

#[async_trait]
impl EventHandler for NoopEventHandler {}

