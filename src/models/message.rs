use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::component::{ActionRow, Component};
use crate::models::embed::Embed;
use crate::models::user::User;
use crate::utils::{attachment_url, IS_COMPONENTS_V2};

/// A channel message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Message ID.
    pub id: Uuid,
    /// Channel ID where message was posted.
    pub channel_id: Uuid,
    /// Author user ID.
    pub author_id: Uuid,
    /// Message content.
    #[serde(default)]
    pub content: String,
    /// Rich embed cards attached to the message.
    #[serde(default)]
    pub embeds: Option<Vec<Embed>>,
    /// Interactive button rows or Components V2 elements.
    #[serde(default)]
    pub components: Option<Vec<Component>>,
    /// Message post timestamp.
    pub created_at: DateTime<Utc>,
    /// Last edit timestamp, if edited.
    #[serde(default)]
    pub edited_at: Option<DateTime<Utc>>,
    /// Path to uploaded file/attachment on CDN.
    #[serde(default)]
    pub attachment_path: Option<String>,
    /// Referenced message ID if this is a reply.
    #[serde(default)]
    pub reply_to_id: Option<Uuid>,
    /// Timestamp when message was pinned, if pinned.
    #[serde(default)]
    pub pinned_at: Option<DateTime<Utc>>,
}

impl Message {
    /// Returns the full attachment URL if set.
    pub fn attachment_url(&self) -> Option<String> {
        self.attachment_path.as_deref().map(attachment_url)
    }

    /// Whether this message is pinned in its channel.
    pub fn is_pinned(&self) -> bool {
        self.pinned_at.is_some()
    }

    /// Formats a mention string for the message author (`<@id>`).
    pub fn author_mention(&self) -> String {
        format!("<@{}>", self.author_id)
    }
}

/// Request payload to create a new message (`POST /channels/{id}/messages`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}

impl CreateMessage {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            reply_to: None,
            embeds: None,
            components: None,
            flags: None,
        }
    }

    pub fn reply(content: impl Into<String>, reply_to: Uuid) -> Self {
        Self::new(content).with_reply_to(reply_to)
    }

    /// Creates a Components V2 layout message (sets flag 32768, components is the layout).
    pub fn layout(
        components: impl IntoIterator<Item = impl Into<Component>>,
        reply_to: Option<Uuid>,
    ) -> Self {
        Self {
            content: None,
            reply_to,
            embeds: None,
            components: Some(components.into_iter().map(Into::into).collect()),
            flags: Some(IS_COMPONENTS_V2),
        }
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    pub fn with_reply_to(mut self, reply_to: Uuid) -> Self {
        self.reply_to = Some(reply_to);
        self
    }

    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }

    pub fn component(mut self, component: impl Into<Component>) -> Self {
        self.components
            .get_or_insert_with(Vec::new)
            .push(component.into());
        self
    }

    pub fn action_row(self, row: ActionRow) -> Self {
        self.component(Component::Row(row))
    }

    pub fn flags(mut self, flags: u32) -> Self {
        self.flags = Some(flags);
        self
    }
}

/// Request payload to edit an existing message (`PATCH /channels/{id}/messages/{mid}`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EditMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Option<Vec<Embed>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Option<Vec<Component>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}

impl EditMessage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    pub fn embeds(mut self, embeds: Vec<Embed>) -> Self {
        self.embeds = Some(Some(embeds));
        self
    }

    pub fn clear_embeds(mut self) -> Self {
        self.embeds = Some(None);
        self
    }

    pub fn components(mut self, components: Vec<Component>) -> Self {
        self.components = Some(Some(components));
        self
    }

    pub fn clear_components(mut self) -> Self {
        self.components = Some(None);
        self
    }

    pub fn layout(components: impl IntoIterator<Item = impl Into<Component>>) -> Self {
        Self {
            content: None,
            embeds: None,
            components: Some(Some(components.into_iter().map(Into::into).collect())),
            flags: Some(IS_COMPONENTS_V2),
        }
    }
}

/// A reaction on a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub id: Uuid,
    pub user_id: Uuid,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

/// Request payload for bulk deleting messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkDeleteRequest {
    pub ids: Vec<Uuid>,
}

impl BulkDeleteRequest {
    pub fn new(ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            ids: ids.into_iter().collect(),
        }
    }
}

/// Response returned from bulk delete operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkDeleteResponse {
    pub deleted: Vec<Uuid>,
    pub skipped: Vec<Uuid>,
}

/// Response returned by `GET /channels/{id}/messages`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagesResponse {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub authors: Vec<User>,
}

/// Response returned by `GET /channels/{id}/messages/{mid}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageDetailResponse {
    pub message: Message,
    #[serde(default)]
    pub author: Option<User>,
}
