use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::user::User;
use crate::utils::icon_url;

/// A Yapin' server (guild).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// Unique server ID.
    pub id: Uuid,
    /// Server name.
    pub name: String,
    /// Path to server icon on CDN.
    #[serde(default)]
    pub icon_path: Option<String>,
    /// Owner user ID.
    pub owner_id: Uuid,
    /// Approximate member count.
    #[serde(default)]
    pub member_count: Option<u64>,
}

impl Server {
    /// Returns the full icon URL if set.
    pub fn icon_url(&self) -> Option<String> {
        self.icon_path.as_deref().map(icon_url)
    }
}

/// A category grouping channels in a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    /// Category ID.
    pub id: Uuid,
    /// Category name.
    pub name: String,
    /// Ordering position.
    #[serde(default)]
    pub position: Option<i32>,
}

/// Request to create a category in a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

impl CreateCategoryRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            position: None,
        }
    }

    pub fn with_position(mut self, position: i32) -> Self {
        self.position = Some(position);
        self
    }
}

/// A server role with permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Role ID.
    pub id: Uuid,
    /// Role name.
    pub name: String,
    /// Color hex code (e.g. `#5865f2`).
    #[serde(default)]
    pub color: Option<String>,
    /// Role position in hierarchy.
    #[serde(default)]
    pub position: Option<i32>,
    /// Whether the role has server management permission.
    #[serde(default)]
    pub manage_server: Option<bool>,
    /// Whether the role has message management permission.
    #[serde(default)]
    pub manage_messages: Option<bool>,
    /// Additional permissions dictionary.
    #[serde(default)]
    pub perms: Option<serde_json::Value>,
    /// System or automatic role indicator.
    #[serde(default)]
    pub auto: Option<String>,
}

/// A member of a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// User profile data.
    #[serde(flatten)]
    pub user: User,
    /// Server-specific nickname.
    #[serde(default)]
    pub nickname: Option<String>,
    /// When the member joined the server.
    #[serde(default)]
    pub joined_at: Option<DateTime<Utc>>,
    /// List of role IDs assigned to the member.
    #[serde(default)]
    pub roles: Vec<Uuid>,
}

impl Member {
    /// Returns the nickname if present, otherwise user display name or username.
    pub fn effective_name(&self) -> &str {
        if let Some(ref nick) = self.nickname {
            nick
        } else if let Some(ref disp) = self.user.display_name {
            disp
        } else {
            &self.user.username
        }
    }
}

/// Detailed member information (`GET /servers/{id}/members/{uid}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberDetail {
    /// Base member data.
    #[serde(flatten)]
    pub member: Member,
    /// Active timeout details, if timed out.
    #[serde(default)]
    pub timeout: Option<serde_json::Value>,
}

/// A banned user entry (`GET /servers/{id}/bans`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ban {
    pub user_id: Uuid,
    #[serde(default)]
    pub banned_by: Option<Uuid>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub user: Option<User>,
}

/// Request payload to ban a user (`POST /servers/{id}/bans`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BanRequest {
    pub user_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl BanRequest {
    pub fn new(user_id: Uuid) -> Self {
        Self {
            user_id,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Request payload to timeout a user (`POST /servers/{id}/timeouts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeoutRequest {
    pub user_id: Uuid,
    /// Duration in minutes (0 clears timeout, maximum 40320).
    pub minutes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl TimeoutRequest {
    pub fn new(user_id: Uuid, minutes: u32) -> Self {
        Self {
            user_id,
            minutes,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Generic response indicating a successful operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}
