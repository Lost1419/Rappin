//! Test-server APIs documented in the Yapin developer guide.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Permission keys are server-defined. Omitted keys in a PATCH stay unchanged.
pub type Permissions = BTreeMap<String, bool>;

/// Payload for creating a role. Hierarchy and permission checks belong to the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRole {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentionable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
}
impl CreateRole {
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
pub struct EditRole {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentionable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// An override targets a UUID role or the literal `everyone`, never an arbitrary path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionTarget {
    Everyone,
    Role(Uuid),
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
pub struct PermissionOverride {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// A channel override; a null role_id means @everyone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPermissionOverride {
    pub role_id: Option<Uuid>,
    #[serde(flatten)]
    pub permissions: PermissionOverride,
}

/// Live counts; online_count is refreshed by Yapin roughly once a minute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberCounts {
    pub server_id: Uuid,
    pub member_count: u64,
    pub online_count: u64,
}

/// Invite information visible to bots with Manage Server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub inviter_id: Option<Uuid>,
    #[serde(default)]
    pub uses: u64,
    #[serde(default)]
    pub max_uses: u64,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub vanity: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

/// Scheduled server event. Status and repeat remain strings so new values decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEvent {
    pub id: Uuid,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    #[serde(default)]
    pub ends_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub channel_id: Option<Uuid>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub announce_channel_id: Option<Uuid>,
    #[serde(default)]
    pub repeat: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub interested_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventRepeat {
    None,
    Weekly,
}

/// Create a scheduled event with a voice channel or an external location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateServerEvent {
    pub title: String,
    pub starts_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce_channel_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<EventRepeat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
impl CreateServerEvent {
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
    pub fn validate(&self) -> crate::Result<()> {
        if self.title.trim().is_empty() {
            return Err(crate::YappinError::validation("title", "must not be empty"));
        }
        if self.channel_id.is_some() == self.location.as_ref().is_some_and(|s| !s.trim().is_empty())
        {
            return Err(crate::YappinError::validation(
                "location",
                "provide either a voice channel or a nonempty external location",
            ));
        }
        if self.ends_at.is_some_and(|end| end <= self.starts_at) {
            return Err(crate::YappinError::validation(
                "ends_at",
                "must be after starts_at",
            ));
        }
        Ok(())
    }
}
