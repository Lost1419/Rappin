use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::server::Category;

/// The type of channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Text,
    Voice,
}

/// A channel in a server or category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Channel ID.
    pub id: Uuid,
    /// Channel name.
    pub name: String,
    /// Channel type ("text" or "voice").
    #[serde(alias = "type")]
    pub kind: ChannelKind,
    /// Associated server ID.
    #[serde(default)]
    pub server_id: Option<Uuid>,
    /// Channel position in listing.
    #[serde(default)]
    pub position: Option<i32>,
    /// Associated category, if expanded.
    #[serde(default)]
    pub category: Option<Category>,
    /// Parent category ID.
    #[serde(default)]
    pub category_id: Option<Uuid>,
    /// Whether the channel is private to permitted roles.
    #[serde(default)]
    pub private: Option<bool>,
    /// Whether the channel is read-only.
    #[serde(default)]
    pub read_only: Option<bool>,
    /// Slowmode rate limit in seconds.
    #[serde(default)]
    pub slowmode: Option<u32>,
}

/// Single channel row returned from `GET /channels/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRow {
    pub id: Uuid,
    #[serde(alias = "type")]
    pub kind: ChannelKind,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub server_id: Option<Uuid>,
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub private: Option<bool>,
    #[serde(default)]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub slowmode: Option<u32>,
}

/// Payload for creating a channel (`POST /servers/{id}/channels`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ChannelKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<Uuid>,
}

impl CreateChannelRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: None,
            category_id: None,
        }
    }

    pub fn text(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: Some(ChannelKind::Text),
            category_id: None,
        }
    }

    pub fn voice(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: Some(ChannelKind::Voice),
            category_id: None,
        }
    }

    pub fn in_category(mut self, category_id: Uuid) -> Self {
        self.category_id = Some(category_id);
        self
    }
}

/// Payload for modifying an existing channel (`PATCH /channels/{id}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditChannelRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<Option<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_ids: Option<Vec<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_role_ids: Option<Vec<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slowmode: Option<u32>,
}

impl EditChannelRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn category_id(mut self, cat_id: Option<Uuid>) -> Self {
        self.category_id = Some(cat_id);
        self
    }

    pub fn private(mut self, private: bool) -> Self {
        self.private = Some(private);
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = Some(read_only);
        self
    }

    pub fn slowmode(mut self, slowmode_seconds: u32) -> Self {
        self.slowmode = Some(slowmode_seconds);
        self
    }
}

/// Response returned by `GET /servers/{id}/channels`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerChannelsResponse {
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub categories: Vec<Category>,
}
