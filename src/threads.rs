//! Thread payloads and route validation for message and reaction operations.

use crate::{Error, Message, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
/// A thread returned by list or create.
///
/// ```
/// assert!(serde_json::from_str::<rappin::Thread>("{}").is_err());
/// ```
pub struct Thread {
    /// Thread channel ID.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Parent text channel.
    pub parent_id: Uuid,
    /// Owning server.
    pub server_id: Uuid,
    /// Author, when reported.
    pub created_by: Option<Uuid>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Most recent message timestamp, when reported.
    pub last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Archive status, when reported.
    pub archived: Option<bool>,
    /// Lock status.
    pub locked: bool,
    /// Parent message from which this thread was started.
    pub starter_message_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
/// A newly created thread and its confirmed first message.
///
/// ```
/// assert!(serde_json::from_str::<rappin::CreatedThread>("{}").is_err());
/// ```
pub struct CreatedThread {
    /// Created thread.
    pub thread: Thread,
    /// First message sent in the thread.
    pub message: Message,
}

#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
/// Thread changes. Omitted fields retain their existing values.
///
/// ```
/// let mut edit = rappin::ThreadEdit::default();
/// edit.archived = Some(true);
/// assert_eq!(serde_json::to_value(edit).unwrap()["archived"], true);
/// ```
pub struct ThreadEdit {
    /// New thread name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New lock state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// New archive state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
/// The outcome of one reaction cleanup request.
///
/// ```
/// let result: rappin::ReactionCleanup = serde_json::from_str(r#"{"removed":12,"more":false}"#).unwrap();
/// assert_eq!(result.removed, 12);
/// ```
pub struct ReactionCleanup {
    /// Number removed by this request.
    pub removed: u32,
    /// The server reports that more matching reactions remain.
    pub more: bool,
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
/// Explicit reaction moderation scope.
///
/// ```
/// let scope = rappin::ReactionTarget::Emoji("👍");
/// assert!(matches!(scope, rappin::ReactionTarget::Emoji(_)));
/// ```
pub enum ReactionTarget<'a> {
    /// Clear reactions to the message.
    Message,
    /// Clear all instances of one emoji.
    Emoji(#[doc = "Emoji to clear."] &'a str),
    /// Clear one user's reaction with the specified emoji.
    User {
        /// Emoji to clear.
        emoji: &'a str,
        /// User whose reaction is targeted.
        user: Uuid,
    },
}

pub(crate) fn thread_name(name: Option<&str>) -> Result<()> {
    if name.is_some_and(|name| name.trim().is_empty() || name.chars().count() > 100) {
        return Err(Error::validation("name", "expected 1 to 100 characters"));
    }
    Ok(())
}

pub(crate) fn reaction_path(
    channel: Uuid,
    message: Uuid,
    target: ReactionTarget<'_>,
) -> Result<String> {
    let mut path = format!("/channels/{channel}/messages/{message}/reactions");
    match target {
        ReactionTarget::Message => {}
        ReactionTarget::Emoji(emoji) => {
            path.push_str(&format!("/{}?all=true", encode_emoji(emoji)?));
        }
        ReactionTarget::User { emoji, user } => {
            path.push_str(&format!("/{}?user_id={user}", encode_emoji(emoji)?));
        }
    }
    Ok(path)
}

fn encode_emoji(emoji: &str) -> Result<String> {
    if emoji.is_empty() {
        return Err(Error::validation("emoji", "emoji is required"));
    }
    Ok(url::form_urlencoded::byte_serialize(emoji.as_bytes())
        .collect::<String>()
        .replace('+', "%20"))
}

pub(crate) fn validate_user_ids(users: &[Uuid]) -> Result<()> {
    if users.len() > 50 {
        return Err(Error::validation("user_ids", "maximum 50 members"));
    }
    let mut unique = std::collections::HashSet::new();
    if users.iter().any(|id| !unique.insert(id)) {
        return Err(Error::validation("user_ids", "duplicate member ID"));
    }
    Ok(())
}

pub(crate) fn validate_nonce(nonce: &str) -> Result<()> {
    if !(8..=64).contains(&nonce.len())
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(Error::validation(
            "nonce",
            "expected 8 to 64 ASCII letters, digits, hyphens or underscores",
        ));
    }
    Ok(())
}

pub(crate) fn unexpected_receipt(message: &str) -> Error {
    Error::Remote(crate::RemoteError {
        kind: "operation_result_scope".into(),
        message: message.into(),
        status: None,
        retry_after: None,
        details: [
            ("outcome".into(), json!("unknown")),
            ("retry_safety".into(), json!("do_not_replay_workflow")),
        ]
        .into(),
    })
}
