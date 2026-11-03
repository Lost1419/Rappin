use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::component::Component;
use crate::models::embed::Embed;

/// A raw event from `GET /events`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
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
    pub fn parse_data(&self) -> Result<EventData, serde_json::Error> {
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
pub struct EventsResponse {
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub last: u64,
}

/// Strongly typed event data variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum EventData {
    Message(MessageEvent),
    DmMessage(DmMessageEvent),
    Interaction(InteractionEvent),
    MemberJoin(MemberJoinEvent),
    MemberLeave(MemberLeaveEvent),
    ReactionAdd(ReactionEvent),
    ReactionRemove(ReactionEvent),
    VoiceJoin(VoiceEvent),
    VoiceLeave(VoiceEvent),
    InviteCreate(InviteEvent),
    InviteDelete(InviteEvent),
    Test(serde_json::Value),
    Unknown {
        kind: String,
        data: serde_json::Value,
    },
}

/// Typed DM delivery, with no server command interpretation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct DmMessageEvent(pub MessageEvent);
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
pub struct MessageAttachment {
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub voice_secs: Option<u32>,
}

/// Data payload for `message` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MessageEvent {
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default)]
    pub dm: bool,
    #[serde(default)]
    pub is_dm: bool,
    #[serde(default)]
    pub dm_channel_id: Option<Uuid>,
    pub id: Uuid,
    pub channel_id: Uuid,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub author: Option<serde_json::Value>,
    #[serde(default)]
    pub attachment: Option<serde_json::Value>,
    #[serde(default)]
    pub server_id: Option<Uuid>,
    pub author_id: Uuid,
    #[serde(default)]
    pub author_bot: Option<bool>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub embeds: Option<Vec<Embed>>,
    #[serde(default)]
    pub components: Option<Vec<Component>>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attachment_path: Option<String>,
    #[serde(default)]
    pub reply_to_id: Option<Uuid>,
    #[serde(default)]
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
pub struct InteractionEvent {
    pub id: String,
    pub token: String,
    #[serde(default)]
    pub channel_id: Option<Uuid>,
    #[serde(default)]
    pub server_id: Option<Uuid>,
    #[serde(default)]
    pub user_id: Option<Uuid>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub custom_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<Uuid>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// Server-issued expiration; authoritative when present.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

/// Data payload for `member_join` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberJoinEvent {
    pub user_id: Uuid,
    pub server_id: Uuid,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub joined_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub invite_code: Option<String>,
    #[serde(default)]
    pub inviter_id: Option<Uuid>,
}

/// Invite created/deleted events require Manage Server and test-server access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteEvent {
    pub server_id: Uuid,
    #[serde(flatten)]
    pub invite: super::Invite,
}

/// Data payload for `member_leave` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberLeaveEvent {
    pub user_id: Uuid,
    pub server_id: Uuid,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub joined_at: Option<DateTime<Utc>>,
}

/// Data payload for `reaction_add` and `reaction_remove` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReactionEvent {
    #[serde(default)]
    pub dm: bool,
    #[serde(default)]
    pub is_dm: bool,
    #[serde(default)]
    pub dm_channel_id: Option<Uuid>,
    pub channel_id: Uuid,
    pub message_id: Uuid,
    pub user_id: Uuid,
    pub emoji: String,
    #[serde(default)]
    pub server_id: Option<Uuid>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub message_author_id: Option<Uuid>,
}

/// Data payload for `voice_join` and `voice_leave` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceEvent {
    pub channel_id: Uuid,
    pub user_id: Uuid,
    #[serde(default)]
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
