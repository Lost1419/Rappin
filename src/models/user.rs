use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::utils::{avatar_url, banner_url};

/// A public user profile on Yapin'.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_path.as_deref().map(avatar_url)
    }

    /// Formats a mention string for this user (`<@id>`).
    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// Detailed user profile returned from `GET /users/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_path.as_deref().map(avatar_url)
    }

    /// Returns the full banner URL if set.
    pub fn banner_url(&self) -> Option<String> {
        self.banner_path.as_deref().map(banner_url)
    }

    /// Formats a mention string for this user (`<@id>`).
    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// The profile of the authenticated bot account (`GET /me`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Bot bio / description.
    #[serde(default)]
    pub bio: Option<String>,
}

impl BotProfile {
    /// Returns the full avatar URL if set.
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_path.as_deref().map(avatar_url)
    }

    /// Returns the full banner URL if set.
    pub fn banner_url(&self) -> Option<String> {
        self.banner_path.as_deref().map(banner_url)
    }

    /// Formats a mention string for this bot (`<@id>`).
    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// Request payload to update bot profile (`PATCH /me`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateBotProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
}

impl UpdateBotProfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    pub fn bio(mut self, bio: impl Into<String>) -> Self {
        self.bio = Some(bio.into());
        self
    }
}

/// Bot online status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BotStatus {
    Online,
    Idle,
    Dnd,
}

/// Status and activity update payload (`PUT /me/status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusUpdateRequest {
    pub status: BotStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
}

impl StatusUpdateRequest {
    pub fn new(status: BotStatus) -> Self {
        Self {
            status,
            activity: None,
        }
    }

    pub fn with_activity(mut self, activity: impl Into<String>) -> Self {
        self.activity = Some(activity.into());
        self
    }
}

/// Uploaded image response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub path: String,
    pub url: String,
}

