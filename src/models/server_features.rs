//! # Example
//!
//! ```rust
//! let rule = rappin::PermissionOverride { allow: vec!["view_channel".into()], deny: vec!["send_messages".into()] };
//! assert_eq!(rule.deny, ["send_messages"]);
//! ```
//! Test-server APIs documented in the Yapin developer guide.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Permission keys are server-defined. Omitted keys in a PATCH stay unchanged.
/// # Example
///
/// ```rust
/// let grants: rappin::Permissions = [("manage_messages".into(), true)].into();
/// assert_eq!(grants.get("manage_messages"), Some(&true));
/// ```
pub type Permissions = BTreeMap<String, bool>;

/// Payload for creating a role. Hierarchy and permission checks belong to the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let mut role = rappin::CreateRole::new("Moderator");
/// role.permissions = Some([("manage_messages".into(), true)].into());
/// assert_eq!(role.name, "Moderator");
/// ```
pub struct CreateRole {
    /// New role display name.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional platform color string.
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional flag allowing role mentions.
    pub mentionable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Explicit permission grants for the new role; the API limits grants to the bot's authority.
    pub permissions: Option<Permissions>,
}
impl CreateRole {
    /// Create a role request containing only its name. Set optional permissions explicitly before sending.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut role = rappin::CreateRole::new("Moderator");
    /// role.permissions = Some([("manage_messages".into(), true)].into());
    /// assert_eq!(role.name, "Moderator");
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: None,
            mentionable: None,
            permissions: None,
        }
    }
}

/// Only supplied role fields and permission keys are changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let patch = rappin::EditRole::default();
/// assert_eq!(serde_json::to_value(patch).unwrap(), serde_json::json!({}));
/// ```
pub struct EditRole {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement role display name, when supplied.
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement role color, when supplied.
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement mentionability setting, when supplied.
    pub mentionable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Permission keys to merge into the existing role.
    pub permissions: Option<Permissions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Requested platform role position; supplying this field does not establish hierarchy authorization.
    pub position: Option<i32>,
}

/// An override targets a UUID role or the literal `everyone`, never an arbitrary path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// # Example
///
/// ```rust
/// let target: rappin::PermissionTarget = "everyone".parse().unwrap();
/// assert_eq!(target, rappin::PermissionTarget::Everyone);
/// assert_eq!(target.to_string(), "everyone");
/// ```
pub enum PermissionTarget {
    /// The platform everyone target, serialized in paths as `everyone`.
    Everyone,
    /// A role identified by UUID.
    Role(#[doc = "Role identifier used as the permission target."] Uuid),
}
impl std::fmt::Display for PermissionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Everyone => f.write_str("everyone"),
            Self::Role(id) => id.fmt(f),
        }
    }
}
impl std::str::FromStr for PermissionTarget {
    type Err = uuid::Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "everyone" {
            Ok(Self::Everyone)
        } else {
            value.parse().map(Self::Role)
        }
    }
}

/// Empty lists clear an override. Permission names remain extensible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let rule = rappin::PermissionOverride { allow: vec!["view_channel".into()], deny: vec!["send_messages".into()] };
/// assert_eq!(rule.deny, ["send_messages"]);
/// ```
pub struct PermissionOverride {
    #[serde(default)]
    /// Permission names explicitly allowed by this override.
    pub allow: Vec<String>,
    #[serde(default)]
    /// Permission names explicitly denied by this override. Platform precedence is a separate contract.
    pub deny: Vec<String>,
}

/// A channel override; a null role_id means @everyone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let rule: rappin::ChannelPermissionOverride = serde_json::from_str(r#"{"role_id":null,"allow":[],"deny":[]}"#).unwrap();
/// assert!(rule.role_id.is_none());
/// ```
pub struct ChannelPermissionOverride {
    /// Target role identifier; null represents the everyone target.
    pub role_id: Option<Uuid>,
    #[serde(flatten)]
    /// Flattened allow/deny lists for this target.
    pub permissions: PermissionOverride,
}

/// Live counts; online_count is refreshed by Yapin roughly once a minute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let counts = rappin::MemberCounts { server_id: uuid::Uuid::nil(), member_count: 100, online_count: 12 };
/// assert!(counts.online_count <= counts.member_count);
/// ```
pub struct MemberCounts {
    /// Server whose counts were requested.
    pub server_id: Uuid,
    /// Total membership reported by the API.
    pub member_count: u64,
    /// Online count reported by the API; it is an approximate snapshot rather than a live subscription.
    pub online_count: u64,
}

/// Invite information visible to bots with Manage Server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let invite: rappin::Invite = serde_json::from_str(r#"{"code":"example"}"#).unwrap();
/// assert_eq!(invite.uses, 0);
/// ```
pub struct Invite {
    /// Invite code.
    pub code: String,
    #[serde(default)]
    /// Absolute invite URL when supplied.
    pub url: Option<String>,
    #[serde(default)]
    /// Creating user, when visible.
    pub inviter_id: Option<Uuid>,
    #[serde(default)]
    /// Uses recorded by the API.
    pub uses: u64,
    #[serde(default)]
    /// Use limit recorded by the API.
    pub max_uses: u64,
    #[serde(default)]
    /// UTC expiration time, when supplied.
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Whether the API identifies this as a vanity invite.
    pub vanity: bool,
    #[serde(default)]
    /// UTC creation time, when supplied.
    pub created_at: Option<DateTime<Utc>>,
}

/// Scheduled server event. Status and repeat remain strings so new values decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let event: rappin::ServerEvent = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"title":"Game night","starts_at":"2026-11-08T20:00:00Z","status":"future-platform-status"})).unwrap();
/// assert_eq!(event.status.as_deref(), Some("future-platform-status"));
/// ```
pub struct ServerEvent {
    /// Scheduled-event identifier.
    pub id: Uuid,
    /// Event heading.
    pub title: String,
    #[serde(default)]
    /// Optional event details.
    pub description: Option<String>,
    /// UTC event start time.
    pub starts_at: DateTime<Utc>,
    #[serde(default)]
    /// Optional UTC event end time.
    pub ends_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Associated voice channel, if the event uses one.
    pub channel_id: Option<Uuid>,
    #[serde(default)]
    /// External location, if the event uses one.
    pub location: Option<String>,
    #[serde(default)]
    /// Channel selected for announcements, when supplied.
    pub announce_channel_id: Option<Uuid>,
    #[serde(default)]
    /// Platform repeat value, retained as a string for forward compatibility.
    pub repeat: Option<String>,
    #[serde(default)]
    /// Platform status value, retained as a string for forward compatibility.
    pub status: Option<String>,
    #[serde(default)]
    /// Interested-member count reported by the API.
    pub interested_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Repeat rule accepted by event-creation payloads.
///
/// # Example
///
/// ```rust
/// assert_eq!(serde_json::to_value(rappin::EventRepeat::Weekly).unwrap(), "weekly");
/// ```
pub enum EventRepeat {
    /// Do not repeat the event.
    None,
    /// Repeat the event weekly.
    Weekly,
}

/// Create a scheduled event with a voice channel or an external location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let mut event = rappin::CreateServerEvent::new("Game night", chrono::Utc::now());
/// event.location = Some("https://example.com/game".into());
/// event.validate().unwrap();
/// ```
pub struct CreateServerEvent {
    /// Nonempty event heading.
    pub title: String,
    /// UTC event start time.
    pub starts_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional end time; validation requires it to be later than starts_at.
    pub ends_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Voice channel for the event; mutually exclusive with a nonempty external location.
    pub channel_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// External location; mutually exclusive with a voice channel.
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional destination for event announcements.
    pub announce_channel_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional supported repeat rule.
    pub repeat: Option<EventRepeat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional platform timezone identifier.
    pub tz: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional event details.
    pub description: Option<String>,
}
impl CreateServerEvent {
    /// Create a scheduled-event body without a location. Choose either a voice channel or a nonempty external location before validating.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut event = rappin::CreateServerEvent::new("Game night", chrono::Utc::now());
    /// event.location = Some("https://example.com/game".into());
    /// event.validate().unwrap();
    /// ```
    pub fn new(title: impl Into<String>, starts_at: DateTime<Utc>) -> Self {
        Self {
            title: title.into(),
            starts_at,
            ends_at: None,
            channel_id: None,
            location: None,
            announce_channel_id: None,
            repeat: None,
            tz: None,
            description: None,
        }
    }
    /// Check the nonempty title, exactly one location form and end-after-start ordering. Permission and rollout checks remain server-side.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut event = rappin::CreateServerEvent::new("Game night", chrono::Utc::now());
    /// event.location = Some("https://example.com/game".into());
    /// event.validate().unwrap();
    /// event.ends_at = Some(event.starts_at - chrono::Duration::seconds(1));
    /// assert!(event.validate().is_err());
    /// ```
    pub fn validate(&self) -> crate::Result<()> {
        if self.title.trim().is_empty() {
            return Err(crate::Error::validation("title", "must not be empty"));
        }
        if self.channel_id.is_some() == self.location.as_ref().is_some_and(|s| !s.trim().is_empty())
        {
            return Err(crate::Error::validation(
                "location",
                "provide either a voice channel or a nonempty external location",
            ));
        }
        if self.ends_at.is_some_and(|end| end <= self.starts_at) {
            return Err(crate::Error::validation(
                "ends_at",
                "must be after starts_at",
            ));
        }
        Ok(())
    }
}
