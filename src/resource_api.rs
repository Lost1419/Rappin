//! Fresh API 1.4 resource reads and mutations through native or worker transports.
use crate::capabilities::{AccessQuestion, CheckResolution, ExternalCheck, OperationPolicy};
use crate::*;
use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;

/// Typed API 1.4 methods on every native or worker Api implementation.
///
/// ```no_run
/// use rappin::{Api, ResourceApiExt, Result};
/// async fn inspect(api: &impl Api, id: uuid::Uuid) -> Result<()> {
///     println!("{}", api.resolve_id(id).await?.object);
///     Ok(())
/// }
/// ```
#[async_trait::async_trait]
pub trait ResourceApiExt: Api {
    /// Resolve an ID visible to the bot. Hidden and absent resources return the platform 404.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
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
            return Err(YappinError::validation(
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
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn access(api: &impl Api, server: uuid::Uuid, user: uuid::Uuid, channel: uuid::Uuid) -> Result<()> {
    ///     let access = api.get_member_permissions(server, user, Some(channel)).await?;
    ///     println!("{:?}", access.permission("manage_messages"));
    ///     Ok(())
    /// }
    /// ```
    async fn get_member_permissions(
        &self,
        server: Uuid,
        user: Uuid,
        channel: Option<Uuid>,
    ) -> Result<MemberPermissions> {
        let path = match channel {
            Some(channel) => format!("/channels/{channel}/permissions/{user}"),
            None => format!("/servers/{server}/members/{user}/permissions"),
        };
        let response = self.request(Method::GET, &path, None).await?;
        let permissions: MemberPermissions =
            serde_json::from_value(response["permissions"].clone())?;
        permissions.validate_scope(server, user, channel)?;
        Ok(permissions)
    }
    /// Read one role in the requested server. Platform access and rollout apply.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn role(api: &impl Api, server: uuid::Uuid, id: uuid::Uuid) -> Result<rappin::Role> {
    ///     api.get_role(server, id).await
    /// }
    /// ```
    async fn get_role(&self, server: Uuid, role: Uuid) -> Result<Role> {
        let response = self
            .request(
                Method::GET,
                &format!("/servers/{server}/roles/{role}"),
                None,
            )
            .await?;
        let result: Role = serde_json::from_value(response["role"].clone())?;
        if result.id != role {
            return Err(YappinError::validation(
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
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn order(api: &impl Api, server: uuid::Uuid, ids: &[uuid::Uuid]) -> Result<()> {
    ///     api.reorder_roles(server, ids).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn reorder_roles(&self, server: Uuid, ids: &[Uuid]) -> Result<Vec<Role>> {
        validate_role_order(ids)?;
        let response = self
            .request(
                Method::PATCH,
                &format!("/servers/{server}/roles"),
                Some(json!({"ids":ids})),
            )
            .await?;
        Ok(serde_json::from_value(response["roles"].clone())?)
    }
    /// Apply explicit role positions using the documented alternate array body.
    /// Ambiguous mutation failures require reconciliation before a retry.
    ///
    /// ```no_run
    /// use rappin::{Api, ResourceApiExt, RolePosition, Result};
    /// async fn reorder(api: &impl Api, server: uuid::Uuid, role: uuid::Uuid) -> Result<()> {
    ///     api.reposition_roles(server, &[RolePosition::new(role, 3)]).await?;
    ///     Ok(())
    /// }
    /// ```
    async fn reposition_roles(
        &self,
        server: Uuid,
        positions: &[RolePosition],
    ) -> Result<Vec<Role>> {
        validate_role_order(
            &positions
                .iter()
                .map(|position| position.id)
                .collect::<Vec<_>>(),
        )?;
        let response = self
            .request(
                Method::PATCH,
                &format!("/servers/{server}/roles"),
                Some(json!(positions)),
            )
            .await?;
        Ok(serde_json::from_value(response["roles"].clone())?)
    }
    /// List channel role/everyone overrides through the explicit override endpoint.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn overrides(api: &impl Api, channel: uuid::Uuid) -> Result<Vec<rappin::ResourceOverride>> {
    ///     api.get_channel_overrides(channel).await
    /// }
    /// ```
    async fn get_channel_overrides(&self, channel: Uuid) -> Result<Vec<ResourceOverride>> {
        let response = self
            .request(Method::GET, &format!("/channels/{channel}/overrides"), None)
            .await?;
        decode_overrides(response, Some(channel), None)
    }
    /// List category role/everyone overrides. Category inheritance is computed by Yapin.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn overrides(api: &impl Api, category: uuid::Uuid) -> Result<Vec<rappin::ResourceOverride>> {
    ///     api.get_category_overrides(category).await
    /// }
    /// ```
    async fn get_category_overrides(&self, category: Uuid) -> Result<Vec<ResourceOverride>> {
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
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget, PermissionOverride};
    /// async fn override_role(api: &impl Api, channel: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.set_channel_override(channel, PermissionTarget::Everyone, &PermissionOverride::default()).await
    /// }
    /// ```
    async fn set_channel_override(
        &self,
        channel: Uuid,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<Value> {
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
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget};
    /// async fn clear(api: &impl Api, channel: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.delete_channel_override(channel, PermissionTarget::Everyone).await
    /// }
    /// ```
    async fn delete_channel_override(
        &self,
        channel: Uuid,
        target: PermissionTarget,
    ) -> Result<Value> {
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
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget, PermissionOverride};
    /// async fn set(api: &impl Api, category: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.set_category_override(category, PermissionTarget::Everyone, &PermissionOverride::default()).await
    /// }
    /// ```
    async fn set_category_override(
        &self,
        category: Uuid,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<Value> {
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
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget};
    /// async fn clear(api: &impl Api, category: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.delete_category_override(category, PermissionTarget::Everyone).await
    /// }
    /// ```
    async fn delete_category_override(
        &self,
        category: Uuid,
        target: PermissionTarget,
    ) -> Result<Value> {
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
    /// # use rappin::{Api, ResourceApiExt, Result, StickerMessage};
    /// async fn send(api: &impl Api, channel: uuid::Uuid, sticker: uuid::Uuid) -> Result<rappin::UiMessage> {
    ///     api.send_sticker(channel, &StickerMessage::new(sticker)).await
    /// }
    /// ```
    async fn send_sticker(&self, channel: Uuid, body: &StickerMessage) -> Result<UiMessage> {
        let value = self
            .request(
                Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(json!(body)),
            )
            .await?;
        let message: UiMessage = serde_json::from_value(value["message"].clone())?;
        if message.channel_id != channel {
            return Err(YappinError::Remote(RemoteError {
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

impl Client {
    /// Resolve an ID visible to the bot. Hidden and absent resources return the platform 404.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn resolve(api: &impl Api, id: uuid::Uuid) -> Result<rappin::ResolvedObject> {
    ///     api.resolve_id(id).await
    /// }
    /// ```
    pub async fn resolve_id(&self, id: Uuid) -> Result<ResolvedObject> {
        ResourceApiExt::resolve_id(self, id).await
    }
    /// Fetch server-computed permissions for one member, validating all requested scope.
    /// Each call performs a fresh read. Platform rollout and access errors propagate.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn access(api: &impl Api, server: uuid::Uuid, user: uuid::Uuid, channel: uuid::Uuid) -> Result<()> {
    ///     let access = api.get_member_permissions(server, user, Some(channel)).await?;
    ///     println!("{:?}", access.permission("manage_messages"));
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_member_permissions(
        &self,
        server: Uuid,
        user: Uuid,
        channel: Option<Uuid>,
    ) -> Result<MemberPermissions> {
        ResourceApiExt::get_member_permissions(self, server, user, channel).await
    }
    /// Read one role in the requested server. Platform access and rollout apply.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn role(api: &impl Api, server: uuid::Uuid, id: uuid::Uuid) -> Result<rappin::Role> {
    ///     api.get_role(server, id).await
    /// }
    /// ```
    pub async fn get_role(&self, server: Uuid, role: Uuid) -> Result<Role> {
        ResourceApiExt::get_role(self, server, role).await
    }
    /// Reorder role IDs highest first. Roles at or above the bot's highest role stay put.
    /// An ambiguous failure must be reconciled before retrying this mutation.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn order(api: &impl Api, server: uuid::Uuid, ids: &[uuid::Uuid]) -> Result<()> {
    ///     api.reorder_roles(server, ids).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reorder_roles(&self, server: Uuid, ids: &[Uuid]) -> Result<Vec<Role>> {
        ResourceApiExt::reorder_roles(self, server, ids).await
    }
    /// List channel role/everyone overrides through the explicit override endpoint.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn overrides(api: &impl Api, channel: uuid::Uuid) -> Result<Vec<rappin::ResourceOverride>> {
    ///     api.get_channel_overrides(channel).await
    /// }
    /// ```
    pub async fn get_channel_overrides(&self, channel: Uuid) -> Result<Vec<ResourceOverride>> {
        ResourceApiExt::get_channel_overrides(self, channel).await
    }
    /// List category role/everyone overrides. Category inheritance is computed by Yapin.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result};
    /// async fn overrides(api: &impl Api, category: uuid::Uuid) -> Result<Vec<rappin::ResourceOverride>> {
    ///     api.get_category_overrides(category).await
    /// }
    /// ```
    pub async fn get_category_overrides(&self, category: Uuid) -> Result<Vec<ResourceOverride>> {
        ResourceApiExt::get_category_overrides(self, category).await
    }
    /// Replace a channel override for a role or everyone. Member overrides are unsupported upstream.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget, PermissionOverride};
    /// async fn override_role(api: &impl Api, channel: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.set_channel_override(channel, PermissionTarget::Everyone, &PermissionOverride::default()).await
    /// }
    /// ```
    pub async fn set_channel_override(
        &self,
        channel: Uuid,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<Value> {
        ResourceApiExt::set_channel_override(self, channel, target, body).await
    }
    /// Remove a channel override. Platform hierarchy, access and rollout remain authoritative.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget};
    /// async fn clear(api: &impl Api, channel: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.delete_channel_override(channel, PermissionTarget::Everyone).await
    /// }
    /// ```
    pub async fn delete_channel_override(
        &self,
        channel: Uuid,
        target: PermissionTarget,
    ) -> Result<Value> {
        ResourceApiExt::delete_channel_override(self, channel, target).await
    }
    /// Replace a category override, subject to access in every affected channel.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget, PermissionOverride};
    /// async fn set(api: &impl Api, category: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.set_category_override(category, PermissionTarget::Everyone, &PermissionOverride::default()).await
    /// }
    /// ```
    pub async fn set_category_override(
        &self,
        category: Uuid,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<Value> {
        ResourceApiExt::set_category_override(self, category, target, body).await
    }
    /// Remove a category override. Unknown mutation outcomes require reconciliation.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result, PermissionTarget};
    /// async fn clear(api: &impl Api, category: uuid::Uuid) -> Result<serde_json::Value> {
    ///     api.delete_category_override(category, PermissionTarget::Everyone).await
    /// }
    /// ```
    pub async fn delete_category_override(
        &self,
        category: Uuid,
        target: PermissionTarget,
    ) -> Result<Value> {
        ResourceApiExt::delete_category_override(self, category, target).await
    }
    /// Send a standalone sticker from the destination server, preserving ordinary API errors.
    ///
    /// ```no_run
    /// # use rappin::{Api, ResourceApiExt, Result, StickerMessage};
    /// async fn send(api: &impl Api, channel: uuid::Uuid, sticker: uuid::Uuid) -> Result<rappin::UiMessage> {
    ///     api.send_sticker(channel, &StickerMessage::new(sticker)).await
    /// }
    /// ```
    pub async fn send_sticker(&self, channel: Uuid, body: &StickerMessage) -> Result<UiMessage> {
        ResourceApiExt::send_sticker(self, channel, body).await
    }
}
impl Client {
    /// Apply explicit role positions using the documented alternate array body.
    /// Ambiguous mutation failures require reconciliation before a retry.
    ///
    /// ```no_run
    /// use rappin::{Api, ResourceApiExt, RolePosition, Result};
    /// async fn reorder(api: &impl Api, server: uuid::Uuid, role: uuid::Uuid) -> Result<()> {
    ///     api.reposition_roles(server, &[RolePosition::new(role, 3)]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn reposition_roles(
        &self,
        server: Uuid,
        positions: &[RolePosition],
    ) -> Result<Vec<Role>> {
        ResourceApiExt::reposition_roles(self, server, positions).await
    }
}
impl<T: Api + ?Sized> ResourceApiExt for T {}

pub(crate) fn validate_role_order(ids: &[Uuid]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if ids.iter().any(|id| !seen.insert(id)) {
        return Err(YappinError::validation(
            "ids",
            "role order contains duplicate IDs",
        ));
    }
    Ok(())
}
pub(crate) fn decode_overrides(
    value: Value,
    channel: Option<Uuid>,
    category: Option<Uuid>,
) -> Result<Vec<ResourceOverride>> {
    let rules: Vec<ResourceOverride> = serde_json::from_value(value["overrides"].clone())?;
    if rules.iter().any(|r| {
        r.object != "override"
            || channel.is_some_and(|id| r.channel_id != Some(id))
            || category.is_some_and(|id| r.category_id != Some(id))
            || r.id != r.role_id
            || !matches!(
                (r.kind.as_str(), r.role_id),
                ("everyone", None) | ("role", Some(_))
            )
    }) {
        return Err(YappinError::validation(
            "overrides",
            "override response has inconsistent resource or target identity",
        ));
    }
    Ok(rules)
}

/// Fresh authoritative channel-permission policy composable with other operation policies.
/// Unknown category, hierarchy, content or rollout facts require another policy.
///
/// ```no_run
/// use rappin::{ComputedPermissionsPolicy, Api};
/// fn policy(api: &impl Api) { let _policy = ComputedPermissionsPolicy::new(api); }
/// ```
pub struct ComputedPermissionsPolicy<'a, A: Api + ?Sized> {
    api: &'a A,
}
impl<'a, A: Api + ?Sized> ComputedPermissionsPolicy<'a, A> {
    /// Bind a native or worker transport for fresh reads on each assessment.
    ///
    /// ```no_run
    /// # use rappin::{ComputedPermissionsPolicy, Api};
    /// fn policy(api: &impl Api) { let _policy = ComputedPermissionsPolicy::new(api); }
    /// ```
    pub fn new(api: &'a A) -> Self {
        Self { api }
    }
}
#[async_trait::async_trait]
impl<A: Api + ?Sized> OperationPolicy for ComputedPermissionsPolicy<'_, A> {
    async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution> {
        if question.check != ExternalCheck::ChannelPermissions {
            return Ok(CheckResolution::unknown(
                "this fact requires a separate operation policy",
            ));
        }
        let (Some(actor), Some(channel)) = (question.actor_id, question.channel_id) else {
            return Ok(CheckResolution::unknown(
                "actor and channel identity are required",
            ));
        };
        let access = self
            .api
            .get_member_permissions(question.server_id, actor, Some(channel))
            .await?;
        let mut unknown = false;
        for permission in question.permissions {
            match access.permission(permission) {
                Some(true) => (),
                Some(false) => {
                    return Ok(CheckResolution::denied(format!(
                        "computed channel permission {permission} is denied"
                    )))
                }
                None => unknown = true,
            }
        }
        if unknown {
            Ok(CheckResolution::unknown(
                "computed response omitted a required permission",
            ))
        } else {
            Ok(CheckResolution::allowed(
                "fresh computed channel permissions allow this actor",
            ))
        }
    }
}

pub(crate) fn role_order_ids(body: &Value) -> Result<Vec<Uuid>> {
    let ids: Vec<Uuid> = if body.is_array() {
        serde_json::from_value::<Vec<RolePosition>>(body.clone())?
            .into_iter()
            .map(|position| position.id)
            .collect()
    } else {
        serde_json::from_value(body["ids"].clone())?
    };
    validate_role_order(&ids)?;
    Ok(ids)
}
