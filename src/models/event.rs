//! Raw delivery envelopes and typed message, membership, reaction, interaction and voice event payloads. Received event IDs are separate from handled checkpoints.
//!
//! # Example
//!
//! ```rust
//! let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":10,"type":"test","data":{"probe":true}})).unwrap();
//! assert_eq!(event.id, 10);
//! assert!(matches!(event.parse_data().unwrap(), rappin::EventData::Test(_)));
//! ```
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::component::Component;
use crate::models::embed::Embed;

/// A raw event from `GET /events`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":10,"type":"test","data":{"probe":true}})).unwrap();
/// assert_eq!(event.id, 10);
/// assert!(matches!(event.parse_data().unwrap(), rappin::EventData::Test(_)));
/// ```
pub struct Event {
    /// Incremental event ID.
    #[serde(default)]
    pub id: u64,
    /// Event type string (e.g. "message", "member_join", "interaction").
    #[serde(rename = "type", alias = "event", default)]
    pub kind: String,
    /// Event payload.
    #[serde(default)]
    pub data: serde_json::Value,
    /// Event timestamp.
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

impl Event {
    /// Attempts to parse the inner `data` JSON into a strongly typed `EventData`.
    ///
    /// If an event kind is unknown or its data schema cannot be deserialized,
    /// most legacy event types fall back to [`EventData::Unknown`]. Typed DM
    /// decoding rejects malformed or inconsistent server scope explicitly.
    /// # Example
    ///
    /// ```rust
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":10,"type":"test","data":{"probe":true}})).unwrap();
    /// assert_eq!(event.id, 10);
    /// assert!(matches!(event.parse_data().unwrap(), rappin::EventData::Test(_)));
    /// let future: rappin::Event = serde_json::from_value(serde_json::json!({"id":11,"type":"future_event","data":{}})).unwrap();
    /// assert!(matches!(future.parse_data().unwrap(), rappin::EventData::Unknown { .. }));
    /// ```
    pub fn parse_data(&self) -> Result<EventData, serde_json::Error> {
        if let Some(resource) = crate::ResourceEvent::from_event(self)
            .map_err(|e| <serde_json::Error as serde::de::Error>::custom(e.to_string()))?
        {
            return Ok(EventData::Resource(resource));
        }
        match self.kind.as_str() {
            "dm_message" => DmMessageEvent::deserialize(&self.data).map(EventData::DmMessage),
            "message" => match serde::Deserialize::deserialize(&self.data) {
                Ok(msg) => Ok(EventData::Message(msg)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'message' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "interaction" => match serde::Deserialize::deserialize(&self.data) {
                Ok(inter) => Ok(EventData::Interaction(inter)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'interaction' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "member_join" => match serde::Deserialize::deserialize(&self.data) {
                Ok(ev) => Ok(EventData::MemberJoin(ev)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'member_join' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "member_leave" => match serde::Deserialize::deserialize(&self.data) {
                Ok(ev) => Ok(EventData::MemberLeave(ev)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'member_leave' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "reaction_add" => match serde::Deserialize::deserialize(&self.data) {
                Ok(ev) => Ok(EventData::ReactionAdd(ev)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'reaction_add' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "reaction_remove" => match serde::Deserialize::deserialize(&self.data) {
                Ok(ev) => Ok(EventData::ReactionRemove(ev)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'reaction_remove' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "voice_join" => match serde::Deserialize::deserialize(&self.data) {
                Ok(ev) => Ok(EventData::VoiceJoin(ev)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'voice_join' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "voice_leave" => match serde::Deserialize::deserialize(&self.data) {
                Ok(ev) => Ok(EventData::VoiceLeave(ev)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse 'voice_leave' event: {}. Falling back to Unknown.",
                        e
                    );
                    Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    })
                }
            },
            "test" => Ok(EventData::Test(self.data.clone())),
            "invite_create" | "invite_delete" => {
                match serde::Deserialize::deserialize(&self.data) {
                    Ok(ev) if self.kind == "invite_create" => Ok(EventData::InviteCreate(ev)),
                    Ok(ev) => Ok(EventData::InviteDelete(ev)),
                    Err(_) => Ok(EventData::Unknown {
                        kind: self.kind.clone(),
                        data: self.data.clone(),
                    }),
                }
            }
            other => Ok(EventData::Unknown {
                kind: other.to_string(),
                data: self.data.clone(),
            }),
        }
    }
}

/// Response returned by `GET /events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let page: rappin::EventsResponse = serde_json::from_str(r#"{"events":[],"last":10}"#).unwrap();
/// assert_eq!(page.last, 10);
/// // Persist a handled checkpoint only after application processing succeeds.
/// ```
pub struct EventsResponse {
    #[serde(default)]
    /// Events returned in this delivery page; handler completion is assessed separately.
    pub events: Vec<Event>,
    #[serde(default)]
    /// Received polling watermark. It is not proof that handlers completed or a durable checkpoint was committed.
    pub last: u64,
}

/// Strongly typed event data variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":10,"type":"test","data":{"probe":true}})).unwrap();
/// assert_eq!(event.id, 10);
/// assert!(matches!(event.parse_data().unwrap(), rappin::EventData::Test(_)));
/// ```
pub enum EventData {
    /// A server/channel message payload.
    Message(#[doc = "Typed original Message event payload."] MessageEvent),
    /// A normalized direct-message payload with no server command interpretation.
    DmMessage(#[doc = "Typed original DmMessage event payload."] DmMessageEvent),
    /// A base interaction payload; use UiInteraction for extended form/select values.
    Interaction(#[doc = "Typed original Interaction event payload."] InteractionEvent),
    /// A server membership addition.
    MemberJoin(#[doc = "Typed original MemberJoin event payload."] MemberJoinEvent),
    /// A server membership removal.
    MemberLeave(#[doc = "Typed original MemberLeave event payload."] MemberLeaveEvent),
    /// A reaction added to a message.
    ReactionAdd(#[doc = "Typed original ReactionAdd event payload."] ReactionEvent),
    /// A reaction removed from a message.
    ReactionRemove(#[doc = "Typed original ReactionRemove event payload."] ReactionEvent),
    /// A user joined a voice channel.
    VoiceJoin(#[doc = "Typed original VoiceJoin event payload."] VoiceEvent),
    /// A user left a voice channel.
    VoiceLeave(#[doc = "Typed original VoiceLeave event payload."] VoiceEvent),
    /// A server invite was created.
    InviteCreate(#[doc = "Typed original InviteCreate event payload."] InviteEvent),
    /// A server invite was deleted.
    InviteDelete(#[doc = "Typed original InviteDelete event payload."] InviteEvent),
    /// A platform test event retaining its raw JSON.
    Test(#[doc = "Typed original Test event payload."] serde_json::Value),
    /// An unrecognized kind or a legacy payload that could not be decoded. Its kind and JSON remain available.
    Unknown {
        /// Original unrecognized event-kind string.
        kind: String,
        /// Original event payload, retained without interpretation.
        data: serde_json::Value,
    },
    /// A validated channel, thread or category lifecycle event.
    Resource(
        #[doc = "Original resource event kind, identity and lossless data."] crate::ResourceEvent,
    ),
}

/// Typed DM delivery, with no server command interpretation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let value: rappin::DmMessageEvent = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"channel_id":uuid::Uuid::nil(),"author_id":uuid::Uuid::nil(),"command":"must-not-dispatch"})).unwrap();
/// assert!(value.is_dm);
/// assert!(value.server_id.is_none());
/// assert!(value.command.is_none());
/// ```
pub struct DmMessageEvent(
    #[doc = "The normalized message payload. DM decoding removes command targeting and rejects server scope."]
    pub MessageEvent,
);
impl<'de> Deserialize<'de> for DmMessageEvent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let mut m = MessageEvent::deserialize(d)?;
        if m.server_id.is_some() {
            return Err(serde::de::Error::custom(
                "DM event cannot have server scope",
            ));
        }
        m.dm = true;
        m.is_dm = true;
        m.dm_channel_id = Some(m.channel_id);
        m.command = None;
        m.args = None;
        m.command_for = None;
        Ok(Self(m))
    }
}
impl std::ops::Deref for DmMessageEvent {
    type Target = MessageEvent;
    fn deref(&self) -> &MessageEvent {
        &self.0
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// Attachment metadata included in an event; dimensions and voice duration are optional.
///
/// # Example
///
/// ```rust
/// let value: rappin::MessageAttachment = serde_json::from_value(serde_json::json!({"path":"report.txt","size":12})).unwrap();
/// assert_eq!(value.size, Some(12));
/// ```
pub struct MessageAttachment {
    /// Stored attachment path.
    pub path: String,
    #[serde(default)]
    /// Original filename, when supplied.
    pub name: Option<String>,
    #[serde(default)]
    /// Attachment size in bytes, when supplied.
    pub size: Option<u64>,
    #[serde(default)]
    /// Image or video width in pixels, when supplied.
    pub width: Option<u32>,
    #[serde(default)]
    /// Image or video height in pixels, when supplied.
    pub height: Option<u32>,
    #[serde(default)]
    /// Voice attachment duration in seconds, when supplied.
    pub voice_secs: Option<u32>,
}

/// Data payload for `message` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let value: rappin::MessageEvent = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"channel_id":uuid::Uuid::nil(),"author_id":uuid::Uuid::nil(),"content":"Hello"})).unwrap();
/// assert_eq!(value.content.as_deref(), Some("Hello"));
/// ```
pub struct MessageEvent {
    #[serde(default)]
    /// Author display name included by the event producer, when supplied.
    pub author_name: Option<String>,
    #[serde(default)]
    /// Expanded attachments; defaults to an empty list.
    pub attachments: Vec<MessageAttachment>,
    #[serde(default)]
    /// Legacy direct-message marker.
    pub dm: bool,
    #[serde(default)]
    /// Direct-message marker used by newer event payloads.
    pub is_dm: bool,
    #[serde(default)]
    /// Direct-message channel identity, when present.
    pub dm_channel_id: Option<Uuid>,
    /// Original message identifier; distinct from the outer event cursor ID.
    pub id: Uuid,
    /// Channel containing the original message.
    pub channel_id: Uuid,
    #[serde(default)]
    /// Channel display name, when supplied.
    pub channel_name: Option<String>,
    #[serde(default)]
    /// Additional author metadata preserved as JSON.
    pub author: Option<serde_json::Value>,
    #[serde(default)]
    /// Legacy single-attachment metadata preserved as JSON.
    pub attachment: Option<serde_json::Value>,
    #[serde(default)]
    /// Server scope; typed DM decoding rejects a non-null server identifier.
    pub server_id: Option<Uuid>,
    /// Original message author; used as requester identity during command dispatch.
    pub author_id: Uuid,
    #[serde(default)]
    /// Whether the producer identifies the author as a bot, when supplied.
    pub author_bot: Option<bool>,
    #[serde(default)]
    /// Original message text, when supplied.
    pub content: Option<String>,
    #[serde(default)]
    /// Rich message cards included in this event.
    pub embeds: Option<Vec<Embed>>,
    #[serde(default)]
    /// Legacy button rows or layouts included in the event. Extended selects use UiInteraction/UiMessage views.
    pub components: Option<Vec<Component>>,
    #[serde(default)]
    /// UTC message creation time, when supplied.
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// UTC last-edit time, when supplied.
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Legacy stored attachment path, when supplied.
    pub attachment_path: Option<String>,
    #[serde(default)]
    /// Message referenced by this reply, when supplied.
    pub reply_to_id: Option<Uuid>,
    #[serde(default)]
    /// UTC pin time, when supplied.
    pub pinned_at: Option<DateTime<Utc>>,
    /// Command name if this message invoked a slash command.
    #[serde(default)]
    pub command: Option<String>,
    /// Command arguments string.
    #[serde(default)]
    pub args: Option<String>,
    /// Targeted bot IDs if command was targeted to specific bots.
    #[serde(default)]
    pub command_for: Option<Vec<Uuid>>,
    /// Whether the bot was directly mentioned in the message.
    #[serde(default)]
    pub mentioned: Option<bool>,
}

/// Data payload for `interaction` (button click) events.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::InteractionEvent = serde_json::from_value(serde_json::json!({"id":"example","token":"unused-example-credential","custom_id":"approve"})).unwrap();
/// assert_eq!(value.custom_id.as_deref(), Some("approve"));
/// assert!(!format!("{value:?}").contains("unused-example-credential"));
/// ```
pub struct InteractionEvent {
    /// Server-issued interaction ID used for callbacks.
    pub id: String,
    /// Sensitive callback credential. Debug redacts it; do not log the serialized event or expose the token.
    pub token: String,
    #[serde(default)]
    /// Channel scope supplied by the platform, if any.
    pub channel_id: Option<Uuid>,
    #[serde(default)]
    /// Server scope supplied by the platform, if any.
    pub server_id: Option<Uuid>,
    #[serde(default)]
    /// User who triggered the interaction, when supplied.
    pub user_id: Option<Uuid>,
    #[serde(default)]
    /// Triggering user's username, when supplied.
    pub username: Option<String>,
    #[serde(default)]
    /// Application callback identifier from the selected control.
    pub custom_id: Option<String>,
    #[serde(default)]
    /// Original message containing the control, if present.
    pub message_id: Option<Uuid>,
    #[serde(default)]
    /// UTC interaction creation time, when supplied.
    pub created_at: Option<DateTime<Utc>>,
    /// Server-issued expiration; authoritative when present.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(rename = "type", default)]
    /// Platform interaction type such as button, select, command or modal_submit.
    pub kind: Option<String>,
}

/// Data payload for `member_join` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::MemberJoinEvent = serde_json::from_value(serde_json::json!({"user_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil(),"member_count":12})).unwrap();
/// assert_eq!(value.member_count, Some(12));
/// ```
pub struct MemberJoinEvent {
    /// Joining user identifier.
    pub user_id: Uuid,
    /// Server the user joined.
    pub server_id: Uuid,
    #[serde(default)]
    /// Joining username, when supplied.
    pub username: Option<String>,
    #[serde(default)]
    /// Membership count reported with the join.
    pub member_count: Option<u64>,
    #[serde(default)]
    /// UTC join time, when supplied.
    pub joined_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Invite attribution visible only when the platform supplies it with the required access.
    pub invite_code: Option<String>,
    #[serde(default)]
    /// Inviting actor, when invite attribution is visible.
    pub inviter_id: Option<Uuid>,
}

/// Invite created/deleted events require Manage Server and test-server access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::InviteEvent = serde_json::from_value(serde_json::json!({"server_id":uuid::Uuid::nil(),"code":"example"})).unwrap();
/// assert_eq!(value.invite.code, "example");
/// ```
pub struct InviteEvent {
    /// Server whose invite changed.
    pub server_id: Uuid,
    #[serde(flatten)]
    /// Flattened invite metadata supplied with this event.
    pub invite: super::Invite,
}

/// Data payload for `member_leave` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::MemberLeaveEvent = serde_json::from_value(serde_json::json!({"user_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil()})).unwrap();
/// assert!(value.joined_at.is_none());
/// ```
pub struct MemberLeaveEvent {
    /// Departing user identifier.
    pub user_id: Uuid,
    /// Server the user left.
    pub server_id: Uuid,
    #[serde(default)]
    /// Membership count reported with the leave.
    pub member_count: Option<u64>,
    #[serde(default)]
    /// Departing username, when supplied.
    pub username: Option<String>,
    #[serde(default)]
    /// Prior membership join time, when supplied; this field is not a leave timestamp.
    pub joined_at: Option<DateTime<Utc>>,
}

/// Data payload for `reaction_add` and `reaction_remove` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let value: rappin::ReactionEvent = serde_json::from_value(serde_json::json!({"channel_id":uuid::Uuid::nil(),"message_id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil(),"emoji":"👍"})).unwrap();
/// assert_eq!(value.emoji, "👍");
/// ```
pub struct ReactionEvent {
    #[serde(default)]
    /// Legacy direct-message marker.
    pub dm: bool,
    #[serde(default)]
    /// Newer direct-message marker.
    pub is_dm: bool,
    #[serde(default)]
    /// Direct-message channel identity, when supplied.
    pub dm_channel_id: Option<Uuid>,
    /// Channel containing the reacted-to message.
    pub channel_id: Uuid,
    /// Message whose reaction changed.
    pub message_id: Uuid,
    /// User who added or removed the reaction.
    pub user_id: Uuid,
    /// Emoji whose reaction changed.
    pub emoji: String,
    #[serde(default)]
    /// Server scope, when supplied; absent for direct-message reactions.
    pub server_id: Option<Uuid>,
    #[serde(default)]
    /// Reacting username, when supplied.
    pub username: Option<String>,
    #[serde(default)]
    /// Author of the reacted-to message, when supplied.
    pub message_author_id: Option<Uuid>,
}

/// Data payload for `voice_join` and `voice_leave` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let value: rappin::VoiceEvent = serde_json::from_value(serde_json::json!({"channel_id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil()})).unwrap();
/// assert!(value.server_id.is_none());
/// ```
pub struct VoiceEvent {
    /// Voice channel the user joined or left.
    pub channel_id: Uuid,
    /// User whose voice membership changed.
    pub user_id: Uuid,
    #[serde(default)]
    /// Containing server, when supplied.
    pub server_id: Option<Uuid>,
}

impl std::fmt::Debug for InteractionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InteractionEvent")
            .field("token", &"[REDACTED]")
            .field("id", &self.id)
            .field("channel_id", &self.channel_id)
            .field("server_id", &self.server_id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Event")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("data", &crate::security::redact_fields(self.data.clone()))
            .finish_non_exhaustive()
    }
}
