//! Shared native/worker API 1.3 operations.
use crate::*;
use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;

#[async_trait::async_trait]
pub trait UiApiExt: Api {
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
pub struct UiInteractionContext {
    pub context: InteractionContext,
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
    pub fn parse<A: crate::FromArgs>(&self) -> Result<A> {
        let mut arguments = crate::Arguments::new(self.data.args.as_deref().unwrap_or(""))?;
        let parsed = A::parse(&mut arguments)?;
        arguments.finish()?;
        Ok(parsed)
    }
    pub async fn show_modal(&self, modal: &Modal) -> Result<()> {
        self.context.show_modal(modal).await
    }
    pub async fn respond_ui(
        &self,
        kind: InteractionCallbackType,
        body: &UiMessageBody,
    ) -> Result<()> {
        UiApiExt::respond_ui(self, &self.context.event, kind, body).await
    }
}
impl InteractionContext {
    pub async fn show_modal(&self, modal: &Modal) -> Result<()> {
        UiApiExt::show_modal(self, &self.event, modal).await
    }
}
impl Client {
    pub async fn get_message_emojis(&self, channel: Uuid, message: Uuid) -> Result<MessageEmojis> {
        UiApiExt::get_message_emojis(self, channel, message).await
    }
}
