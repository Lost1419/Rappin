//! Voice channel membership and LiveKit ticket payloads. Playback uses the separate optional voice bridge.
//!
//! # Example
//!
//! ```rust
//! let body = rappin::MoveMemberRequest::new(uuid::Uuid::nil(), uuid::Uuid::nil());
//! assert_eq!(body.channel_id, uuid::Uuid::nil());
//! ```
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Active participants in a voice channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let channel = rappin::VoiceChannel { channel_id: uuid::Uuid::nil(), user_ids: vec![] };
/// assert!(channel.user_ids.is_empty());
/// ```
pub struct VoiceChannel {
    /// Voice channel whose participants were requested.
    pub channel_id: Uuid,
    /// Identifiers of users currently present in the channel.
    pub user_ids: Vec<Uuid>,
}

/// A LiveKit connection ticket for bot voice connection (`POST /servers/{id}/voice/join`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let ticket: rappin::VoiceTicket = serde_json::from_value(serde_json::json!({"url":"wss://example.com","token":"example-not-a-real-ticket","channel_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil(),"e2ee":true})).unwrap();
/// assert!(ticket.e2ee);
/// assert!(!format!("{ticket:?}").contains("example-not-a-real-ticket"));
/// ```
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
/// # Example
///
/// ```rust
/// let body = rappin::MoveMemberRequest::new(uuid::Uuid::nil(), uuid::Uuid::nil());
/// assert_eq!(body.channel_id, uuid::Uuid::nil());
/// ```
pub struct MoveMemberRequest {
    /// Member to move; authorization and hierarchy remain platform-enforced.
    pub user_id: Uuid,
    /// Destination voice channel in the same server.
    pub channel_id: Uuid,
}

impl MoveMemberRequest {
    /// Create a move payload. The destination must be a voice channel within the selected server.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::MoveMemberRequest::new(uuid::Uuid::nil(), uuid::Uuid::nil());
    /// assert_eq!(body.channel_id, uuid::Uuid::nil());
    /// ```
    pub fn new(user_id: Uuid, channel_id: Uuid) -> Self {
        Self {
            user_id,
            channel_id,
        }
    }
}

/// Response returned when moving a member between voice channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let response: rappin::MoveMemberResponse = serde_json::from_value(serde_json::json!({"ok":true,"channel_id":uuid::Uuid::nil()})).unwrap();
/// assert!(response.ok);
/// assert!(response.from.is_none());
/// ```
pub struct MoveMemberResponse {
    /// Whether the API reports the move succeeded.
    pub ok: bool,
    #[serde(default)]
    /// Previous voice channel, when supplied.
    pub from: Option<Uuid>,
    /// Destination channel confirmed by the API.
    pub channel_id: Uuid,
}

/// Request payload to disconnect a member from voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let body = rappin::DisconnectMemberRequest::new(uuid::Uuid::nil());
/// assert_eq!(body.user_id, uuid::Uuid::nil());
/// ```
pub struct DisconnectMemberRequest {
    /// Member to disconnect from voice.
    pub user_id: Uuid,
}

impl DisconnectMemberRequest {
    /// Create a disconnect payload for one member.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::DisconnectMemberRequest::new(uuid::Uuid::nil());
    /// assert_eq!(body.user_id, uuid::Uuid::nil());
    /// ```
    pub fn new(user_id: Uuid) -> Self {
        Self { user_id }
    }
}

/// Response returned when disconnecting a member from voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let response: rappin::DisconnectMemberResponse = serde_json::from_str(r#"{"ok":true}"#).unwrap();
/// assert!(response.ok);
/// assert!(response.channel_id.is_none());
/// ```
pub struct DisconnectMemberResponse {
    /// Whether the API reports the disconnect succeeded.
    pub ok: bool,
    #[serde(default)]
    /// Channel the member left, when returned.
    pub channel_id: Option<Uuid>,
}

impl std::fmt::Debug for VoiceTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceTicket")
            .field("token", &"[REDACTED]")
            .field("channel_id", &self.channel_id)
            .field("server_id", &self.server_id)
            .field("e2ee", &self.e2ee)
            .finish_non_exhaustive()
    }
}
