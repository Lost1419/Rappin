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
use crate::{Freshness, Result};
use async_trait::async_trait;
use reqwest::Method;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
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
        Err(crate::YappinError::validation(
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
        Err(crate::YappinError::validation(
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
            return Err(crate::YappinError::validation(
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
    /// Fetch the authenticated bot profile through the native or worker transport.
    /// Always requests fresh data. The Client inherent method also updates its cache.
    ///
    /// # Example
    /// ```no_run
    /// use rappin::ApiExt;
    /// async fn identify(api: &(impl rappin::Api + ?Sized)) -> rappin::Result<rappin::BotProfile> {
    ///     api.get_me().await
    /// }
    /// ```
    async fn get_me(&self) -> Result<crate::BotProfile> {
        Ok(serde_json::from_value(
            self.request(Method::GET, "/me", None).await?,
        )?)
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
            return Err(crate::YappinError::validation(
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
    async fn get_emojis(&self, server: Uuid) -> Result<crate::EmojisResponse> {
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
        server: Uuid,
        body: &crate::CreateEmoji,
    ) -> Result<crate::EmojiCreateResult> {
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
        server: Uuid,
        name: &str,
        file: &crate::UploadFile,
    ) -> Result<crate::EmojiCreateResult> {
        crate::validate_emoji_name(name)?;
        file.validate()?;
        if file.data.len() > 2_097_152
            || !matches!(file.mime()?, "image/png" | "image/gif" | "image/webp")
        {
            return Err(crate::YappinError::validation(
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
        server: Uuid,
        emoji: Uuid,
        name: &str,
    ) -> Result<crate::EmojiCreateResult> {
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
    async fn delete_emoji(&self, server: Uuid, emoji: Uuid) -> Result<()> {
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
    async fn get_channel_webhooks(&self, channel: Uuid) -> Result<crate::BotWebhooksResponse> {
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
        channel: Uuid,
        body: &crate::CreateBotWebhook,
    ) -> Result<crate::BotWebhook> {
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
    async fn delete_bot_webhook(&self, webhook: Uuid) -> Result<()> {
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
        webhook: Uuid,
        body: &crate::webhook::WebhookMessage,
    ) -> Result<crate::Message> {
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
        channel: Uuid,
        body: &crate::CreateMessage,
        files: &[crate::UploadFile],
    ) -> Result<Vec<crate::Message>> {
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
        server: Uuid,
        freshness: Freshness,
    ) -> Result<crate::PermissionEvaluation> {
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
        server: Uuid,
        actor: Uuid,
        freshness: Freshness,
    ) -> Result<crate::PermissionEvaluation> {
        let snapshot = self.permission_snapshot(server, actor, freshness).await?;
        if snapshot.server.id != server || snapshot.actor != actor {
            return Err(crate::YappinError::validation(
                "permissions",
                "snapshot identity does not match the requested actor/server",
            ));
        }
        snapshot.evaluate()
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
            return Err(crate::YappinError::validation(
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
        self.require_ui_api().await
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
