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
pub trait Api: Send + Sync {
    /// Record callback deadlines/kind when supported by the transport. Custom
    /// adapters own their coordination; worker invocations are tracked by the parent.
    async fn track_interaction(&self, _event: &crate::InteractionEvent) -> Result<()> {
        Ok(())
    }
    /// Sanitize operation results before returning them to model-facing code.
    fn sanitize(&self, value: Value) -> Value {
        crate::security::redact_fields(value)
    }
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value>;
    /// Exactly one transport attempt. Custom adapters must opt in explicitly;
    /// delegating to an ordinary retrying request would violate this contract.
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
    async fn public_request(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None).await
    }
    /// Bounded multipart transfer. Unsupported custom adapters fail explicitly.
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
pub trait ApiExt: Api {
    async fn get_api_version(&self) -> Result<crate::VersionInfo> {
        Ok(serde_json::from_value(
            self.public_request("/version").await?,
        )?)
    }
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
    async fn get_emojis(&self, server: Uuid) -> Result<crate::EmojisResponse> {
        self.api_json::<_, ()>(Method::GET, &format!("/servers/{server}/emojis"), None)
            .await
    }
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
    async fn upload_emoji(
        &self,
        server: Uuid,
        name: &str,
        file: &crate::UploadFile,
    ) -> Result<crate::EmojiCreateResult> {
        crate::validate_emoji_name(name)?;
        file.validate()?;
        if file.data.len() > 1_048_576
            || !matches!(file.mime()?, "image/png" | "image/gif" | "image/webp")
        {
            return Err(crate::YappinError::validation(
                "file",
                "emoji requires PNG/GIF/WebP up to 1 MiB",
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
    async fn delete_emoji(&self, server: Uuid, emoji: Uuid) -> Result<()> {
        self.request(
            Method::DELETE,
            &format!("/servers/{server}/emojis/{emoji}"),
            None,
        )
        .await?;
        Ok(())
    }
    async fn get_channel_webhooks(&self, channel: Uuid) -> Result<crate::BotWebhooksResponse> {
        self.api_json::<_, ()>(Method::GET, &format!("/channels/{channel}/webhooks"), None)
            .await
    }
    async fn get_bot_webhooks(&self) -> Result<crate::BotWebhooksResponse> {
        self.api_json::<_, ()>(Method::GET, "/webhooks", None).await
    }
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
    async fn delete_bot_webhook(&self, webhook: Uuid) -> Result<()> {
        self.request(Method::DELETE, &format!("/webhooks/{webhook}"), None)
            .await?;
        Ok(())
    }
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
    async fn evaluate_bot_permissions(
        &self,
        server: Uuid,
        freshness: Freshness,
    ) -> Result<crate::PermissionEvaluation> {
        let profile: crate::BotProfile = self.api_json::<_, ()>(Method::GET, "/me", None).await?;
        self.evaluate_permissions(server, profile.id, freshness)
            .await
    }
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
