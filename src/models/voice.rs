use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Active participants in a voice channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceChannel {
    pub channel_id: Uuid,
    pub user_ids: Vec<Uuid>,
}

/// A LiveKit connection ticket for bot voice connection (`POST /servers/{id}/voice/join`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceTicket {
    /// LiveKit server WebSocket URL.
    pub url: String,
    /// LiveKit access token.
    pub token: String,
    /// Whether the voice channel has end-to-end encryption enabled.
    #[serde(default)]
    pub e2ee: bool,
    /// Whether the bot's audio is muted on join.
    #[serde(default)]
    pub voice_muted: bool,
    /// Channel ID joined.
    pub channel_id: Uuid,
    /// Server ID.
    pub server_id: Uuid,
    /// Seconds the ticket remains valid to establish connection.
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// Request payload to move a member to another voice channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveMemberRequest {
    pub user_id: Uuid,
    pub channel_id: Uuid,
}

impl MoveMemberRequest {
    pub fn new(user_id: Uuid, channel_id: Uuid) -> Self {
        Self {
            user_id,
            channel_id,
        }
    }
}

/// Response returned when moving a member between voice channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveMemberResponse {
    pub ok: bool,
    #[serde(default)]
    pub from: Option<Uuid>,
    pub channel_id: Uuid,
}

/// Request payload to disconnect a member from voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectMemberRequest {
    pub user_id: Uuid,
}

impl DisconnectMemberRequest {
    pub fn new(user_id: Uuid) -> Self {
        Self { user_id }
    }
}

/// Response returned when disconnecting a member from voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectMemberResponse {
    pub ok: bool,
    #[serde(default)]
    pub channel_id: Option<Uuid>,
}
