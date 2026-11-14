//! # Example
//!
//! ```rust
//! use rappin::*;
//! use uuid::Uuid;
//! use reqwest::Method;
//! async fn example(api: &impl Api, server: Uuid, requester: Uuid) -> Result<()> {
//!     let response = api.evaluate_permissions(server, requester, Freshness::Refresh).await?;
//!     Ok(())
//! }
//! ```
//! Shared JSON transport for native handlers and isolated Rust workers.
use crate::resource_api::{decode_overrides, validate_role_order};
use crate::*;
use async_trait::async_trait;
use reqwest::Method;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// Authenticated, origin-confined JSON access without exposing credentials.
/// Implement this for application adapters or local test transports as needed.
#[async_trait]
/// # Example
///
/// ```rust
/// use rappin::*;
/// use uuid::Uuid;
/// use reqwest::Method;
/// async fn example(api: &impl Api) -> Result<()> {
///     let response = api.request(Method::GET, "/me", None).await?;
///     Ok(())
/// }
/// ```
pub trait Api: Send + Sync {
    /// Record callback deadlines/kind when supported by the transport. Custom
    /// adapters own their coordination; worker invocations are tracked by the parent.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, event: &InteractionEvent) -> Result<()> {
    ///     api.track_interaction(event).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn track_interaction(&self, _event: &crate::InteractionEvent) -> Result<()> {
        Ok(())
    }
    /// Sanitize operation results before returning them to model-facing code.
    /// # Example
    ///
    /// ```rust
    /// fn safe_output(api: &impl rappin::Api) {
    ///     let output = api.sanitize(serde_json::json!({"token":"sensitive-example","count":3}));
    ///     assert_ne!(output["token"], "sensitive-example");
    /// }
    /// ```
    fn sanitize(&self, value: Value) -> Value {
        crate::security::redact_fields(value)
    }
    /// Send an origin-relative JSON operation through this authenticated transport. Retry and timeout behavior belong to the implementation. This method does not establish requester permission or authorize an application workflow.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.request(Method::GET, "/me", None).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value>;
    /// Exactly one transport attempt. Custom adapters must opt in explicitly;
    /// delegating to an ordinary retrying request would violate this contract.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid) -> Result<()> {
    ///     let response = api.request_once(Method::POST, &format!("/channels/{channel}/messages"), Some(serde_json::json!({"content":"Hello"}))).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn request_once(
        &self,
        _method: Method,
        _path: &str,
        _body: Option<Value>,
    ) -> Result<Value> {
        Err(crate::Error::validation(
            "request_once",
            "transport does not implement single-attempt requests",
        ))
    }

    /// Public metadata reads. Client omits Authorization; adapters can override.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.public_request("/version").await?;
    ///     Ok(())
    /// }
    /// ```
    async fn public_request(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None).await
    }
    /// Bounded multipart transfer. Unsupported custom adapters fail explicitly.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid, file: UploadFile) -> Result<()> {
    ///     let response = api.upload(&format!("/channels/{channel}/messages"), serde_json::json!({"content":"Report"}), &[file]).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn upload(
        &self,
        _path: &str,
        _body: Value,
        _files: &[crate::UploadFile],
    ) -> Result<Value> {
        Err(crate::Error::validation(
            "upload",
            "transport does not support multipart uploads",
        ))
    }

    /// Server-level permission snapshot for any actor. The default fetches fresh
    /// data; Client overrides this to honor its application-owned lookup cache.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid, requester: Uuid) -> Result<()> {
    ///     let response = api.permission_snapshot(server, requester, Freshness::Refresh).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        _freshness: Freshness,
    ) -> Result<crate::PermissionSnapshot> {
        let value = self
            .request(Method::GET, &format!("/servers/{server}"), None)
            .await?;
        let expected_server = server;
        let server: crate::Server = serde_json::from_value(value["server"].clone())?;
        if server.id != expected_server {
            return Err(crate::Error::validation(
                "permissions",
                "API returned a different server ID",
            ));
        }
        if server.owner_id == actor {
            return Ok(crate::PermissionSnapshot {
                server,
                actor,
                member: None,
                roles: vec![],
            });
        }
        let member = self
            .request(
                Method::GET,
                &format!("/servers/{}/members/{actor}", server.id),
                None,
            )
            .await?;
        let roles = self
            .request(Method::GET, &format!("/servers/{}/roles", server.id), None)
            .await?;
        Ok(crate::PermissionSnapshot {
            server,
            actor,
            member: Some(serde_json::from_value(member["member"].clone())?),
            roles: serde_json::from_value(roles["roles"].clone())?,
        })
    }
}

/// Generic decoding and permission evaluation on every Api implementation.
#[async_trait]
/// # Example
///
/// ```rust
/// use rappin::*;
/// use uuid::Uuid;
/// use reqwest::Method;
/// async fn example(api: &impl Api, server: Uuid) -> Result<()> {
///     let response = api.get_emojis(server).await?;
///     Ok(())
/// }
/// ```
pub trait ApiExt: Api {
    /// Update the original interaction message using an explicit edit patch.
    /// Omitted fields retain their values and null fields clear them. Ownership,
    /// source-message eligibility and callback deadlines remain enforced.
    ///
    /// ```no_run
    /// async fn clear(api: &impl rappin::Api, event: &rappin::InteractionEvent) -> rappin::Result<()> {
    ///     use rappin::ApiExt;
    ///     api.update_interaction(event, &rappin::EditMessage::new().clear_components()).await
    /// }
    /// ```
    async fn update_interaction(
        &self,
        event: &InteractionEvent,
        patch: &EditMessage,
    ) -> Result<()> {
        if event.kind.as_deref() == Some("command")
            || event.kind.as_deref() == Some("modal_submit") && event.message_id.is_none()
        {
            return Err(Error::validation(
                "type",
                "this interaction has no original message to update",
            ));
        }
        let mut value = json!(patch);
        value["type"] = json!(InteractionCallbackType::Update);
        crate::validation::validate_callback(&value, InteractionCallbackType::Update)?;
        Api::track_interaction(self, event).await?;
        crate::utils::strip_proxy_metadata(&mut value);
        self.request(
            Method::POST,
            &crate::interactions::path(&event.id, &event.token),
            Some(value),
        )
        .await?;
        Ok(())
    }
    /// Read unauthenticated API/build and official SDK version metadata. Client omits Authorization; custom adapters define their public-read handling.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.get_api_version().await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_api_version(&self) -> Result<crate::VersionInfo> {
        Ok(serde_json::from_value(
            self.public_request("/version").await?,
        )?)
    }
    /// Read 1 through 50 entries from the public platform changelog.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.get_api_changelog(10).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_api_changelog(&self, limit: u32) -> Result<crate::ApiChangelog> {
        if !(1..=50).contains(&limit) {
            return Err(crate::Error::validation(
                "limit",
                "requires 1 through 50 entries",
            ));
        }
        Ok(serde_json::from_value(
            self.public_request(&format!("/changelog?limit={limit}"))
                .await?,
        )?)
    }
    /// Read custom-emoji inventory and the reported server slot limit.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid) -> Result<()> {
    ///     let response = api.get_emojis(server).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_emojis(
        &self,
        server: impl Into<crate::ServerId> + Send,
    ) -> Result<crate::EmojisResponse> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        self.api_json::<_, ()>(Method::GET, &format!("/servers/{server}/emojis"), None)
            .await
    }
    /// Validate and create a base64/data-URL emoji. The current SDK cap is 2 MiB decoded; a pending moderation result is successful but not yet usable.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid, body: &CreateEmoji) -> Result<()> {
    ///     let response = api.create_emoji(server, body).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn create_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        body: &crate::CreateEmoji,
    ) -> Result<crate::EmojiCreateResult> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        body.validate()?;
        self.api_json(
            Method::POST,
            &format!("/servers/{server}/emojis"),
            Some(body),
        )
        .await
    }
    /// Validate and upload one PNG/GIF/WebP emoji using the adapter's multipart support. The current SDK cap is 2 MiB.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid, file: &UploadFile) -> Result<()> {
    ///     let response = api.upload_emoji(server, "wave", file).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn upload_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        name: &str,
        file: &crate::UploadFile,
    ) -> Result<crate::EmojiCreateResult> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        crate::validate_emoji_name(name)?;
        file.validate()?;
        if file.data.len() > 2_097_152
            || !matches!(file.mime()?, "image/png" | "image/gif" | "image/webp")
        {
            return Err(crate::Error::validation(
                "file",
                "emoji requires PNG/GIF/WebP up to 2 MiB",
            ));
        }
        Ok(serde_json::from_value(
            self.upload(
                &format!("/servers/{server}/emojis"),
                serde_json::json!({"name":name}),
                std::slice::from_ref(file),
            )
            .await?,
        )?)
    }
    /// Validate a replacement emoji name and submit the rename. Platform permissions and scope remain authoritative.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid, emoji: Uuid) -> Result<()> {
    ///     let response = api.edit_emoji(server, emoji, "wave_renamed").await?;
    ///     Ok(())
    /// }
    /// ```
    async fn edit_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        emoji: impl Into<crate::EmojiId> + Send,
        name: &str,
    ) -> Result<crate::EmojiCreateResult> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let emoji: crate::EmojiId = emoji.into();
        let emoji = Uuid::from(emoji);

        crate::validate_emoji_name(name)?;
        self.api_json(
            Method::PATCH,
            &format!("/servers/{server}/emojis/{emoji}"),
            Some(&serde_json::json!({"name":name})),
        )
        .await
    }
    /// Delete one server emoji. Do not retry the workflow after an uncertain mutation outcome.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid, emoji: Uuid) -> Result<()> {
    ///     let response = api.delete_emoji(server, emoji).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn delete_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        emoji: impl Into<crate::EmojiId> + Send,
    ) -> Result<()> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let emoji: crate::EmojiId = emoji.into();
        let emoji = Uuid::from(emoji);

        self.request(
            Method::DELETE,
            &format!("/servers/{server}/emojis/{emoji}"),
            None,
        )
        .await?;
        Ok(())
    }
    /// Read bot-owned webhooks visible in the selected channel.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid) -> Result<()> {
    ///     let response = api.get_channel_webhooks(channel).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_channel_webhooks(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
    ) -> Result<crate::BotWebhooksResponse> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        self.api_json::<_, ()>(Method::GET, &format!("/channels/{channel}/webhooks"), None)
            .await
    }
    /// Read webhooks owned by the authenticated bot.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.get_bot_webhooks().await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_bot_webhooks(&self) -> Result<crate::BotWebhooksResponse> {
        self.api_json::<_, ()>(Method::GET, "/webhooks", None).await
    }
    /// Validate a webhook name and create a bot-owned webhook in the selected channel.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid) -> Result<()> {
    ///     let response = api.create_bot_webhook(channel, &CreateBotWebhook::new("Notifier")).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn create_bot_webhook(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        body: &crate::CreateBotWebhook,
    ) -> Result<crate::BotWebhook> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        body.validate()?;
        #[derive(serde::Deserialize)]
        struct Response {
            webhook: crate::BotWebhook,
        }
        let response: Response = self
            .api_json(
                Method::POST,
                &format!("/channels/{channel}/webhooks"),
                Some(body),
            )
            .await?;
        Ok(response.webhook)
    }
    /// Delete a bot-owned webhook. Ownership and channel permissions remain API checks.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, webhook: Uuid) -> Result<()> {
    ///     let response = api.delete_bot_webhook(webhook).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn delete_bot_webhook(&self, webhook: impl Into<crate::WebhookId> + Send) -> Result<()> {
        let webhook: crate::WebhookId = webhook.into();
        let webhook = Uuid::from(webhook);

        self.request(Method::DELETE, &format!("/webhooks/{webhook}"), None)
            .await?;
        Ok(())
    }
    /// Validate and post through a bot-owned webhook using bot authentication. Response-only image proxy metadata is removed from the outbound body.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, webhook: Uuid) -> Result<()> {
    ///     let response = api.post_bot_webhook(webhook, &WebhookMessage { content: Some("Service is online".into()), ..Default::default() }).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn post_bot_webhook(
        &self,
        webhook: impl Into<crate::WebhookId> + Send,
        body: &crate::webhook::WebhookMessage,
    ) -> Result<crate::Message> {
        let webhook: crate::WebhookId = webhook.into();
        let webhook = Uuid::from(webhook);

        let mut value = serde_json::to_value(body)?;
        crate::utils::strip_proxy_metadata(&mut value);
        crate::validation::validate_webhook(&value)?;
        #[derive(serde::Deserialize)]
        struct Response {
            message: crate::Message,
        }
        let response: Response = self
            .api_json(Method::POST, &format!("/webhooks/{webhook}"), Some(&value))
            .await?;
        Ok(response.message)
    }
    /// Send files once. A partial HTTP-success response becomes PartialWrite with receipts.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, channel: Uuid, files: &[UploadFile]) -> Result<()> {
    ///     let response = api.send_files(channel, &CreateMessage::new("Report"), files).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn send_files(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        body: &crate::CreateMessage,
        files: &[crate::UploadFile],
    ) -> Result<Vec<crate::Message>> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        crate::validate_uploads(files)?;
        let clean = body.for_request();
        let body = clean.as_ref();
        let body = serde_json::to_value(body)?;
        crate::validation::validate_message(&body, false)?;
        let value = self
            .upload(&format!("/channels/{channel}/messages"), body, files)
            .await?;
        serde_json::from_value::<crate::UploadedMessages>(value)?.into_result_for(files.len())
    }

    /// Evaluate the authenticated bot using the same rules as any other member.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid) -> Result<()> {
    ///     let response = api.evaluate_bot_permissions(server, Freshness::Refresh).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn evaluate_bot_permissions(
        &self,
        server: impl Into<crate::ServerId> + Send,
        freshness: Freshness,
    ) -> Result<crate::PermissionEvaluation> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        let profile: crate::BotProfile = self.api_json::<_, ()>(Method::GET, "/me", None).await?;
        self.evaluate_permissions(server, profile.id, freshness)
            .await
    }
    /// Serialize an optional typed body, request JSON through Api, and deserialize the result. It performs no capability scope or requester-authorization assessment.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api) -> Result<()> {
    ///     let response = api.api_json::<BotProfile, ()>(Method::GET, "/me", None).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn api_json<T: DeserializeOwned + Send, B: Serialize + Sync + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        let value = self
            .request(method, path, body.map(serde_json::to_value).transpose()?)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Fetch and evaluate server-level grants for a selected actor. Identity mismatches or incomplete role evidence are errors; channel overrides, timeouts and hierarchy require separate policy.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// use reqwest::Method;
    /// async fn example(api: &impl Api, server: Uuid, requester: Uuid) -> Result<()> {
    ///     let response = api.evaluate_permissions(server, requester, Freshness::Refresh).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn evaluate_permissions(
        &self,
        server: impl Into<crate::ServerId> + Send,
        actor: impl Into<crate::UserId> + Send,
        freshness: Freshness,
    ) -> Result<crate::PermissionEvaluation> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let actor: crate::UserId = actor.into();
        let actor = Uuid::from(actor);

        let snapshot = self.permission_snapshot(server, actor, freshness).await?;
        if snapshot.server.id != server || snapshot.actor != actor {
            return Err(crate::Error::validation(
                "permissions",
                "snapshot identity does not match the requested actor/server",
            ));
        }
        snapshot.evaluate()
    }
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
    async fn get_message_emojis(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        message: impl Into<crate::MessageId> + Send,
    ) -> Result<MessageEmojis> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);
        let message: crate::MessageId = message.into();
        let message = Uuid::from(message);

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
    ///     let response = api.get_messages(channel, Some(25), None, None).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_messages(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        limit: Option<u32>,
        before: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<MessagesResponse> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        if before.is_some() && after.is_some() {
            return Err(Error::validation(
                "cursor",
                "before and after are mutually exclusive",
            ));
        }
        if limit.is_some_and(|limit| !(1..=100).contains(&limit)) {
            return Err(Error::validation("limit", "must be 1..=100"));
        }
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            if let Some(limit) = limit {
                query.append_pair("limit", &limit.to_string());
            }
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
    ///     let response = api.get_message(channel, message).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn get_message(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        message: impl Into<crate::MessageId> + Send,
    ) -> Result<MessageDetailResponse> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);
        let message: crate::MessageId = message.into();
        let message = Uuid::from(message);

        let value = self
            .request(
                Method::GET,
                &format!("/channels/{channel}/messages/{message}"),
                None,
            )
            .await?;
        Ok(serde_json::from_value(value)?)
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
    ///     let body = CreateMessage::new("Choose fruit").component(menu.row());
    ///     let response = api.send_message(channel, &body).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn send_message(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        body: &CreateMessage,
    ) -> Result<Message> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        let mut value = json!(body);
        crate::validation::validate_message(&value, true)?;
        crate::utils::strip_proxy_metadata(&mut value);
        let mut response = self
            .request(
                Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(value),
            )
            .await?;
        Ok(serde_json::from_value(
            response
                .get_mut("message")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?)
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
    ///     let body = EditMessage::new().content("Updated");
    ///     let response = api.edit_message(channel, message, &body).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn edit_message(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        message: impl Into<crate::MessageId> + Send,
        body: &EditMessage,
    ) -> Result<Message> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);
        let message: crate::MessageId = message.into();
        let message = Uuid::from(message);

        let mut value = json!(body);
        crate::validation::validate_message(&value, false)?;
        crate::utils::strip_proxy_metadata(&mut value);
        let mut response = self
            .request(
                Method::PATCH,
                &format!("/channels/{channel}/messages/{message}"),
                Some(value),
            )
            .await?;
        Ok(serde_json::from_value(
            response
                .get_mut("message")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?)
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
            return Err(Error::validation(
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
    ///     let response = api.respond(event, InteractionCallbackType::Ephemeral, &CreateMessage::new("Recorded")).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn respond(
        &self,
        event: &InteractionEvent,
        kind: InteractionCallbackType,
        body: &CreateMessage,
    ) -> Result<()> {
        if body.sticker_id.is_some() {
            return Err(Error::validation(
                "sticker_id",
                "send stickers through the channel message endpoint",
            ));
        }
        if body.reply_to.is_some() {
            return Err(Error::validation(
                "reply_to",
                "interaction callbacks do not accept reply targets",
            ));
        }
        if kind == InteractionCallbackType::Ack {
            return Err(Error::validation(
                "type",
                "use the existing acknowledgement API",
            ));
        }
        if kind == InteractionCallbackType::Update
            && (event.kind.as_deref() == Some("command")
                || event.kind.as_deref() == Some("modal_submit") && event.message_id.is_none())
        {
            return Err(Error::validation(
                "type",
                "this interaction has no original message to update",
            ));
        }
        let mut value = json!(body);
        value["type"] = json!(kind);
        crate::validation::validate_callback(&value, kind)?;
        Api::track_interaction(self, event).await?;
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
        let mut value = self
            .request(
                Method::PUT,
                "/commands",
                Some(json!({"commands":commands,"interactions":enabled})),
            )
            .await?;
        Ok(serde_json::from_value(
            value
                .get_mut("commands")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?)
    }
    /// Resolve an ID visible to the bot. Hidden and absent resources return the platform 404.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result};
    /// async fn resolve(api: &impl Api, id: uuid::Uuid) -> Result<rappin::ResolvedObject> {
    ///     api.resolve_id(id).await
    /// }
    /// ```
    async fn resolve_id(&self, id: Uuid) -> Result<ResolvedObject> {
        let response = self
            .request(Method::GET, &format!("/resolve/{id}"), None)
            .await?;
        let resolved: ResolvedObject = serde_json::from_value(response)?;
        if resolved.id != id || resolved.object.is_empty() {
            return Err(Error::validation(
                "id",
                "resolver returned a different identity or missing object kind",
            ));
        }
        Ok(resolved)
    }
    /// Fetch server-computed permissions for one member, validating all requested scope.
    /// Each call performs a fresh read. Platform rollout and access errors propagate.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result};
    /// async fn access(api: &impl Api, server: uuid::Uuid, user: uuid::Uuid, channel: uuid::Uuid) -> Result<()> {
    ///     let access = api.get_member_permissions(server, user, Some(channel)).await?;
    ///     println!("{:?}", access.permission("manage_messages"));
    ///     Ok(())
    /// }
    /// ```
    async fn get_member_permissions(
        &self,
        server: impl Into<crate::ServerId> + Send,
        user: impl Into<crate::UserId> + Send,
        channel: Option<Uuid>,
    ) -> Result<MemberPermissions> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let user: crate::UserId = user.into();
        let user = Uuid::from(user);

        let path = match channel {
            Some(channel) => format!("/channels/{channel}/permissions/{user}"),
            None => format!("/servers/{server}/members/{user}/permissions"),
        };
        let mut response = self.request(Method::GET, &path, None).await?;
        let permissions: MemberPermissions = serde_json::from_value(
            response
                .get_mut("permissions")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?;
        permissions.validate_scope(server, user, channel)?;
        Ok(permissions)
    }
    /// Read one role in the requested server. Platform access and rollout apply.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result};
    /// async fn role(api: &impl Api, server: uuid::Uuid, id: uuid::Uuid) -> Result<rappin::Role> {
    ///     api.get_role(server, id).await
    /// }
    /// ```
    async fn get_role(
        &self,
        server: impl Into<crate::ServerId> + Send,
        role: impl Into<crate::RoleId> + Send,
    ) -> Result<Role> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let role: crate::RoleId = role.into();
        let role = Uuid::from(role);

        let mut response = self
            .request(
                Method::GET,
                &format!("/servers/{server}/roles/{role}"),
                None,
            )
            .await?;
        let result: Role = serde_json::from_value(
            response
                .get_mut("role")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?;
        if result.id != role {
            return Err(Error::validation(
                "role",
                "API returned a different role identity",
            ));
        }
        Ok(result)
    }
    /// Reorder role IDs highest first. Roles at or above the bot's highest role stay put.
    /// An ambiguous failure must be reconciled before retrying this mutation.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result};
    /// async fn order(api: &impl Api, server: uuid::Uuid, ids: &[uuid::Uuid]) -> Result<()> {
    ///     api.reorder_roles(server, ids).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn reorder_roles(
        &self,
        server: impl Into<crate::ServerId> + Send,
        ids: &[Uuid],
    ) -> Result<Vec<Role>> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        validate_role_order(ids)?;
        let mut response = self
            .request(
                Method::PATCH,
                &format!("/servers/{server}/roles"),
                Some(json!({"ids":ids})),
            )
            .await?;
        Ok(serde_json::from_value(
            response
                .get_mut("roles")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?)
    }
    /// Apply explicit role positions using the documented alternate array body.
    /// Ambiguous mutation failures require reconciliation before a retry.
    ///
    /// ```no_run
    /// use rappin::{Api, ApiExt, RolePosition, Result};
    /// async fn reorder(api: &impl Api, server: uuid::Uuid, role: uuid::Uuid) -> Result<()> {
    ///     api.reposition_roles(server, &[RolePosition::new(role, 3)]).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn reposition_roles(
        &self,
        server: impl Into<crate::ServerId> + Send,
        positions: &[RolePosition],
    ) -> Result<Vec<Role>> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        validate_role_order(
            &positions
                .iter()
                .map(|position| position.id)
                .collect::<Vec<_>>(),
        )?;
        let mut response = self
            .request(
                Method::PATCH,
                &format!("/servers/{server}/roles"),
                Some(json!(positions)),
            )
            .await?;
        Ok(serde_json::from_value(
            response
                .get_mut("roles")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?)
    }
    /// List channel role/everyone overrides through the explicit override endpoint.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result};
    /// async fn overrides(api: &impl Api, channel: uuid::Uuid) -> Result<Vec<rappin::ResourceOverride>> {
    ///     api.get_channel_overrides(channel).await
    /// }
    /// ```
    async fn get_channel_overrides(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
    ) -> Result<Vec<ResourceOverride>> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        let response = self
            .request(Method::GET, &format!("/channels/{channel}/overrides"), None)
            .await?;
        decode_overrides(response, Some(channel), None)
    }
    /// List category role/everyone overrides. Category inheritance is computed by Yapin.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result};
    /// async fn overrides(api: &impl Api, category: uuid::Uuid) -> Result<Vec<rappin::ResourceOverride>> {
    ///     api.get_category_overrides(category).await
    /// }
    /// ```
    async fn get_category_overrides(
        &self,
        category: impl Into<crate::CategoryId> + Send,
    ) -> Result<Vec<ResourceOverride>> {
        let category: crate::CategoryId = category.into();
        let category = Uuid::from(category);

        let response = self
            .request(
                Method::GET,
                &format!("/categories/{category}/overrides"),
                None,
            )
            .await?;
        decode_overrides(response, None, Some(category))
    }
    /// Replace a channel override for a role or everyone. Member overrides are unsupported upstream.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result, PermissionTarget, PermissionOverride};
    /// async fn override_role(api: &impl Api, channel: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.set_channel_override(channel, PermissionTarget::Everyone, &PermissionOverride::default()).await
    /// }
    /// ```
    async fn set_channel_override(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<Value> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        crate::models::api14::validate_override(body)?;
        self.request(
            Method::PUT,
            &format!("/channels/{channel}/overrides/{target}"),
            Some(json!(body)),
        )
        .await
    }
    /// Remove a channel override. Platform hierarchy, access and rollout remain authoritative.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result, PermissionTarget};
    /// async fn clear(api: &impl Api, channel: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.delete_channel_override(channel, PermissionTarget::Everyone).await
    /// }
    /// ```
    async fn delete_channel_override(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        target: PermissionTarget,
    ) -> Result<Value> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        self.request(
            Method::DELETE,
            &format!("/channels/{channel}/overrides/{target}"),
            None,
        )
        .await
    }
    /// Replace a category override, subject to access in every affected channel.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result, PermissionTarget, PermissionOverride};
    /// async fn set(api: &impl Api, category: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.set_category_override(category, PermissionTarget::Everyone, &PermissionOverride::default()).await
    /// }
    /// ```
    async fn set_category_override(
        &self,
        category: impl Into<crate::CategoryId> + Send,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<Value> {
        let category: crate::CategoryId = category.into();
        let category = Uuid::from(category);

        crate::models::api14::validate_override(body)?;
        self.request(
            Method::PUT,
            &format!("/categories/{category}/overrides/{target}"),
            Some(json!(body)),
        )
        .await
    }
    /// Remove a category override. Unknown mutation outcomes require reconciliation.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result, PermissionTarget};
    /// async fn clear(api: &impl Api, category: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.delete_category_override(category, PermissionTarget::Everyone).await
    /// }
    /// ```
    async fn delete_category_override(
        &self,
        category: impl Into<crate::CategoryId> + Send,
        target: PermissionTarget,
    ) -> Result<Value> {
        let category: crate::CategoryId = category.into();
        let category = Uuid::from(category);

        self.request(
            Method::DELETE,
            &format!("/categories/{category}/overrides/{target}"),
            None,
        )
        .await
    }
    /// Send a standalone sticker from the destination server, preserving ordinary API errors.
    ///
    /// ```no_run
    /// # use rappin::{Api, ApiExt, Result, StickerMessage};
    /// async fn send(api: &impl Api, channel: uuid::Uuid, sticker: uuid::Uuid) -> Result<rappin::Message> {
    ///     api.send_sticker(channel, &StickerMessage::new(sticker)).await
    /// }
    /// ```
    async fn send_sticker(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        body: &StickerMessage,
    ) -> Result<Message> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        let mut value = self
            .request(
                Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(json!(body)),
            )
            .await?;
        let message: Message = serde_json::from_value(
            value
                .get_mut("message")
                .map(Value::take)
                .unwrap_or(Value::Null),
        )?;
        if message.channel_id != channel {
            return Err(Error::Remote(RemoteError {
                kind: "operation_result_scope".into(),
                message:
                    "sticker write response has a different channel; reconcile before retrying"
                        .into(),
                status: None,
                retry_after: None,
                details: [("outcome".into(), "unknown".into())].into(),
            }));
        }
        Ok(message)
    }
}
impl<T: Api + ?Sized> ApiExt for T {}

#[async_trait]
impl Api for crate::Client {
    async fn track_interaction(&self, event: &crate::InteractionEvent) -> Result<()> {
        crate::Client::track_interaction(self, event).await
    }
    fn sanitize(&self, value: Value) -> Value {
        crate::security::redact_fields(self.redact_credentials(value))
    }
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.request_json(method, path, body.as_ref()).await
    }
    async fn request_once(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.request_json_once(method, path, body).await
    }
    async fn public_request(&self, path: &str) -> Result<Value> {
        self.public_json(path).await
    }
    async fn upload(&self, path: &str, body: Value, files: &[crate::UploadFile]) -> Result<Value> {
        self.request_upload(path, body, files).await
    }

    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<crate::PermissionSnapshot> {
        let expected_server = server;
        let server = self.get_server(server).await?;
        if server.id != expected_server {
            return Err(crate::Error::validation(
                "permissions",
                "API returned a different server ID",
            ));
        }
        let (member, roles) = if server.owner_id == actor {
            (None, vec![])
        } else {
            (
                Some(self.fetch_member(server.id, actor, freshness).await?),
                self.fetch_roles(server.id, freshness).await?,
            )
        };
        Ok(crate::PermissionSnapshot {
            server,
            actor,
            member,
            roles,
        })
    }
}

#[async_trait]
impl<T: Api + ?Sized> Api for std::sync::Arc<T> {
    async fn track_interaction(&self, event: &crate::InteractionEvent) -> Result<()> {
        Api::track_interaction(&**self, event).await
    }
    fn sanitize(&self, value: Value) -> Value {
        (**self).sanitize(value)
    }
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        (**self).request(method, path, body).await
    }
    async fn request_once(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        (**self).request_once(method, path, body).await
    }
    async fn public_request(&self, path: &str) -> Result<Value> {
        (**self).public_request(path).await
    }
    async fn upload(&self, path: &str, body: Value, files: &[crate::UploadFile]) -> Result<Value> {
        (**self).upload(path, body, files).await
    }
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<crate::PermissionSnapshot> {
        (**self).permission_snapshot(server, actor, freshness).await
    }
}

macro_rules! native_context {
    ($ty:ty) => {
        #[async_trait]
        impl Api for $ty {
            async fn track_interaction(&self, event: &crate::InteractionEvent) -> Result<()> {
                self.client.track_interaction(event).await
            }
            fn sanitize(&self, value: Value) -> Value {
                self.client.sanitize(value)
            }
            async fn request(
                &self,
                method: Method,
                path: &str,
                body: Option<Value>,
            ) -> Result<Value> {
                Api::request(self.client.as_ref(), method, path, body).await
            }
            async fn request_once(
                &self,
                method: Method,
                path: &str,
                body: Option<Value>,
            ) -> Result<Value> {
                self.client.request_once(method, path, body).await
            }
            async fn public_request(&self, path: &str) -> Result<Value> {
                self.client.public_request(path).await
            }
            async fn upload(
                &self,
                path: &str,
                body: Value,
                files: &[crate::UploadFile],
            ) -> Result<Value> {
                self.client.upload(path, body, files).await
            }
            async fn permission_snapshot(
                &self,
                server: Uuid,
                actor: Uuid,
                freshness: Freshness,
            ) -> Result<crate::PermissionSnapshot> {
                Api::permission_snapshot(self.client.as_ref(), server, actor, freshness).await
            }
        }
    };
}
native_context!(crate::MessageContext);
native_context!(crate::InteractionContext);

#[cfg(feature = "hot-reload")]
#[async_trait]
impl Api for crate::WorkerContext {
    async fn track_interaction(&self, _event: &crate::InteractionEvent) -> Result<()> {
        self.ensure_active()
    }
    async fn request_once(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.api_once(method.as_str(), path, body).await
    }
    async fn public_request(&self, path: &str) -> Result<Value> {
        crate::WorkerContext::public_request(self, path).await
    }
    async fn upload(&self, path: &str, body: Value, files: &[crate::UploadFile]) -> Result<Value> {
        crate::WorkerContext::upload(self, path, body, files).await
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.api(method.as_str(), path, body).await
    }
    async fn permission_snapshot(
        &self,
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<crate::PermissionSnapshot> {
        self.proxy_permissions(server, actor, freshness).await
    }
}
