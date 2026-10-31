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
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value>;

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
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.request_json(method, path, body.as_ref()).await
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
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        (**self).request(method, path, body).await
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
            async fn request(
                &self,
                method: Method,
                path: &str,
                body: Option<Value>,
            ) -> Result<Value> {
                Api::request(self.client.as_ref(), method, path, body).await
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
