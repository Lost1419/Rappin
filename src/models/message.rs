//! Channel message models and create/edit payloads. Patch fields distinguish omission from explicit clearing.
//!
//! # Example
//!
//! ```rust
//! let patch = rappin::EditMessage::new().clear_embeds();
//! assert_eq!(serde_json::to_value(patch).unwrap(), serde_json::json!({"embeds":null}));
//! ```
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::component::{ActionRow, Component};
use crate::models::embed::Embed;
use crate::models::user::User;
use crate::utils::{attachment_url, IS_COMPONENTS_V2};

/// A channel message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let mut message = rappin::Message::default();
/// message.content = "Hello".into();
/// assert_eq!(message.content, "Hello");
/// assert!(!message.is_pinned());
/// ```
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
    #[serde(default)]
    /// Original attachment filename supplied by the API.
    pub attachment_name: Option<String>,
    #[serde(default)]
    /// Attachment size in bytes, when supplied.
    pub attachment_size: Option<u64>,
    /// Referenced message ID if this is a reply.
    #[serde(default)]
    pub reply_to_id: Option<Uuid>,
    /// Timestamp when message was pinned, if pinned.
    #[serde(default)]
    pub pinned_at: Option<DateTime<Utc>>,
}

impl Message {
    /// Returns the full attachment URL if set.
    /// # Example
    ///
    /// ```rust
    /// let mut message = rappin::Message::default();
    /// assert!(message.attachment_url().is_none());
    /// message.attachment_path = Some("report.txt".into());
    /// assert!(message.attachment_url().unwrap().ends_with("report.txt"));
    /// ```
    pub fn attachment_url(&self) -> Option<String> {
        self.attachment_path.as_deref().map(attachment_url)
    }

    /// Whether this message is pinned in its channel.
    /// # Example
    ///
    /// ```rust
    /// let mut message = rappin::Message::default();
    /// assert!(!message.is_pinned());
    /// message.pinned_at = Some(chrono::Utc::now());
    /// assert!(message.is_pinned());
    /// ```
    pub fn is_pinned(&self) -> bool {
        self.pinned_at.is_some()
    }

    /// Formats a mention string for the message author (`<@id>`).
    /// # Example
    ///
    /// ```rust
    /// let message = rappin::Message::default();
    /// assert_eq!(message.author_mention(), format!("<@{}>", message.author_id));
    /// ```
    pub fn author_mention(&self) -> String {
        format!("<@{}>", self.author_id)
    }
}

/// Request payload to create a new message (`POST /channels/{id}/messages`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let body = rappin::CreateMessage::new("Hello").with_reply_to(uuid::Uuid::nil());
/// assert_eq!(body.reply_to, Some(uuid::Uuid::nil()));
/// ```
pub struct CreateMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional message text; normal message validation limits it to 2,000 characters.
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Message ID to reply to in the destination channel.
    pub reply_to: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Rich cards attached to this message, in display order.
    pub embeds: Option<Vec<Embed>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Legacy button rows or Components V2 layout components.
    pub components: Option<Vec<Component>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Message flags; 32768 selects a Components V2 layout.
    pub flags: Option<u32>,
}

impl CreateMessage {
    /// Create a plain-text message body. No request is sent until you pass it to a Client or context.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello");
    /// assert_eq!(body.content.as_deref(), Some("Hello"));
    /// ```
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            reply_to: None,
            embeds: None,
            components: None,
            flags: None,
        }
    }

    /// Create a text reply to a message in the destination channel.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::reply("Thanks", uuid::Uuid::nil());
    /// assert_eq!(body.reply_to, Some(uuid::Uuid::nil()));
    /// ```
    pub fn reply(content: impl Into<String>, reply_to: Uuid) -> Self {
        Self::new(content).with_reply_to(reply_to)
    }

    /// Creates a Components V2 layout message (sets flag 32768, components is the layout).
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::layout([rappin::TextDisplay::new("Welcome")], None);
    /// assert_eq!(body.flags, Some(rappin::IS_COMPONENTS_V2));
    /// ```
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

    /// Replace the body text.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello").with_content("Updated");
    /// assert_eq!(body.content.as_deref(), Some("Updated"));
    /// ```
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Set the message this body replies to. The caller must use the matching destination channel.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello").with_reply_to(uuid::Uuid::nil());
    /// assert_eq!(body.reply_to, Some(uuid::Uuid::nil()));
    /// ```
    pub fn with_reply_to(mut self, reply_to: Uuid) -> Self {
        self.reply_to = Some(reply_to);
        self
    }

    /// Append an embed without removing existing embeds.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello").embed(rappin::Embed::new().title("Result"));
    /// assert_eq!(body.embeds.as_ref().unwrap().len(), 1);
    /// ```
    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }

    /// Append a legacy row or layout component. The message still needs the correct layout flag for Components V2.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello").component(rappin::ActionRow::new([rappin::Button::primary("Next", "next")]));
    /// assert_eq!(body.components.as_ref().unwrap().len(), 1);
    /// ```
    pub fn component(mut self, component: impl Into<Component>) -> Self {
        self.components
            .get_or_insert_with(Vec::new)
            .push(component.into());
        self
    }

    /// Append a row of buttons.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello").action_row(rappin::ActionRow::new([rappin::Button::primary("Next", "next")]));
    /// assert_eq!(body.components.as_ref().unwrap().len(), 1);
    /// ```
    pub fn action_row(self, row: ActionRow) -> Self {
        self.component(Component::Row(row))
    }

    /// Set message flags. Prefer the layout constructor for Components V2 payloads.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello").flags(0);
    /// assert_eq!(body.flags, Some(0));
    /// ```
    pub fn flags(mut self, flags: u32) -> Self {
        self.flags = Some(flags);
        self
    }
}

/// Request payload to edit an existing message (`PATCH /channels/{id}/messages/{mid}`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let patch = rappin::EditMessage::new().clear_embeds();
/// assert_eq!(serde_json::to_value(patch).unwrap(), serde_json::json!({"embeds":null}));
/// ```
pub struct EditMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement text; omission leaves the current text unchanged. An empty string clears it.
    pub content: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    /// Embed patch: omission retains embeds, `Some(None)` clears them, and `Some(Some(values))` replaces them.
    pub embeds: Option<Option<Vec<Embed>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    /// Component patch: omission retains components, `Some(None)` clears them, and `Some(Some(values))` replaces them.
    pub components: Option<Option<Vec<Component>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Replacement message flags, when supplied.
    pub flags: Option<u32>,
}

impl EditMessage {
    /// Create an empty edit; omitted fields are left unchanged.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::new();
    /// assert_eq!(serde_json::to_value(patch).unwrap(), serde_json::json!({}));
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace message text; an empty string clears it.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::new().content("Updated");
    /// assert_eq!(patch.content.as_deref(), Some("Updated"));
    /// ```
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Replace the entire embed list.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::new().embeds(vec![rappin::Embed::new().title("Result")]);
    /// assert_eq!(patch.embeds.as_ref().unwrap().as_ref().unwrap().len(), 1);
    /// ```
    pub fn embeds(mut self, embeds: Vec<Embed>) -> Self {
        self.embeds = Some(Some(embeds));
        self
    }

    /// Explicitly clear embeds using a JSON null patch.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::new().clear_embeds();
    /// assert_eq!(patch.embeds, Some(None));
    /// ```
    pub fn clear_embeds(mut self) -> Self {
        self.embeds = Some(None);
        self
    }

    /// Replace the entire component list.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::new().components(vec![]);
    /// assert_eq!(patch.components, Some(Some(vec![])));
    /// ```
    pub fn components(mut self, components: Vec<Component>) -> Self {
        self.components = Some(Some(components));
        self
    }

    /// Explicitly clear components using a JSON null patch.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::new().clear_components();
    /// assert_eq!(patch.components, Some(None));
    /// ```
    pub fn clear_components(mut self) -> Self {
        self.components = Some(None);
        self
    }

    /// Replace the message with a Components V2 layout and set flag 32768.
    ///
    /// # Example
    ///
    /// ```rust
    /// let patch = rappin::EditMessage::layout([rappin::TextDisplay::new("Updated")]);
    /// assert_eq!(patch.flags, Some(rappin::IS_COMPONENTS_V2));
    /// assert!(patch.content.is_none());
    /// ```
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
/// # Example
///
/// ```rust
/// let reaction: rappin::Reaction = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil(),"emoji":"👍","created_at":"2026-11-08T00:00:00Z"})).unwrap();
/// assert_eq!(reaction.emoji, "👍");
/// ```
pub struct Reaction {
    /// Identifier of this reaction entry.
    pub id: Uuid,
    /// User who placed the reaction.
    pub user_id: Uuid,
    /// Reaction emoji as returned by the API.
    pub emoji: String,
    /// UTC time when the reaction was added.
    pub created_at: DateTime<Utc>,
}

/// Request payload for bulk deleting messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let body = rappin::BulkDeleteRequest::new([uuid::Uuid::nil()]);
/// assert_eq!(body.ids.len(), 1);
/// ```
pub struct BulkDeleteRequest {
    /// Message IDs requested for deletion. The API determines which are eligible.
    pub ids: Vec<Uuid>,
}

impl BulkDeleteRequest {
    /// Collect the requested message IDs. This only creates the payload and confirms no deletions.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::BulkDeleteRequest::new([uuid::Uuid::nil()]);
    /// assert_eq!(body.ids.len(), 1);
    /// ```
    pub fn new(ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            ids: ids.into_iter().collect(),
        }
    }
}

/// Response returned from bulk delete operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let response: rappin::BulkDeleteResponse = serde_json::from_str(r#"{"deleted":[],"skipped":[]}"#).unwrap();
/// assert!(response.skipped.is_empty());
/// ```
pub struct BulkDeleteResponse {
    /// Message IDs confirmed deleted by the API.
    pub deleted: Vec<Uuid>,
    /// Requested message IDs the API did not delete.
    pub skipped: Vec<Uuid>,
}

/// Response returned by `GET /channels/{id}/messages`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let page: rappin::MessagesResponse = serde_json::from_str(r#"{"messages":[]}"#).unwrap();
/// assert!(page.messages.is_empty());
/// assert!(page.authors.is_empty());
/// ```
pub struct MessagesResponse {
    /// Messages returned in this page; keep the API order for pagination.
    pub messages: Vec<Message>,
    #[serde(default)]
    /// Expanded public profiles for message authors, if supplied.
    pub authors: Vec<User>,
}

/// Response returned by `GET /channels/{id}/messages/{mid}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let message = rappin::Message::default();
/// let response = rappin::MessageDetailResponse { message, author: None };
/// assert!(response.author.is_none());
/// ```
pub struct MessageDetailResponse {
    /// The requested channel message.
    pub message: Message,
    #[serde(default)]
    /// Expanded author profile when included in the response.
    pub author: Option<User>,
}

impl CreateMessage {
    pub(crate) fn for_request(&self) -> std::borrow::Cow<'_, Self> {
        if self
            .embeds
            .as_ref()
            .is_some_and(|e| e.iter().any(Embed::has_proxy_metadata))
        {
            let mut copy = self.clone();
            for e in copy.embeds.iter_mut().flatten() {
                e.clear_proxy_metadata();
            }
            std::borrow::Cow::Owned(copy)
        } else {
            std::borrow::Cow::Borrowed(self)
        }
    }
}
impl EditMessage {
    pub(crate) fn for_request(&self) -> std::borrow::Cow<'_, Self> {
        if self
            .embeds
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|e| e.iter().any(Embed::has_proxy_metadata))
        {
            let mut copy = self.clone();
            if let Some(Some(embeds)) = &mut copy.embeds {
                for e in embeds {
                    e.clear_proxy_metadata();
                }
            }
            std::borrow::Cow::Owned(copy)
        } else {
            std::borrow::Cow::Borrowed(self)
        }
    }
}
