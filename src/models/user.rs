//! Public user profiles, bot ownership metadata, profile patches and presence payloads.
//!
//! # Example
//!
//! ```rust
//! let patch = rappin::UpdateBotProfile::new().display_name("Helper").bio("Server assistant");
//! assert_eq!(patch.display_name.as_deref(), Some("Helper"));
//! ```
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::utils::{avatar_url, banner_url};

/// A public user profile on Yapin'.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let mut user: rappin::User = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
/// assert_eq!(user.username, "river");
/// assert_eq!(user.mention(), format!("<@{}>", user.id));
/// ```
pub struct User {
    /// Unique user ID.
    pub id: Uuid,
    /// Unique username.
    pub username: String,
    /// Optional display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Path to user avatar on CDN.
    #[serde(default)]
    pub avatar_path: Option<String>,
    /// Badges earned by the user.
    #[serde(default)]
    pub badges: Vec<String>,
}

impl User {
    /// Returns the full avatar URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut user: rappin::User = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert!(user.avatar_url().is_none());
    /// user.avatar_path = Some("profile.png".into());
    /// assert!(user.avatar_url().unwrap().ends_with("profile.png"));
    /// ```
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_path.as_deref().map(avatar_url)
    }

    /// Formats a mention string for this user (`<@id>`).
    /// # Example
    ///
    /// ```rust
    /// let mut user: rappin::User = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert_eq!(user.mention(), format!("<@{}>", user.id));
    /// ```
    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// Detailed user profile returned from `GET /users/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let mut user: rappin::UserDetail = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
/// assert_eq!(user.username, "river");
/// assert_eq!(user.mention(), format!("<@{}>", user.id));
/// ```
pub struct UserDetail {
    /// Unique user ID.
    pub id: Uuid,
    /// Unique username.
    pub username: String,
    /// Optional display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Path to user avatar on CDN.
    #[serde(default)]
    pub avatar_path: Option<String>,
    /// Badges earned by the user.
    #[serde(default)]
    pub badges: Vec<String>,
    /// Path to profile banner image on CDN.
    #[serde(default)]
    pub banner_path: Option<String>,
    /// Account creation timestamp.
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// User biography.
    #[serde(default)]
    pub bio: Option<String>,
    /// User pronouns.
    #[serde(default)]
    pub pronouns: Option<String>,
}

impl UserDetail {
    /// Returns the full avatar URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut user: rappin::UserDetail = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert!(user.avatar_url().is_none());
    /// user.avatar_path = Some("profile.png".into());
    /// assert!(user.avatar_url().unwrap().ends_with("profile.png"));
    /// ```
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_path.as_deref().map(avatar_url)
    }

    /// Returns the full banner URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut user: rappin::UserDetail = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert!(user.banner_url().is_none());
    /// user.banner_path = Some("banner.png".into());
    /// assert!(user.banner_url().unwrap().ends_with("banner.png"));
    /// ```
    pub fn banner_url(&self) -> Option<String> {
        self.banner_path.as_deref().map(banner_url)
    }

    /// Formats a mention string for this user (`<@id>`).
    /// # Example
    ///
    /// ```rust
    /// let mut user: rappin::UserDetail = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert_eq!(user.mention(), format!("<@{}>", user.id));
    /// ```
    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// The profile of the authenticated bot account (`GET /me`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let mut bot: rappin::BotProfile = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
/// assert_eq!(bot.username, "river");
/// assert_eq!(bot.mention(), format!("<@{}>", bot.id));
/// ```
pub struct BotProfile {
    /// Unique bot ID.
    pub id: Uuid,
    /// Unique username.
    pub username: String,
    /// Optional display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Avatar image path.
    #[serde(default)]
    pub avatar_path: Option<String>,
    /// Banner image path.
    #[serde(default)]
    pub banner_path: Option<String>,
    /// Absolute image URLs supplied by GET /me.
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    /// Absolute banner URL returned by the API; preferred over banner_path by the helper.
    pub banner_url: Option<String>,
    #[serde(default)]
    /// UTC creation time of the bot account, when supplied.
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Bot owner and publication metadata, when expanded.
    pub bot: Option<BotMetadata>,
    /// Bot bio / description.
    #[serde(default)]
    pub bio: Option<String>,
}

impl BotProfile {
    /// Returns the full avatar URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut bot: rappin::BotProfile = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert!(bot.avatar_url().is_none());
    /// bot.avatar_path = Some("profile.png".into());
    /// assert!(bot.avatar_url().unwrap().ends_with("profile.png"));
    /// ```
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_url
            .clone()
            .or_else(|| self.avatar_path.as_deref().map(avatar_url))
    }

    /// Returns the full banner URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut bot: rappin::BotProfile = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert!(bot.banner_url().is_none());
    /// bot.banner_path = Some("banner.png".into());
    /// assert!(bot.banner_url().unwrap().ends_with("banner.png"));
    /// ```
    pub fn banner_url(&self) -> Option<String> {
        self.banner_url
            .clone()
            .or_else(|| self.banner_path.as_deref().map(banner_url))
    }

    /// Formats a mention string for this bot (`<@id>`).
    /// # Example
    ///
    /// ```rust
    /// let mut bot: rappin::BotProfile = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"username":"river"})).unwrap();
    /// assert_eq!(bot.mention(), format!("<@{}>", bot.id));
    /// ```
    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// Request payload to update bot profile (`PATCH /me`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let patch = rappin::UpdateBotProfile::new().display_name("Helper").bio("Server assistant");
/// assert_eq!(patch.display_name.as_deref(), Some("Helper"));
/// ```
pub struct UpdateBotProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement display name, limited to 32 characters by profile validation.
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement profile biography, limited to 300 characters by profile validation.
    pub bio: Option<String>,
}

impl UpdateBotProfile {
    /// Create an empty profile patch. Omitted fields keep their current platform values.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::UpdateBotProfile::new();
    /// assert_eq!(serde_json::to_value(patch).unwrap(), serde_json::json!({}));
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the replacement display name.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::UpdateBotProfile::new().display_name("Helper").bio("Server assistant");
    /// assert_eq!(patch.display_name.as_deref(), Some("Helper"));
    /// ```
    pub fn display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set the replacement profile biography.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::UpdateBotProfile::new().bio("Community helper");
    /// assert_eq!(patch.bio.as_deref(), Some("Community helper"));
    /// ```
    pub fn bio(mut self, bio: impl Into<String>) -> Self {
        self.bio = Some(bio.into());
        self
    }
}

/// Bot online status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// # Example
///
/// ```rust
/// assert_eq!(serde_json::to_value(rappin::BotStatus::Dnd).unwrap(), "dnd");
/// ```
pub enum BotStatus {
    /// Online presence.
    Online,
    /// Idle presence.
    Idle,
    /// Do-not-disturb presence.
    Dnd,
}

/// Status and activity update payload (`PUT /me/status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let status = rappin::StatusUpdateRequest::new(rappin::BotStatus::Online);
/// assert!(status.activity.is_none());
/// ```
pub struct StatusUpdateRequest {
    /// Requested online, idle or do-not-disturb state.
    pub status: BotStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional activity line; platform validation limits it to 60 characters.
    pub activity: Option<String>,
}

impl StatusUpdateRequest {
    /// Create a presence update without an activity line.
    ///
    /// # Example
    ///
    /// ```rust
    /// let status = rappin::StatusUpdateRequest::new(rappin::BotStatus::Online);
    /// assert!(status.activity.is_none());
    /// ```
    pub fn new(status: BotStatus) -> Self {
        Self {
            status,
            activity: None,
        }
    }

    /// Set the accompanying activity line.
    ///
    /// # Example
    ///
    /// ```rust
    /// let status = rappin::StatusUpdateRequest::new(rappin::BotStatus::Idle).with_activity("Waiting for commands");
    /// assert_eq!(status.activity.as_deref(), Some("Waiting for commands"));
    /// ```
    pub fn with_activity(mut self, activity: impl Into<String>) -> Self {
        self.activity = Some(activity.into());
        self
    }
}

/// Uploaded image response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let image = rappin::Image { path: "profile.png".into(), url: "https://example.com/profile.png".into() };
/// assert!(image.url.ends_with(&image.path));
/// ```
pub struct Image {
    /// Stored image path returned by the upload endpoint.
    pub path: String,
    /// Absolute image URL returned with the stored path.
    pub url: String,
}

/// Bot ownership and publication settings returned by GET /me.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let metadata = rappin::BotMetadata { name: "Helper".into(), public: false, owner_id: uuid::Uuid::nil() };
/// assert!(!metadata.public);
/// ```
pub struct BotMetadata {
    /// Bot application name returned in its metadata.
    pub name: String,
    /// Whether the bot application is publicly listed according to the platform.
    pub public: bool,
    /// User who owns the bot application; separate from server ownership.
    pub owner_id: Uuid,
}
