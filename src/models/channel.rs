//! Channel metadata, create/edit payloads and explicit nullable category removal.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::server::Category;

/// The type of channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// # Example
///
/// ```rust
/// use rappin::ChannelKind;
/// let kind: ChannelKind = serde_json::from_str("\"voice\"").unwrap();
/// assert_eq!(kind, ChannelKind::Voice);
/// ```
pub enum ChannelKind {
    /// A text channel for messages.
    Text,
    /// A voice channel for call participants.
    Voice,
}

// Yapin can return both keys on the same object. An alias alone treats them as a
// duplicate field, so decode the flattened pair and validate that they agree.
fn deserialize_channel_kind<'de, D>(deserializer: D) -> Result<ChannelKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default, deserialize_with = "present_channel_kind")]
        kind: Option<ChannelKind>,
        #[serde(rename = "type", default, deserialize_with = "present_channel_kind")]
        channel_type: Option<ChannelKind>,
    }

    let fields = Fields::deserialize(deserializer)?;
    match (fields.kind, fields.channel_type) {
        (Some(kind), Some(channel_type)) if kind != channel_type => {
            Err(serde::de::Error::custom("channel kind and type must agree"))
        }
        (Some(kind), _) | (_, Some(kind)) => Ok(kind),
        (None, None) => Err(serde::de::Error::missing_field("kind")),
    }
}

// Omitted keys are optional; a present key must still be a valid, non-null kind.
fn present_channel_kind<'de, D>(deserializer: D) -> Result<Option<ChannelKind>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    ChannelKind::deserialize(deserializer).map(Some)
}

fn serialize_channel_kind<S>(kind: &ChannelKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry("kind", kind)?;
    map.end()
}

/// A channel in a server or category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{Channel, ChannelKind};
/// let channel: Channel = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"general","type":"text"})).unwrap();
/// assert_eq!(channel.kind, ChannelKind::Text);
/// ```
pub struct Channel {
    /// Channel ID.
    pub id: Uuid,
    /// Channel name.
    pub name: String,
    /// Channel type ("text" or "voice"). Accepts `kind`, `type`, or both when equal.
    #[serde(
        flatten,
        deserialize_with = "deserialize_channel_kind",
        serialize_with = "serialize_channel_kind"
    )]
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
/// # Example
///
/// ```rust
/// use rappin::{ChannelRow, ChannelKind};
/// let channel: ChannelRow = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"general","type":"text"})).unwrap();
/// assert_eq!(channel.kind, ChannelKind::Text);
/// ```
pub struct ChannelRow {
    /// Unique channel identifier.
    pub id: Uuid,
    /// Accepts `kind`, `type`, or both when equal; serializes as `kind`.
    #[serde(
        flatten,
        deserialize_with = "deserialize_channel_kind",
        serialize_with = "serialize_channel_kind"
    )]
    pub kind: ChannelKind,
    #[serde(default)]
    /// Channel display name, when returned by this endpoint.
    pub name: Option<String>,
    #[serde(default)]
    /// Identifier of the server containing this channel, when returned.
    pub server_id: Option<Uuid>,
    #[serde(default)]
    /// Parent category identifier, or no category when absent.
    pub category_id: Option<Uuid>,
    #[serde(default)]
    /// Platform-supplied position in the channel listing; it is not a role hierarchy position.
    pub position: Option<i32>,
    #[serde(default)]
    /// Whether access is restricted to permitted roles, when provided by the API.
    pub private: Option<bool>,
    #[serde(default)]
    /// Whether posting is restricted by the channel settings.
    pub read_only: Option<bool>,
    #[serde(default)]
    /// Minimum posting interval in seconds; zero disables slowmode.
    pub slowmode: Option<u32>,
}

/// Payload for creating a channel (`POST /servers/{id}/channels`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{CreateChannelRequest, ChannelKind};
/// let request = CreateChannelRequest::voice("Lobby").in_category(uuid::Uuid::nil());
/// assert_eq!(request.category_id, Some(uuid::Uuid::nil()));
/// ```
pub struct CreateChannelRequest {
    /// Channel display name, when returned by this endpoint.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Requested channel kind; omission delegates the default to the API.
    pub kind: Option<ChannelKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Parent category identifier, or no category when absent.
    pub category_id: Option<Uuid>,
}

impl CreateChannelRequest {
    /// Create a request with only a name; the API selects the channel kind.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{CreateChannelRequest, ChannelKind};
    /// let request = CreateChannelRequest::new("lobby");
    /// assert!(request.kind.is_none());
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: None,
            category_id: None,
        }
    }

    /// Create a text-channel request.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{CreateChannelRequest, ChannelKind};
    /// let request = CreateChannelRequest::text("lobby");
    /// assert_eq!(request.kind, Some(ChannelKind::Text));
    /// ```
    pub fn text(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: Some(ChannelKind::Text),
            category_id: None,
        }
    }

    /// Create a voice-channel request.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{CreateChannelRequest, ChannelKind};
    /// let request = CreateChannelRequest::voice("lobby");
    /// assert_eq!(request.kind, Some(ChannelKind::Voice));
    /// ```
    pub fn voice(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: Some(ChannelKind::Voice),
            category_id: None,
        }
    }

    /// Place the new channel in the specified category; callers must choose a category in the same server.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{CreateChannelRequest, ChannelKind};
    /// let request = CreateChannelRequest::text("announcements").in_category(uuid::Uuid::nil());
    /// assert_eq!(request.category_id, Some(uuid::Uuid::nil()));
    /// ```
    pub fn in_category(mut self, category_id: Uuid) -> Self {
        self.category_id = Some(category_id);
        self
    }
}

/// Payload for modifying an existing channel (`PATCH /channels/{id}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::EditChannelRequest;
/// let request = EditChannelRequest::new().category_id(None).slowmode(5);
/// assert_eq!(request.category_id, Some(None));
/// request.validate().unwrap();
/// ```
pub struct EditChannelRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Channel display name, when returned by this endpoint.
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    /// Category patch: `None` leaves it unchanged, `Some(None)` removes the category, and `Some(Some(id))` moves it.
    pub category_id: Option<Option<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Whether access is restricted to permitted roles, when provided by the API.
    pub private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Role IDs selected by the private-channel access setting.
    pub role_ids: Option<Vec<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Whether posting is restricted by the channel settings.
    pub read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Role IDs selected by the channel posting restriction.
    pub post_role_ids: Option<Vec<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Minimum posting interval in seconds; zero disables slowmode.
    pub slowmode: Option<u32>,
}

impl EditChannelRequest {
    /// Validate documented slowmode bounds before making an API request.
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// assert!(EditChannelRequest::new().slowmode(21_600).validate().is_ok());
    /// assert!(EditChannelRequest::new().slowmode(21_601).validate().is_err());
    /// ```
    pub fn validate(&self) -> crate::Result<()> {
        if self.slowmode.is_some_and(|seconds| seconds > 21600) {
            return Err(crate::YappinError::validation(
                "slowmode",
                "must be between 0 and 21600 seconds",
            ));
        }
        Ok(())
    }

    /// Create an empty patch; omitted fields retain their existing platform values.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// let patch = EditChannelRequest::new();
    /// assert_eq!(serde_json::to_value(patch).unwrap(), serde_json::json!({}));
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the replacement channel name.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// let patch = EditChannelRequest::new().name("announcements");
    /// assert_eq!(patch.name.as_deref(), Some("announcements"));
    /// ```
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set or remove the parent category. Passing `None` explicitly serializes JSON null; omitting this setter keeps the category unchanged.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// let patch = EditChannelRequest::new().category_id(None);
    /// assert_eq!(patch.category_id, Some(None));
    /// ```
    pub fn category_id(mut self, cat_id: Option<Uuid>) -> Self {
        self.category_id = Some(cat_id);
        self
    }

    /// Set the private-channel flag. This does not independently establish any actor's effective access.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// let patch = EditChannelRequest::new().private(true);
    /// assert_eq!(patch.private, Some(true));
    /// ```
    pub fn private(mut self, private: bool) -> Self {
        self.private = Some(private);
        self
    }

    /// Set the posting restriction flag.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// let patch = EditChannelRequest::new().read_only(true);
    /// assert_eq!(patch.read_only, Some(true));
    /// ```
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = Some(read_only);
        self
    }

    /// Set the posting interval in seconds. Validation accepts zero through 21,600.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EditChannelRequest;
    /// let patch = EditChannelRequest::new().slowmode(5);
    /// assert_eq!(patch.slowmode, Some(5));
    /// ```
    pub fn slowmode(mut self, slowmode_seconds: u32) -> Self {
        self.slowmode = Some(slowmode_seconds);
        self
    }
}

/// Response returned by `GET /servers/{id}/channels`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let response: rappin::ServerChannelsResponse = serde_json::from_str(r#"{"channels":[]}"#).unwrap();
/// assert!(response.channels.is_empty());
/// assert!(response.categories.is_empty());
/// ```
pub struct ServerChannelsResponse {
    /// Channels returned for the requested server.
    pub channels: Vec<Channel>,
    #[serde(default)]
    /// Category inventory included in the same response; defaults to an empty list if omitted.
    pub categories: Vec<Category>,
}
