//! Original message/interaction contexts with shared API, reply helpers and authenticated invocation provenance.
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
/// # Example
///
/// ```no_run
/// use rappin::{MessageContext, Result};
/// async fn status(ctx: MessageContext) -> Result<()> {
///     println!("Message {} by {} in {}", ctx.message_id(), ctx.author_id(), ctx.channel_id());
///     ctx.reply("Ready").await?;
///     Ok(())
/// }
/// ```
pub struct MessageContext {
    /// Shared authenticated client for this dispatch.
    pub client: Arc<Client>,
    /// Original message identity and scope; changing it does not create checked-invocation authority.
    pub event: MessageEvent,
    /// Matched local command name, if dispatch found one.
    pub command: Option<String>,
    /// Raw argument text after the matched command path.
    pub args: String,
    /// Decoded tokens when quoting is valid; legacy whitespace fallback on malformed input. Use Arguments/FromArgs for validated parsing.
    pub argv: Vec<String>,
    /// Control handle when constructed by a running bot.
    pub bot: Option<BotHandle>,
}

impl MessageContext {
    /// Retrieve process-local application state registered on the bot builder.
    ///
    /// ```rust
    /// fn settings(ctx: &rappin::MessageContext) -> rappin::Result<std::sync::Arc<String>> {
    ///     ctx.state::<String>()
    /// }
    /// ```
    pub fn state<T: Send + Sync + 'static>(&self) -> Result<Arc<T>> {
        self.bot()
            .ok_or_else(|| crate::Error::Config("context has no bot state".into()))?
            .state::<T>()
    }
    /// Active consumer operation key. None for legacy explicit dispatch and
    /// outside the authenticated task. This is metadata, never permission proof.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     let id = ctx.operation_id();
    ///     // A local journal label only; it does not make remote requests idempotent.
    ///     println!("Operation: {:?}", id);
    /// }
    /// ```
    pub fn operation_id(&self) -> Option<super::OperationId> {
        super::invocation::operation_id(self)
    }

    /// Register before posting a question, then await the returned collector after sending it.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn waiter(ctx: &MessageContext) {
    ///     let builder = ctx.replies();
    ///     // Register the waiter before posting a prompt to avoid a response race.
    /// }
    /// ```
    pub fn replies(&self) -> crate::collectors::CollectorBuilder<MessageEvent> {
        self.client
            .events()
            .collect()
            .channel(self.channel_id())
            .author(self.author_id())
    }
    /// Collect matching component events from the running bot's event hub.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn waiter(ctx: &MessageContext, message: uuid::Uuid) {
    ///     let builder = ctx.component_events(message);
    ///     // Register the waiter before posting a prompt to avoid a response race.
    /// }
    /// ```
    pub fn component_events(
        &self,
        message: Uuid,
    ) -> crate::collectors::CollectorBuilder<InteractionEvent> {
        self.client
            .events()
            .collect()
            .channel(self.channel_id())
            .author(self.author_id())
            .message(message)
    }
    /// Construct a context for direct helper use. Checked invocation still requires the original framework-authenticated task and cannot be enabled by this constructor.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, MessageContext, MessageEvent};
    /// use std::sync::Arc;
    /// fn context(client: Arc<Client>, event: MessageEvent) -> MessageContext {
    ///     MessageContext::new(client, event, None, String::new(), None)
    /// }
    /// ```
    pub fn new(
        client: Arc<Client>,
        event: MessageEvent,
        command: Option<String>,
        args: String,
        bot: Option<BotHandle>,
    ) -> Self {
        let argv = crate::commands::tokenize(&args)
            .map(|tokens| tokens.into_iter().map(|token| token.value).collect())
            .unwrap_or_else(|_| args.split_whitespace().map(String::from).collect());
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
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) { if let Some(handle) = ctx.bot() { println!("Draining: {}", handle.is_draining()); } }
    /// ```
    pub fn bot(&self) -> Option<&BotHandle> {
        self.bot.as_ref()
    }

    /// Access the bot's in-memory cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) { let cache = ctx.cache().clone(); }
    /// ```
    pub fn cache(&self) -> &crate::cache::Cache {
        self.client.cache()
    }

    /// Checks if this command invocation hits the specified cooldown.
    ///
    /// Returns `Some(remaining_duration)` if rate limited, or `None` if allowed.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{MessageContext, cooldown::Cooldown};
    /// async fn admit(ctx: &MessageContext, cooldown: &Cooldown) -> bool {
    ///     // A successful check consumes the configured bucket.
    ///     ctx.check_cooldown(cooldown).await.is_none()
    /// }
    /// ```
    pub async fn check_cooldown(
        &self,
        cooldown: &crate::cooldown::Cooldown,
    ) -> Option<std::time::Duration> {
        cooldown.check(self).await
    }

    /// ID of the message.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     println!("Message: {}", ctx.message_id());
    /// }
    /// ```
    pub fn message_id(&self) -> Uuid {
        self.event.id
    }

    /// ID of the channel where the message was sent.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     println!("Channel: {}", ctx.channel_id());
    /// }
    /// ```
    pub fn channel_id(&self) -> Uuid {
        self.event.channel_id
    }

    /// Server ID if this message was sent in a server channel.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     if let Some(server) = ctx.server_id() { println!("Server: {server}"); }
    /// }
    /// ```
    pub fn server_id(&self) -> Option<Uuid> {
        self.event.server_id
    }

    /// Author user ID.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     println!("Requester: {}", ctx.author_id());
    /// }
    /// ```
    pub fn author_id(&self) -> Uuid {
        self.event.author_id
    }

    /// Text content of the message.
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     let normalized = ctx.content().trim();
    ///     assert!(normalized.len() <= ctx.content().len());
    /// }
    /// ```
    pub fn content(&self) -> &str {
        self.event.content.as_deref().unwrap_or_default()
    }

    /// Author mention string (`<@author_id>`).
    /// # Example
    ///
    /// ```rust
    /// use rappin::MessageContext;
    /// fn inspect(ctx: &MessageContext) {
    ///     let mention = ctx.author_mention();
    ///     println!("{mention}");
    /// }
    /// ```
    pub fn author_mention(&self) -> String {
        format!("<@{}>", self.event.author_id)
    }

    /// Replies to this message quoting it (`reply_to`).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::*;
    ///
    /// async fn on_ping(ctx: MessageContext) -> Result<()> {
    ///     ctx.reply("pong! 🏓").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reply(&self, content: impl Into<String>) -> Result<Message> {
        let msg = CreateMessage::reply(content, self.event.id);
        self.client.send_message(self.event.channel_id, &msg).await
    }

    /// Replies with a fully customized message (e.g. with embeds or buttons).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{MessageContext, Result};
    /// async fn handle(ctx: &MessageContext) -> Result<()> {
    ///     let receipt = ctx.reply_with(rappin::CreateMessage::new("Rich reply").embed(rappin::Embed::new().title("Status"))).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reply_with(&self, mut message: CreateMessage) -> Result<Message> {
        if message.reply_to.is_none() {
            message.reply_to = Some(self.event.id);
        }
        self.client
            .send_message(self.event.channel_id, &message)
            .await
    }

    /// Posts a message into the same channel without quoting.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::*;
    ///
    /// async fn on_hello(ctx: MessageContext) -> Result<()> {
    ///     ctx.say("Hello everyone in the channel!").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn say(&self, content: impl Into<String>) -> Result<Message> {
        let msg = CreateMessage::new(content);
        self.client.send_message(self.event.channel_id, &msg).await
    }

    /// Posts a Components V2 layout message in the channel.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{MessageContext, Result};
    /// async fn handle(ctx: &MessageContext) -> Result<()> {
    ///     let receipt = ctx.send_layout([rappin::Container::new().add(rappin::TextDisplay::new("Status"))]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_layout(
        &self,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<Message> {
        self.client
            .send_layout(self.event.channel_id, components, Some(self.event.id))
            .await
    }

    /// Adds a reaction to this message.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{MessageContext, Result};
    /// async fn handle(ctx: &MessageContext) -> Result<()> {
    ///     let receipt = ctx.react("✅").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn react(&self, emoji: &str) -> Result<()> {
        self.client
            .add_reaction(self.event.channel_id, self.event.id, emoji)
            .await
    }

    /// Deletes this message.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{MessageContext, Result};
    /// async fn handle(ctx: &MessageContext) -> Result<()> {
    ///     let receipt = ctx.delete().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete(&self) -> Result<()> {
        self.client
            .delete_message(self.event.channel_id, self.event.id)
            .await
    }

    /// Sends an interactive multi-page embed message.
    ///
    /// Returns the sent `Message` and the `Paginator` helper.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::*;
    ///
    /// async fn show_help(ctx: MessageContext) -> Result<()> {
    ///     let pages = vec![
    ///         Embed::new().title("Page 1").description("Commands"),
    ///         Embed::new().title("Page 2").description("Configuration"),
    ///     ];
    ///     let (_msg, paginator) = ctx.send_paginated(pages).await?;
    ///     Ok(())
    /// }
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::*;
    ///
    /// async fn ask_reset(ctx: MessageContext) -> Result<()> {
    ///     let (_msg, dialog) = ctx.send_confirm("Are you sure you want to reset settings?").await?;
    ///     Ok(())
    /// }
    /// ```
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
/// # Example
///
/// ```no_run
/// use rappin::{InteractionContext, Result};
/// async fn clicked(ctx: InteractionContext) -> Result<()> {
///     if ctx.custom_id() == "confirm" { ctx.ephemeral("Confirmed").await?; }
///     Ok(())
/// }
/// ```
pub struct InteractionContext {
    /// Typed selection or modal values delivered by the original event.
    pub values: Option<crate::InteractionValues>,
    /// Original slash command name when supplied by Yapin.
    pub command: Option<String>,
    /// Original slash command argument text.
    pub args: Option<String>,
    /// Parent authenticated API client.
    pub client: Arc<Client>,
    /// Original interaction, including its sensitive callback credential.
    pub event: InteractionEvent,
    /// Bot control handle when dispatched by the framework.
    pub bot: Option<BotHandle>,
}

impl InteractionContext {
    /// Update the original message, including explicit field clearing.
    ///
    /// ```no_run
    /// async fn clear(ctx: &rappin::InteractionContext) -> rappin::Result<()> {
    ///     ctx.update_with(&rappin::EditMessage::new().clear_components()).await
    /// }
    /// ```
    pub async fn update_with(&self, patch: &crate::EditMessage) -> Result<()> {
        crate::ApiExt::update_interaction(self, &self.event, patch).await
    }

    /// Reply with a complete message builder through this interaction.
    ///
    /// ```no_run
    /// async fn reply(ctx: &rappin::InteractionContext) -> rappin::Result<()> {
    ///     ctx.reply_with(rappin::CreateMessage::new("Hello")).await
    /// }
    /// ```
    pub async fn reply_with(&self, body: CreateMessage) -> Result<()> {
        self.respond(crate::InteractionCallbackType::Reply, &body)
            .await
    }
    #[cfg(feature = "hot-reload")]
    pub(crate) fn data(&self) -> crate::UiInteraction {
        crate::UiInteraction {
            interaction: self.event.clone(),
            values: self.values.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
        }
    }

    /// Parse command arguments with the same parser used by message commands.
    ///
    /// ```rust
    /// fn no_arguments(ctx: &rappin::InteractionContext) -> rappin::Result<()> {
    ///     ctx.parse::<()>()
    /// }
    /// ```
    pub fn parse<A: crate::FromArgs>(&self) -> Result<A> {
        let mut args = crate::Arguments::new(self.args.as_deref().unwrap_or(""))?;
        let value = A::parse(&mut args)?;
        args.finish()?;
        Ok(value)
    }

    /// Open a modal using this interaction's tracked initial response.
    ///
    /// ```no_run
    /// async fn ask(ctx: &rappin::InteractionContext) -> rappin::Result<()> {
    ///     ctx.show_modal(&rappin::Modal::new("feedback", "Feedback").input(rappin::TextInput::short("name", "Name"))).await
    /// }
    /// ```
    pub async fn show_modal(&self, modal: &crate::Modal) -> Result<()> {
        crate::ApiExt::show_modal(self, &self.event, modal).await
    }

    /// Send a rich reply, ephemeral reply or message update through the original
    /// callback identity. Reply ownership and deadlines are shared by clones.
    ///
    /// ```no_run
    /// async fn reply(ctx: &rappin::InteractionContext) -> rappin::Result<()> {
    ///     ctx.respond(rappin::InteractionCallbackType::Reply, &rappin::CreateMessage::new("Hello")).await
    /// }
    /// ```
    pub async fn respond(
        &self,
        kind: crate::InteractionCallbackType,
        body: &CreateMessage,
    ) -> Result<()> {
        crate::ApiExt::respond(self, &self.event, kind, body).await
    }
    /// Retrieve process-local application state registered on the bot builder.
    ///
    /// ```rust
    /// fn settings(ctx: &rappin::InteractionContext) -> rappin::Result<std::sync::Arc<String>> {
    ///     ctx.state::<String>()
    /// }
    /// ```
    pub fn state<T: Send + Sync + 'static>(&self) -> Result<Arc<T>> {
        self.bot()
            .ok_or_else(|| crate::Error::Config("context has no bot state".into()))?
            .state::<T>()
    }
    async fn respond_callback(&self, callback: &InteractionCallback) -> Result<()> {
        self.client.track_interaction(&self.event).await?;
        self.client
            .respond_interaction(&self.event.id, &self.event.token, callback)
            .await
    }

    /// Read local acknowledgement tracking for this interaction.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Result};
    /// async fn clicked(ctx: &InteractionContext) -> Result<()> {
    ///     let result = ctx.response_state().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn response_state(&self) -> Result<crate::interactions::InteractionState> {
        self.client.interaction_state(&self.event).await
    }
    /// Alias for silent acknowledgement; follow with an eligible callback as documented.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Result};
    /// async fn clicked(ctx: &InteractionContext) -> Result<()> {
    ///     let result = ctx.defer().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn defer(&self) -> Result<()> {
        self.ack().await
    }
    /// Construct a context using the original verified interaction and shared client.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, InteractionContext, InteractionEvent};
    /// use std::sync::Arc;
    /// fn context(client: Arc<Client>, event: InteractionEvent) -> InteractionContext {
    ///     InteractionContext::new(client, event, None)
    /// }
    /// ```
    pub fn new(
        client: Arc<Client>,
        event: impl Into<crate::UiInteraction>,
        bot: Option<BotHandle>,
    ) -> Self {
        let data = event.into();
        Self {
            client,
            event: data.interaction,
            bot,
            values: data.values,
            command: data.command,
            args: data.args,
        }
    }

    /// Access the bot handle to manage cogs or bot state dynamically.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) { if let Some(handle) = ctx.bot() { println!("Draining: {}", handle.is_draining()); } }
    /// ```
    pub fn bot(&self) -> Option<&BotHandle> {
        self.bot.as_ref()
    }

    /// Access the bot's in-memory cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) { let cache = ctx.cache().clone(); }
    /// ```
    pub fn cache(&self) -> &crate::cache::Cache {
        self.client.cache()
    }

    /// Updates the interaction message with a new page from a paginator.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, paginator::Paginator, Result};
    /// async fn next_page(ctx: &InteractionContext, pages: &Paginator) -> Result<()> {
    ///     ctx.update_paginator(pages, 1).await
    /// }
    /// ```
    pub async fn update_paginator(
        &self,
        paginator: &crate::paginator::Paginator,
        page: usize,
    ) -> Result<()> {
        let callback = paginator.build_update(page);
        self.respond_callback(&callback).await
    }

    /// The interaction ID.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     println!("Interaction: {}", ctx.interaction_id());
    /// }
    /// ```
    pub fn interaction_id(&self) -> &str {
        &self.event.id
    }

    /// The interaction security token.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     let callback_credential = ctx.token();
    ///     // Supply only to the matching callback endpoint; never log or persist it.
    ///     assert!(!callback_credential.is_empty());
    /// }
    /// ```
    pub fn token(&self) -> &str {
        &self.event.token
    }

    /// The custom_id on the clicked button.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     if ctx.custom_id() == "confirm" { println!("Confirmation route"); }
    /// }
    /// ```
    pub fn custom_id(&self) -> &str {
        self.event.custom_id.as_deref().unwrap_or_default()
    }

    /// User ID of the person who clicked.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     if let Some(user) = ctx.user_id() { println!("Requester: {user}"); }
    /// }
    /// ```
    pub fn user_id(&self) -> Option<Uuid> {
        self.event.user_id
    }

    /// Channel ID where the button was clicked.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     if let Some(channel) = ctx.channel_id() { println!("Channel: {channel}"); }
    /// }
    /// ```
    pub fn channel_id(&self) -> Option<Uuid> {
        self.event.channel_id
    }

    /// Server ID where the button was clicked.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     if let Some(server) = ctx.server_id() { println!("Server: {server}"); }
    /// }
    /// ```
    pub fn server_id(&self) -> Option<Uuid> {
        self.event.server_id
    }

    /// Mention string for the interacting user.
    /// # Example
    ///
    /// ```rust
    /// use rappin::InteractionContext;
    /// fn inspect(ctx: &InteractionContext) {
    ///     let mention = ctx.user_mention();
    ///     println!("{mention}");
    /// }
    /// ```
    pub fn user_mention(&self) -> String {
        if let Some(uid) = self.event.user_id {
            format!("<@{}>", uid)
        } else {
            String::new()
        }
    }

    /// Posts a new public reply in the channel.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Result};
    /// async fn clicked(ctx: &InteractionContext) -> Result<()> {
    ///     let result = ctx.reply("Public reply").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reply(&self, content: impl Into<String>) -> Result<()> {
        let callback = InteractionCallback::reply(content);
        self.respond_callback(&callback).await
    }

    /// Sends an ephemeral message visible only to the person who clicked.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::*;
    ///
    /// async fn on_secret_click(ctx: InteractionContext) -> Result<()> {
    ///     ctx.ephemeral("🤫 Only you can see this message!").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn ephemeral(&self, content: impl Into<String>) -> Result<()> {
        let callback = InteractionCallback::ephemeral(content);
        self.respond_callback(&callback).await
    }

    /// Updates the message the clicked button is on.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::*;
    ///
    /// async fn on_vote(ctx: InteractionContext) -> Result<()> {
    ///     ctx.update("✅ Thank you for voting!").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn update(&self, content: impl Into<String>) -> Result<()> {
        let callback = InteractionCallback::update(content);
        self.respond_callback(&callback).await
    }

    /// Acknowledges the interaction silently ("got it").
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Result};
    /// async fn clicked(ctx: &InteractionContext) -> Result<()> {
    ///     let result = ctx.ack().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn ack(&self) -> Result<()> {
        let callback = InteractionCallback::ack();
        self.respond_callback(&callback).await
    }

    /// Replies with a Components V2 layout.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Result};
    /// async fn clicked(ctx: &InteractionContext) -> Result<()> {
    ///     let result = ctx.reply_layout([rappin::Container::new().add(rappin::TextDisplay::new("Ready"))]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reply_layout(
        &self,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<()> {
        let callback = InteractionCallback::reply_layout(components);
        self.respond_callback(&callback).await
    }

    /// Updates the message with a Components V2 layout.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Result};
    /// async fn clicked(ctx: &InteractionContext) -> Result<()> {
    ///     let result = ctx.update_layout([rappin::Container::new().add(rappin::TextDisplay::new("Updated"))]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn update_layout(
        &self,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<()> {
        let callback = InteractionCallback::update_layout(components);
        self.respond_callback(&callback).await
    }
}
