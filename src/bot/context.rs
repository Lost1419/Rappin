use std::sync::Arc;

use uuid::Uuid;

use crate::client::Client;
use crate::error::Result;
use crate::models::component::Component;
use crate::models::interaction::InteractionCallback;
use crate::models::message::{CreateMessage, Message};
use crate::models::{InteractionEvent, MessageEvent};

// Forward declaration of BotHandle
use super::BotHandle;

/// Context provided to command and message handlers.
#[derive(Clone)]
pub struct MessageContext {
    pub client: Arc<Client>,
    pub event: MessageEvent,
    pub command: Option<String>,
    pub args: String,
    pub argv: Vec<String>,
    pub bot: Option<BotHandle>,
}

impl MessageContext {
    pub fn new(
        client: Arc<Client>,
        event: MessageEvent,
        command: Option<String>,
        args: String,
        bot: Option<BotHandle>,
    ) -> Self {
        let argv = args.split_whitespace().map(String::from).collect();
        Self {
            client,
            event,
            command,
            args,
            argv,
            bot,
        }
    }

    /// Access the bot handle to manage cogs or bot state dynamically.
    pub fn bot(&self) -> Option<&BotHandle> {
        self.bot.as_ref()
    }

    /// Access the bot's in-memory cache.
    pub fn cache(&self) -> &crate::cache::Cache {
        self.client.cache()
    }

    /// Checks if this command invocation hits the specified cooldown.
    ///
    /// Returns `Some(remaining_duration)` if rate limited, or `None` if allowed.
    pub async fn check_cooldown(
        &self,
        cooldown: &crate::cooldown::Cooldown,
    ) -> Option<std::time::Duration> {
        cooldown.check(self).await
    }

    /// ID of the message.
    pub fn message_id(&self) -> Uuid {
        self.event.id
    }

    /// ID of the channel where the message was sent.
    pub fn channel_id(&self) -> Uuid {
        self.event.channel_id
    }

    /// Server ID if this message was sent in a server channel.
    pub fn server_id(&self) -> Option<Uuid> {
        self.event.server_id
    }

    /// Author user ID.
    pub fn author_id(&self) -> Uuid {
        self.event.author_id
    }

    /// Text content of the message.
    pub fn content(&self) -> &str {
        self.event.content.as_deref().unwrap_or_default()
    }

    /// Author mention string (`<@author_id>`).
    pub fn author_mention(&self) -> String {
        format!("<@{}>", self.event.author_id)
    }

    /// Replies to this message quoting it (`reply_to`).
    pub async fn reply(&self, content: impl Into<String>) -> Result<Message> {
        let msg = CreateMessage::reply(content, self.event.id);
        self.client.send_message(self.event.channel_id, &msg).await
    }

    /// Replies with a fully customized message (e.g. with embeds or buttons).
    pub async fn reply_with(&self, mut message: CreateMessage) -> Result<Message> {
        if message.reply_to.is_none() {
            message.reply_to = Some(self.event.id);
        }
        self.client
            .send_message(self.event.channel_id, &message)
            .await
    }

    /// Posts a message into the same channel without quoting.
    pub async fn say(&self, content: impl Into<String>) -> Result<Message> {
        let msg = CreateMessage::new(content);
        self.client.send_message(self.event.channel_id, &msg).await
    }

    /// Posts a Components V2 layout message in the channel.
    pub async fn send_layout(
        &self,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<Message> {
        self.client
            .send_layout(self.event.channel_id, components, Some(self.event.id))
            .await
    }

    /// Adds a reaction to this message.
    pub async fn react(&self, emoji: &str) -> Result<()> {
        self.client
            .add_reaction(self.event.channel_id, self.event.id, emoji)
            .await
    }

    /// Deletes this message.
    pub async fn delete(&self) -> Result<()> {
        self.client
            .delete_message(self.event.channel_id, self.event.id)
            .await
    }

    /// Sends an interactive multi-page embed message.
    ///
    /// Returns the sent `Message` and the `Paginator` helper.
    pub async fn send_paginated(
        &self,
        embeds: impl IntoIterator<Item = crate::models::embed::Embed>,
    ) -> Result<(Message, crate::paginator::Paginator)> {
        let paginator = crate::paginator::Paginator::new(embeds);
        let msg = paginator.build_message(0);
        let sent = self.reply_with(msg).await?;
        Ok((sent, paginator))
    }

    /// Sends an interactive confirmation prompt with Confirm and Cancel buttons.
    ///
    /// Returns the sent `Message` and the `ConfirmDialog` helper.
    pub async fn send_confirm(
        &self,
        content: impl Into<String>,
    ) -> Result<(Message, crate::paginator::ConfirmDialog)> {
        let dialog = crate::paginator::ConfirmDialog::new(content);
        let msg = dialog.build_message();
        let sent = self.reply_with(msg).await?;
        Ok((sent, dialog))
    }
}

/// Context provided to button click / interaction handlers.
#[derive(Clone)]
pub struct InteractionContext {
    pub client: Arc<Client>,
    pub event: InteractionEvent,
    pub bot: Option<BotHandle>,
}

impl InteractionContext {
    pub fn new(client: Arc<Client>, event: InteractionEvent, bot: Option<BotHandle>) -> Self {
        Self { client, event, bot }
    }

    /// Access the bot handle to manage cogs or bot state dynamically.
    pub fn bot(&self) -> Option<&BotHandle> {
        self.bot.as_ref()
    }

    /// Access the bot's in-memory cache.
    pub fn cache(&self) -> &crate::cache::Cache {
        self.client.cache()
    }

    /// Updates the interaction message with a new page from a paginator.
    pub async fn update_paginator(
        &self,
        paginator: &crate::paginator::Paginator,
        page: usize,
    ) -> Result<()> {
        let callback = paginator.build_update(page);
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }

    /// The interaction ID.
    pub fn interaction_id(&self) -> &str {
        &self.event.id
    }

    /// The interaction security token.
    pub fn token(&self) -> &str {
        &self.event.token
    }

    /// The custom_id on the clicked button.
    pub fn custom_id(&self) -> &str {
        self.event.custom_id.as_deref().unwrap_or_default()
    }

    /// User ID of the person who clicked.
    pub fn user_id(&self) -> Option<Uuid> {
        self.event.user_id
    }

    /// Channel ID where the button was clicked.
    pub fn channel_id(&self) -> Option<Uuid> {
        self.event.channel_id
    }

    /// Server ID where the button was clicked.
    pub fn server_id(&self) -> Option<Uuid> {
        self.event.server_id
    }

    /// Mention string for the interacting user.
    pub fn user_mention(&self) -> String {
        if let Some(uid) = self.event.user_id {
            format!("<@{}>", uid)
        } else {
            String::new()
        }
    }

    /// Posts a new public reply in the channel.
    pub async fn reply(&self, content: impl Into<String>) -> Result<()> {
        let callback = InteractionCallback::reply(content);
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }

    /// Sends an ephemeral message visible only to the person who clicked.
    pub async fn ephemeral(&self, content: impl Into<String>) -> Result<()> {
        let callback = InteractionCallback::ephemeral(content);
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }

    /// Updates the message the clicked button is on.
    pub async fn update(&self, content: impl Into<String>) -> Result<()> {
        let callback = InteractionCallback::update(content);
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }

    /// Acknowledges the interaction silently ("got it").
    pub async fn ack(&self) -> Result<()> {
        let callback = InteractionCallback::ack();
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }

    /// Replies with a Components V2 layout.
    pub async fn reply_layout(
        &self,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<()> {
        let callback = InteractionCallback::reply_layout(components);
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }

    /// Updates the message with a Components V2 layout.
    pub async fn update_layout(
        &self,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<()> {
        let callback = InteractionCallback::update_layout(components);
        self.client
            .respond_interaction(&self.event.id, &self.event.token, &callback)
            .await
    }
}
