use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::component::Component;
use crate::models::embed::Embed;

/// A raw event from `GET /events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Incremental event ID.
    pub id: u64,
    /// Event type string (e.g. "message", "member_join", "interaction").
    #[serde(rename = "type")]
    pub kind: String,
    /// Event payload.
    pub data: serde_json::Value,
    /// Event timestamp.
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

impl Event {
    /// Attempts to parse the inner `data` JSON into a strongly typed `EventData`.
    pub fn parse_data(&self) -> Result<EventData, serde_json::Error> {
        match self.kind.as_str() {
            "message" => serde_json::from_value(self.data.clone()).map(EventData::Message),
            "interaction" => serde_json::from_value(self.data.clone()).map(EventData::Interaction),
            "member_join" => serde_json::from_value(self.data.clone()).map(EventData::MemberJoin),
            "member_leave" => serde_json::from_value(self.data.clone()).map(EventData::MemberLeave),
            "reaction_add" => serde_json::from_value(self.data.clone()).map(EventData::ReactionAdd),
            "reaction_remove" => serde_json::from_value(self.data.clone()).map(EventData::ReactionRemove),
            "voice_join" => serde_json::from_value(self.data.clone()).map(EventData::VoiceJoin),
            "voice_leave" => serde_json::from_value(self.data.clone()).map(EventData::VoiceLeave),
            "test" => Ok(EventData::Test(self.data.clone())),
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
pub enum EventData {
    Message(MessageEvent),
    Interaction(InteractionEvent),
    MemberJoin(MemberJoinEvent),
    MemberLeave(MemberLeaveEvent),
    ReactionAdd(ReactionEvent),
    ReactionRemove(ReactionEvent),
    VoiceJoin(VoiceEvent),
    VoiceLeave(VoiceEvent),
    Test(serde_json::Value),
    Unknown {
        kind: String,
        data: serde_json::Value,
    },
}

/// Data payload for `message` events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageEvent {
    pub id: Uuid,
    pub channel_id: Uuid,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

/// Data payload for `member_leave` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberLeaveEvent {
    pub user_id: Uuid,
    pub server_id: Uuid,
    #[serde(default)]
    pub member_count: Option<u64>,
}

/// Data payload for `reaction_add` and `reaction_remove` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionEvent {
    pub channel_id: Uuid,
    pub message_id: Uuid,
    pub user_id: Uuid,
    pub emoji: String,
    #[serde(default)]
    pub server_id: Option<Uuid>,
}

/// Data payload for `voice_join` and `voice_leave` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceEvent {
    pub channel_id: Uuid,
    pub user_id: Uuid,
    #[serde(default)]
    pub server_id: Option<Uuid>,
}

