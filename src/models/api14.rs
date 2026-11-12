//! API 1.4 computed permissions, resource identity and explicit overrides.
use crate::{PermissionOverride, Result, YappinError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Server-computed access for one member and one optional channel.
///
/// ```rust
/// let value = serde_json::json!({"object":"permissions","server_id":uuid::Uuid::nil(),
///     "user_id":uuid::Uuid::nil(),"channel_id":null,"owner":false,
///     "administrator":false,"permissions":{"send_messages":true}});
/// let access: rappin::MemberPermissions = serde_json::from_value(value)?;
/// assert_eq!(access.permission("send_messages"), Some(true));
/// assert_eq!(access.permission("future_permission"), None);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MemberPermissions {
    /// Platform object discriminator, expected to be permissions.
    pub object: String,
    /// Server containing the assessed member.
    pub server_id: Uuid,
    /// Assessed user, independently of the authenticated bot.
    pub user_id: Uuid,
    /// Assessed channel, or null for server-wide permissions.
    #[serde(deserialize_with = "required_channel")]
    pub channel_id: Option<Uuid>,
    /// Whether this member owns the server.
    pub owner: bool,
    /// Whether owner/Administrator bypass applies.
    pub administrator: bool,
    /// Effective permission flags returned by the platform.
    pub permissions: BTreeMap<String, bool>,
}
impl MemberPermissions {
    /// Read a computed flag, retaining missing evidence as None.
    ///
    /// ```rust
    /// # let access: rappin::MemberPermissions = serde_json::from_value(serde_json::json!({"object":"permissions","server_id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil(),"channel_id":null,"owner":false,"administrator":false,"permissions":{}}))?;
    /// assert_eq!(access.permission("send_messages"), None);
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    pub fn permission(&self, name: &str) -> Option<bool> {
        if self.owner || self.administrator {
            Some(true)
        } else {
            self.permissions.get(name).copied()
        }
    }
    /// Verify authoritative response identity before using its permission evidence.
    ///
    /// ```rust
    /// # let access: rappin::MemberPermissions = serde_json::from_value(serde_json::json!({"object":"permissions","server_id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil(),"channel_id":null,"owner":false,"administrator":false,"permissions":{}}))?;
    /// access.validate_scope(uuid::Uuid::nil(), uuid::Uuid::nil(), None)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn validate_scope(&self, server: Uuid, user: Uuid, channel: Option<Uuid>) -> Result<()> {
        if self.object != "permissions"
            || self.server_id != server
            || self.user_id != user
            || self.channel_id != channel
        {
            return Err(YappinError::validation(
                "permissions",
                "computed permission response has a different object or scope",
            ));
        }
        Ok(())
    }
}

/// Visible object identity returned by GET /resolve/{id}.
///
/// ```rust
/// let resolved: rappin::ResolvedObject = serde_json::from_value(serde_json::json!({
///     "id":uuid::Uuid::nil(),"object":"channel","server_id":uuid::Uuid::nil()}))?;
/// assert_eq!(resolved.object, "channel");
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResolvedObject {
    /// Original requested UUID.
    pub id: Uuid,
    /// Extensible platform kind such as user, channel, role or sticker.
    pub object: String,
    /// Owning server, when applicable and reported.
    pub server_id: Option<Uuid>,
    /// Associated channel, when applicable and reported.
    pub channel_id: Option<Uuid>,
    /// Whether a resolved user is a bot account.
    pub bot: Option<bool>,
}

/// Explicit role/everyone override with its channel or category identity.
///
/// ```rust
/// let rule: rappin::ResourceOverride = serde_json::from_value(serde_json::json!({
///     "object":"override","type":"everyone","id":null,"role_id":null,"allow":[],"deny":[],
///     "channel_id":uuid::Uuid::nil()}))?;
/// assert!(rule.role_id.is_none());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResourceOverride {
    /// Platform discriminator.
    pub object: String,
    /// Role or everyone target kind.
    #[serde(rename = "type")]
    pub kind: String,
    /// Role target, null for everyone.
    pub id: Option<Uuid>,
    /// Compatibility role target, null for everyone.
    pub role_id: Option<Uuid>,
    /// Permissions enabled by this override.
    pub allow: Vec<String>,
    /// Permissions disabled by this override.
    pub deny: Vec<String>,
    /// Resource channel, when this is a channel override.
    pub channel_id: Option<Uuid>,
    /// Resource category, when this is a category override.
    pub category_id: Option<Uuid>,
}

/// Standalone sticker message. The sticker occupies the entire message.
///
/// ```rust
/// let body = rappin::StickerMessage::new(uuid::Uuid::nil());
/// assert_eq!(serde_json::to_value(body)?["sticker_id"], uuid::Uuid::nil().to_string());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StickerMessage {
    /// Sticker belonging to the destination server.
    pub sticker_id: Uuid,
}

/// Explicit role position accepted by the platform's alternate reorder body.
///
/// ```rust
/// let position = rappin::RolePosition::new(uuid::Uuid::nil(), 3);
/// assert_eq!(position.position, 3);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RolePosition {
    /// Role whose position is requested.
    pub id: Uuid,
    /// Requested platform position. Higher numbers rank above lower numbers.
    pub position: i32,
}
impl RolePosition {
    /// Construct an explicit role-position request without granting hierarchy authority.
    ///
    /// ```rust
    /// let position = rappin::RolePosition::new(uuid::Uuid::nil(), 0);
    /// assert_eq!(position.id, uuid::Uuid::nil());
    /// ```
    pub fn new(id: Uuid, position: i32) -> Self {
        Self { id, position }
    }
}
impl StickerMessage {
    /// Construct a message containing only this sticker.
    ///
    /// ```rust
    /// let body = rappin::StickerMessage::new(uuid::Uuid::nil());
    /// assert_eq!(body.sticker_id, uuid::Uuid::nil());
    /// ```
    pub fn new(sticker_id: Uuid) -> Self {
        Self { sticker_id }
    }
}

pub(crate) fn validate_override(body: &PermissionOverride) -> Result<()> {
    let spec = crate::capabilities::bundled_schema();
    let allowed =
        &spec["components"]["schemas"]["Override"]["properties"]["allow"]["items"]["enum"];
    if body.allow.iter().chain(&body.deny).any(|key| {
        !allowed
            .as_array()
            .is_some_and(|a| a.contains(&serde_json::json!(key)))
    }) {
        return Err(YappinError::validation(
            "permissions",
            "unsupported resource override permission",
        ));
    }
    Ok(())
}

fn required_channel<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> std::result::Result<Option<Uuid>, D::Error> {
    Option::<Uuid>::deserialize(de)
}
