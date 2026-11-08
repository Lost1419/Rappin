//! Server resources, role snapshots, member profiles and moderation payloads. Platform mutation authorization remains independent of constructing these values.
//!
//! # Example
//!
//! ```rust
//! let body = rappin::CreateCategoryRequest::new("Information").with_position(0);
//! assert_eq!(body.position, Some(0));
//! ```
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::user::User;
use crate::utils::icon_url;

/// A Yapin' server (guild).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let mut server: rappin::Server = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"Community","owner_id":uuid::Uuid::nil()})).unwrap();
/// assert_eq!(server.name, "Community");
/// ```
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
    /// People online now; absent where live counts are not enabled.
    #[serde(default)]
    pub online_count: Option<u64>,
}

impl Server {
    /// Returns the full icon URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut server: rappin::Server = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"Community","owner_id":uuid::Uuid::nil()})).unwrap();
    /// assert!(server.icon_url().is_none());
    /// server.icon_path = Some("server.png".into());
    /// assert!(server.icon_url().unwrap().ends_with("server.png"));
    /// ```
    pub fn icon_url(&self) -> Option<String> {
        self.icon_path.as_deref().map(icon_url)
    }
}

/// A category grouping channels in a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::Category = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"Information"})).unwrap();
/// assert_eq!(value.name, "Information");
/// ```
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
/// # Example
///
/// ```rust
/// let body = rappin::CreateCategoryRequest::new("Information").with_position(0);
/// assert_eq!(body.position, Some(0));
/// ```
pub struct CreateCategoryRequest {
    /// Display name for the new category.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Requested category-list position, if supplied.
    pub position: Option<i32>,
}

impl CreateCategoryRequest {
    /// Create a category payload without a requested position.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateCategoryRequest::new("Information");
    /// assert!(body.position.is_none());
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            position: None,
        }
    }

    /// Set the requested category-list position.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateCategoryRequest::new("Information").with_position(0);
    /// assert_eq!(body.position, Some(0));
    /// ```
    pub fn with_position(mut self, position: i32) -> Self {
        self.position = Some(position);
        self
    }
}

/// Request to rename a category in a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let patch = rappin::EditCategoryRequest { name: "Community".into() };
/// assert_eq!(patch.name, "Community");
/// ```
pub struct EditCategoryRequest {
    /// Replacement category display name.
    pub name: String,
}

/// A server role with permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::Role = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"Moderator"})).unwrap();
/// assert_eq!(value.position, None);
/// ```
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
    #[serde(default)]
    /// Whether the role can be mentioned, when supplied by the platform.
    pub mentionable: Option<bool>,
    /// Permission dictionary returned by newer role endpoints.
    #[serde(default)]
    pub permissions: Option<super::Permissions>,
}

/// A member of a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::Member = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river","nickname":"River"})).unwrap();
/// assert_eq!(value.effective_name(), "River");
/// ```
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
    /// # Example
    ///
    /// ```rust
    /// let mut member: rappin::Member = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert_eq!(member.effective_name(), "river");
    /// member.nickname = Some("River".into());
    /// assert_eq!(member.effective_name(), "River");
    /// ```
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
/// # Example
///
/// ```rust
/// let value: rappin::MemberDetail = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
/// assert_eq!(value.inviter_id, None);
/// ```
pub struct MemberDetail {
    /// Base member data.
    #[serde(flatten)]
    pub member: Member,
    /// Active timeout details, if timed out.
    #[serde(default)]
    pub timeout: Option<serde_json::Value>,
    /// Available only with Manage Server where invite tracking is enabled.
    #[serde(default)]
    pub invite_code: Option<String>,
    #[serde(default)]
    /// Inviting user, when invite attribution is visible with the required server permission.
    pub inviter_id: Option<Uuid>,
}

/// A banned user entry (`GET /servers/{id}/bans`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::Ban = serde_json::from_value(serde_json::json!({"user_id":uuid::Uuid::nil()})).unwrap();
/// assert_eq!(value.user, None);
/// ```
pub struct Ban {
    /// Banned user identifier.
    pub user_id: Uuid,
    #[serde(default)]
    /// Actor recorded as creating the ban, when supplied.
    pub banned_by: Option<Uuid>,
    #[serde(default)]
    /// UTC ban creation time, when supplied.
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Expanded banned-user profile, when supplied.
    pub user: Option<User>,
}

/// Request payload to ban a user (`POST /servers/{id}/bans`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let body = rappin::BanRequest::new(uuid::Uuid::nil()).with_reason("Repeated spam");
/// assert_eq!(body.reason.as_deref(), Some("Repeated spam"));
/// ```
pub struct BanRequest {
    /// User to ban from the selected server.
    pub user_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional moderation reason recorded with the ban.
    pub reason: Option<String>,
}

impl BanRequest {
    /// Create a moderation request body; construction alone performs no authorization or API mutation.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::BanRequest::new(uuid::Uuid::nil()).with_reason("Repeated spam");
    /// assert_eq!(body.reason.as_deref(), Some("Repeated spam"));
    /// ```
    pub fn new(user_id: Uuid) -> Self {
        Self {
            user_id,
            reason: None,
        }
    }

    /// Set the moderation reason included in the request.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::BanRequest::new(uuid::Uuid::nil()).with_reason("Repeated spam");
    /// assert_eq!(body.reason.as_deref(), Some("Repeated spam"));
    /// ```
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Request payload to timeout a user (`POST /servers/{id}/timeouts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let body = rappin::TimeoutRequest::new(uuid::Uuid::nil(), 10).with_reason("Flooding");
/// assert_eq!(body.minutes, 10);
/// ```
pub struct TimeoutRequest {
    /// Member whose timeout is changed.
    pub user_id: Uuid,
    /// Duration in minutes (0 clears timeout, maximum 40320).
    pub minutes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional moderation reason accompanying the timeout change.
    pub reason: Option<String>,
}

impl TimeoutRequest {
    /// Create a moderation request body; construction alone performs no authorization or API mutation.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::TimeoutRequest::new(uuid::Uuid::nil(), 10).with_reason("Flooding");
    /// assert_eq!(body.minutes, 10);
    /// ```
    pub fn new(user_id: Uuid, minutes: u32) -> Self {
        Self {
            user_id,
            minutes,
            reason: None,
        }
    }

    /// Set the moderation reason included in the request.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::TimeoutRequest::new(uuid::Uuid::nil(), 10).with_reason("Flooding");
    /// assert_eq!(body.minutes, 10);
    /// ```
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Generic response indicating a successful operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::OkResponse = serde_json::from_value(serde_json::json!({"ok":true})).unwrap();
/// assert_eq!(value.ok, true);
/// ```
pub struct OkResponse {
    /// Platform success flag; this does not reconcile a different timed-out mutation.
    pub ok: bool,
}
