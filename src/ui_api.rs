//! # Example
//!
//! ```rust
//! use rappin::*;
//! use uuid::Uuid;
//! use reqwest::Method;
//! async fn example(api: &impl Api, channel: Uuid, message: Uuid) -> Result<()> {
//!     let response = api.get_ui_message(channel, message).await?;
//!     Ok(())
//! }
//! ```
//! Shared native/worker API 1.3 operations.
use crate::*;
use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;

#[async_trait::async_trait]
/// Extended message, select-menu and modal operations shared by native and worker API adapters. Receiving form/select values requires opt-in UI event routing.
///
/// # Example
///
/// ```rust
/// use rappin::*;
/// use uuid::Uuid;
/// use reqwest::Method;
/// async fn example(api: &impl Api, channel: Uuid, message: Uuid) -> Result<()> {
///     let response = api.get_ui_message(channel, message).await?;
///     Ok(())
/// }
/// ```
pub trait UiApiExt: Api {
    /// Read custom emojis used in one message, retaining platform image URLs and server-membership metadata. Platform scope, access and rollout still apply.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid, message: Uuid) -> Result<()> {
    ///     let response = api.get_message_emojis(channel, message).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_message_emojis(&self, channel: Uuid, message: Uuid) -> Result<MessageEmojis> {
        Ok(serde_json::from_value(
            self.request(
                Method::GET,
                &format!("/channels/{channel}/messages/{message}/emojis"),
                None,
            )
            .await?,
        )?)
    }
    /// Read a page of messages with lossless UI component trees. Limit must be 1 through 100; before and after are mutually exclusive.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid) -> Result<()> {
    ///     let response = api.get_ui_messages(channel, Some(25), None, None).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_ui_messages(
        &self,
        channel: Uuid,
        limit: Option<u32>,
        before: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<UiMessagesResponse> {
        if before.is_some() && after.is_some() {
            return Err(YappinError::validation(
                "cursor",
                "before and after are mutually exclusive",
            ));
        }
        let limit = limit.unwrap_or(50);
        if !(1..=100).contains(&limit) {
            return Err(YappinError::validation("limit", "must be 1..=100"));
        }
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("limit", &limit.to_string());
            if let Some(cursor) = before {
                query.append_pair("before", &cursor.to_rfc3339());
            }
            if let Some(cursor) = after {
                query.append_pair("after", &cursor.to_rfc3339());
            }
            format!("/channels/{channel}/messages?{}", query.finish())
        };
        Ok(serde_json::from_value(
            self.request(Method::GET, &path, None).await?,
        )?)
    }
    /// Read one message without discarding extended or unknown component fields.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid, message: Uuid) -> Result<()> {
    ///     let response = api.get_ui_message(channel, message).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_ui_message(&self, channel: Uuid, message: Uuid) -> Result<UiMessage> {
        let value = self
            .request(
                Method::GET,
                &format!("/channels/{channel}/messages/{message}"),
                None,
            )
            .await?;
        Ok(serde_json::from_value(value["message"].clone())?)
    }
    /// Validate and send an extended message body. This low-level helper does not assess requester authorization or application approval.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid) -> Result<()> {
    ///     let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
    ///     let body = UiMessageBody::new("Choose fruit").component(menu.row());
    ///     let response = api.send_ui_message(channel, &body).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn send_ui_message(&self, channel: Uuid, body: &UiMessageBody) -> Result<UiMessage> {
        body.validate(true)?;
        let mut value = json!(body);
        crate::utils::strip_proxy_metadata(&mut value);
        let response = self
            .request(
                Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(value),
            )
            .await?;
        Ok(serde_json::from_value(response["message"].clone())?)
    }
    /// Validate and edit an extended message body, preserving omission versus explicit null clearing. Reply targets cannot be changed by edits.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid, message: Uuid) -> Result<()> {
    ///     let body = UiMessageBody::new("Updated");
    ///     let response = api.edit_ui_message(channel, message, &body).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn edit_ui_message(
        &self,
        channel: Uuid,
        message: Uuid,
        body: &UiMessageBody,
    ) -> Result<UiMessage> {
        body.validate(false)?;
        let mut value = json!(body);
        crate::utils::strip_proxy_metadata(&mut value);
        let response = self
            .request(
                Method::PATCH,
                &format!("/channels/{channel}/messages/{message}"),
                Some(value),
            )
            .await?;
        Ok(serde_json::from_value(response["message"].clone())?)
    }
    /// Open a validated modal as an eligible initial interaction response. Opening from a modal submission is rejected; response deadlines and ownership are tracked by supporting adapters.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, event: &InteractionEvent) -> Result<()> {
    ///     let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Comment"));
    ///     let response = api.show_modal(event, &modal).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn show_modal(&self, event: &InteractionEvent, modal: &Modal) -> Result<()> {
        if event.kind.as_deref() == Some("modal_submit") {
            return Err(YappinError::validation(
                "modal",
                "cannot open a modal from a modal submission",
            ));
        }
        modal.validate()?;
        Api::track_interaction(self, event).await?;
        self.request(
            Method::POST,
            &crate::interactions::path(&event.id, &event.token),
            Some(json!({"type":"modal","modal":modal})),
        )
        .await?;
        Ok(())
    }
    /// Send an extended reply, ephemeral response or original-message update. Reply targets, invalid updates, component-bearing ephemeral replies and Ack payloads are rejected before sending. Use existing ack helpers for acknowledgement.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, event: &InteractionEvent) -> Result<()> {
    ///     let response = api.respond_ui(event, InteractionCallbackType::Ephemeral, &UiMessageBody::new("Recorded")).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn respond_ui(
        &self,
        event: &InteractionEvent,
        kind: InteractionCallbackType,
        body: &UiMessageBody,
    ) -> Result<()> {
        if body.reply_to.is_some() {
            return Err(YappinError::validation(
                "reply_to",
                "interaction callbacks do not accept reply targets",
            ));
        }
        if kind == InteractionCallbackType::Ack {
            return Err(YappinError::validation(
                "type",
                "use the existing acknowledgement API",
            ));
        }
        if kind == InteractionCallbackType::Update
            && (event.kind.as_deref() == Some("command")
                || event.kind.as_deref() == Some("modal_submit") && event.message_id.is_none())
        {
            return Err(YappinError::validation(
                "type",
                "this interaction has no original message to update",
            ));
        }
        if kind == InteractionCallbackType::Ephemeral
            && (body.components.is_some() || body.flags.is_some_and(|f| f & 32768 != 0))
        {
            return Err(YappinError::validation(
                "components",
                "ephemeral replies do not support components",
            ));
        }
        body.validate(kind != InteractionCallbackType::Update)?;
        Api::track_interaction(self, event).await?;
        let mut value = json!(body);
        value["type"] = json!(kind);
        crate::utils::strip_proxy_metadata(&mut value);
        self.request(
            Method::POST,
            &crate::interactions::path(&event.id, &event.token),
            Some(value),
        )
        .await?;
        Ok(())
    }
    /// Opt-in slash events. Existing set_commands leaves interactions omitted.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.set_interaction_commands(&[Command::new("feedback", "Open feedback form")], true).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn set_interaction_commands(
        &self,
        commands: &[Command],
        enabled: bool,
    ) -> Result<Vec<Command>> {
        crate::validation::validate_commands(commands)?;
        let value = self
            .request(
                Method::PUT,
                "/commands",
                Some(json!({"commands":commands,"interactions":enabled})),
            )
            .await?;
        Ok(serde_json::from_value(value["commands"].clone())?)
    }
}
impl<T: Api + ?Sized> UiApiExt for T {}

/// Extended context used only by opt-in UI event routing.
#[derive(Clone)]
#[non_exhaustive]
/// # Example
///
/// ```no_run
/// use rappin::{UiInteractionContext, UiMessageBody, InteractionCallbackType, Result};
/// async fn respond(ctx: UiInteractionContext) -> Result<()> {
///     ctx.respond_ui(InteractionCallbackType::Ephemeral, &UiMessageBody::new("Recorded")).await
/// }
/// ```
pub struct UiInteractionContext {
    /// Base interaction context sharing the parent client, response ownership and original interaction scope.
    pub context: InteractionContext,
    /// Extended values and command metadata from the original interaction event.
    pub data: UiInteraction,
}
impl std::ops::Deref for UiInteractionContext {
    type Target = InteractionContext;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[async_trait::async_trait]
impl Api for UiInteractionContext {
    async fn public_request(&self, path: &str) -> Result<Value> {
        Api::public_request(&self.context, path).await
    }
    async fn upload(&self, path: &str, body: Value, files: &[UploadFile]) -> Result<Value> {
        Api::upload(&self.context, path, body, files).await
    }
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<PermissionSnapshot> {
        Api::permission_snapshot(&self.context, server, actor, freshness).await
    }
    async fn track_interaction(&self, event: &InteractionEvent) -> Result<()> {
        self.context.client.track_interaction(event).await
    }
    fn sanitize(&self, value: Value) -> Value {
        Api::sanitize(&self.context, value)
    }
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        Api::request(&self.context, method, path, body).await
    }
    async fn request_once(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        Api::request_once(&self.context, method, path, body).await
    }
}
impl UiInteractionContext {
    /// Construct a context for a verified custom transport. Construction alone does not authenticate the event or establish checked-invocation task provenance.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn context(client: std::sync::Arc<rappin::Client>, data: rappin::UiInteraction) -> rappin::UiInteractionContext {
    ///     rappin::UiInteractionContext::new(client, data, None)
    /// }
    /// ```
    pub fn new(
        client: std::sync::Arc<Client>,
        data: UiInteraction,
        bot: Option<BotHandle>,
    ) -> Self {
        Self {
            context: InteractionContext::new(client, data.interaction.clone(), bot),
            data,
        }
    }
    /// Parse the original command arguments using FromArgs and reject leftover tokens. This does not interpret modal fields as command arguments.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Arguments, CommandError, FromArgs, UiInteractionContext, Result};
    /// struct Name(String);
    /// impl FromArgs for Name {
    ///     fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> { Ok(Self(args.required("name")?)) }
    /// }
    /// fn parse(ctx: &UiInteractionContext) -> Result<String> {
    ///     Ok(ctx.parse::<Name>()?.0)
    /// }
    /// ```
    pub fn parse<A: crate::FromArgs>(&self) -> Result<A> {
        let mut arguments = crate::Arguments::new(self.data.args.as_deref().unwrap_or(""))?;
        let parsed = A::parse(&mut arguments)?;
        arguments.finish()?;
        Ok(parsed)
    }
    /// Open a modal through the shared response/deadline coordinator. A modal requires an eligible initial response and cannot be opened from modal_submit.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{UiInteractionContext, Modal, TextInput, Result};
    /// async fn form(ctx: &UiInteractionContext) -> Result<()> {
    ///     let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Comment"));
    ///     ctx.show_modal(&modal).await
    /// }
    /// ```
    pub async fn show_modal(&self, modal: &Modal) -> Result<()> {
        self.context.show_modal(modal).await
    }
    /// Send an extended UI response through this context's original interaction identity and shared response ownership.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{UiInteractionContext, UiMessageBody, InteractionCallbackType, Result};
    /// async fn respond(ctx: &UiInteractionContext) -> Result<()> {
    ///     ctx.respond_ui(InteractionCallbackType::Ephemeral, &UiMessageBody::new("Recorded")).await
    /// }
    /// ```
    pub async fn respond_ui(
        &self,
        kind: InteractionCallbackType,
        body: &UiMessageBody,
    ) -> Result<()> {
        UiApiExt::respond_ui(self, &self.context.event, kind, body).await
    }
}
impl InteractionContext {
    /// Open a modal through the shared response/deadline coordinator. A modal requires an eligible initial response and cannot be opened from modal_submit.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{InteractionContext, Modal, TextInput, Result};
    /// async fn form(ctx: &InteractionContext) -> Result<()> {
    ///     let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Comment"));
    ///     ctx.show_modal(&modal).await
    /// }
    /// ```
    pub async fn show_modal(&self, modal: &Modal) -> Result<()> {
        UiApiExt::show_modal(self, &self.event, modal).await
    }
}
impl Client {
    /// Read custom emoji metadata from one scoped message; the platform enforces access and rollout.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// async fn inspect(client: &Client, channel: uuid::Uuid, message: uuid::Uuid) -> Result<()> {
    ///     let result = client.get_message_emojis(channel, message).await?;
    ///     for emoji in result.emojis { println!("{}: {}", emoji.name, emoji.url); }
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_message_emojis(&self, channel: Uuid, message: Uuid) -> Result<MessageEmojis> {
        UiApiExt::get_message_emojis(self, channel, message).await
    }
}
