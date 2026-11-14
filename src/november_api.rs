//! Opt-in API 1.4 operations from the November 2026 contract update.

use crate::{
    Api, Channel, CreateChannelRequest, EditChannelRequest, Result, UiMessage, UiMessageBody,
    YappinError,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    pub message: UiMessage,
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

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
/// A message send receipt with optional nonce metadata.
///
/// ```
/// assert!(serde_json::from_str::<rappin::NonceMessage>("{}").is_err());
/// ```
pub struct NonceMessage {
    /// Message returned by the API.
    pub message: UiMessage,
    /// Nonce acknowledged by the API, when reported.
    pub nonce: Option<String>,
    /// Whether the API returned the original receipt for a repeated send.
    #[serde(default)]
    pub duplicate: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
/// A channel edit and the allowlist returned by the server.
///
/// ```
/// assert!(serde_json::from_str::<rappin::ChannelAudience>("{}").is_err());
/// ```
pub struct ChannelAudience {
    /// Edited channel.
    pub channel: Channel,
    /// Returned member allowlist, when supplied.
    pub user_ids: Option<Vec<Uuid>>,
}

/// Additive API methods available with native and worker transports.
///
/// ```no_run
/// use rappin::{Api, NovemberApiExt, Result};
/// async fn inspect(api: &impl Api, channel: uuid::Uuid) -> Result<()> {
///     println!("{:?}", api.list_threads(channel, None).await?);
///     Ok(())
/// }
/// ```
#[async_trait::async_trait]
pub trait NovemberApiExt: Api {
    /// List recent threads in a parent channel. The optional limit is 1–50.
    ///
    /// ```no_run
    /// # use rappin::{Client, NovemberApiExt};
    /// # async fn run(client: &Client, channel: uuid::Uuid) -> rappin::Result<()> {
    /// client.list_threads(channel, Some(25)).await?;
    /// # Ok(()) }
    /// ```
    async fn list_threads(&self, channel: Uuid, limit: Option<u8>) -> Result<Vec<Thread>> {
        let mut path = format!("/channels/{channel}/threads");
        if let Some(limit) = limit {
            if !(1..=50).contains(&limit) {
                return Err(YappinError::validation("limit", "expected 1 to 50"));
            }
            path.push_str(&format!("?limit={limit}"));
        }
        let response = self.request(Method::GET, &path, None).await?;
        let threads: Vec<Thread> = decode(response, "threads")?;
        if threads.iter().any(|thread| thread.parent_id != channel) {
            return Err(YappinError::validation(
                "parent_id",
                "thread belongs to a different channel",
            ));
        }
        Ok(threads)
    }

    /// Create a thread with a first message and optional parent message ID.
    ///
    /// ```no_run
    /// # use rappin::{Client, NovemberApiExt, UiMessageBody};
    /// # async fn run(client: &Client, channel: uuid::Uuid) -> rappin::Result<()> {
    /// client.create_thread(channel, Some("Discussion"), None, &UiMessageBody::new("Welcome")).await?;
    /// # Ok(()) }
    /// ```
    async fn create_thread(
        &self,
        channel: Uuid,
        name: Option<&str>,
        message_id: Option<Uuid>,
        first: &UiMessageBody,
    ) -> Result<CreatedThread> {
        thread_name(name)?;
        if name.is_none() && message_id.is_none() {
            return Err(YappinError::validation(
                "name",
                "provide a name or a parent message",
            ));
        }
        if first.reply_to.is_some() || first.sticker_id.is_some() || first.flags.is_some() {
            return Err(YappinError::validation(
                "message",
                "thread creation accepts content, embeds and button rows",
            ));
        }
        first.validate(true)?;
        let mut body = json!(first);
        if let Some(name) = name {
            body["name"] = json!(name);
        }
        if let Some(message_id) = message_id {
            body["message_id"] = json!(message_id);
        }
        crate::utils::strip_proxy_metadata(&mut body);
        let response = self
            .request(
                Method::POST,
                &format!("/channels/{channel}/threads"),
                Some(body),
            )
            .await?;
        let created: CreatedThread = serde_json::from_value(response)?;
        if created.thread.parent_id != channel || created.message.channel_id != created.thread.id {
            return Err(unexpected_receipt(
                "thread creation returned a different channel",
            ));
        }
        Ok(created)
    }

    /// Rename, lock or archive a thread.
    ///
    /// ```no_run
    /// # use rappin::{Client, NovemberApiExt, ThreadEdit};
    /// # async fn run(client: &Client, thread: uuid::Uuid) -> rappin::Result<()> {
    /// let mut edit = ThreadEdit::default();
    /// edit.locked = Some(true);
    /// client.edit_thread(thread, &edit).await?;
    /// # Ok(()) }
    /// ```
    async fn edit_thread(&self, thread: Uuid, edit: &ThreadEdit) -> Result<()> {
        thread_name(edit.name.as_deref())?;
        self.request(
            Method::PATCH,
            &format!("/threads/{thread}"),
            Some(json!(edit)),
        )
        .await?;
        Ok(())
    }

    /// Delete a thread explicitly.
    ///
    /// ```no_run
    /// # use rappin::{Client, NovemberApiExt};
    /// # async fn run(client: &Client, thread: uuid::Uuid) -> rappin::Result<()> {
    /// client.delete_thread(thread).await?;
    /// # Ok(()) }
    /// ```
    async fn delete_thread(&self, thread: Uuid) -> Result<()> {
        self.request(Method::DELETE, &format!("/threads/{thread}"), None)
            .await?;
        Ok(())
    }

    /// Run one reaction moderation request. The caller decides whether to use `more`.
    ///
    /// ```no_run
    /// # use rappin::{Client, NovemberApiExt, ReactionTarget};
    /// # async fn run(client: &Client, channel: uuid::Uuid, message: uuid::Uuid) -> rappin::Result<()> {
    /// client.moderate_reactions(channel, message, ReactionTarget::Emoji("👍")).await?;
    /// # Ok(()) }
    /// ```
    async fn moderate_reactions(
        &self,
        channel: Uuid,
        message: Uuid,
        target: ReactionTarget<'_>,
    ) -> Result<ReactionCleanup> {
        let path = reaction_path(channel, message, target)?;
        Ok(serde_json::from_value(
            self.request(Method::DELETE, &path, None).await?,
        )?)
    }

    /// Submit one file-free send using a caller-supplied nonce.
    /// The caller owns nonce storage, reuse and reconciliation.
    ///
    /// ```no_run
    /// # use rappin::{Client, NovemberApiExt, UiMessageBody};
    /// # async fn run(client: &Client, channel: uuid::Uuid) -> rappin::Result<()> {
    /// client.send_with_nonce(channel, &UiMessageBody::new("Hello"), "send-0001").await?;
    /// # Ok(()) }
    /// ```
    async fn send_with_nonce(
        &self,
        channel: Uuid,
        message: &UiMessageBody,
        nonce: &str,
    ) -> Result<NonceMessage> {
        validate_nonce(nonce)?;
        message.validate(true)?;
        let mut body = json!(message);
        body["nonce"] = json!(nonce);
        crate::utils::strip_proxy_metadata(&mut body);
        let receipt: NonceMessage = serde_json::from_value(
            self.request_once(
                Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(body),
            )
            .await?,
        )?;
        if receipt.message.channel_id != channel
            || receipt
                .nonce
                .as_deref()
                .is_some_and(|received| received != nonce)
        {
            return Err(unexpected_receipt(
                "send returned a different nonce or channel",
            ));
        }
        Ok(receipt)
    }

    /// Create a channel with a private audience in one request.
    ///
    /// ```no_run
    /// # use rappin::{Client, CreateChannelRequest, NovemberApiExt};
    /// # async fn run(client: &Client, server: uuid::Uuid, member: uuid::Uuid, channel: &CreateChannelRequest) -> rappin::Result<()> {
    /// client.create_private_channel(server, channel, &[], &[member]).await?;
    /// # Ok(()) }
    /// ```
    async fn create_private_channel(
        &self,
        server: Uuid,
        channel: &CreateChannelRequest,
        roles: &[Uuid],
        users: &[Uuid],
    ) -> Result<Channel> {
        validate_user_ids(users)?;
        let mut body = json!(channel);
        body["private"] = json!(true);
        body["role_ids"] = json!(roles);
        body["user_ids"] = json!(users);
        decode(
            self.request(
                Method::POST,
                &format!("/servers/{server}/channels"),
                Some(body),
            )
            .await?,
            "channel",
        )
    }

    /// Edit a channel and replace its user allowlist. An empty list clears it.
    ///
    /// ```no_run
    /// # use rappin::{Client, EditChannelRequest, NovemberApiExt};
    /// # async fn run(client: &Client, channel: uuid::Uuid, edit: &EditChannelRequest) -> rappin::Result<()> {
    /// client.edit_channel_audience(channel, edit, &[]).await?;
    /// # Ok(()) }
    /// ```
    async fn edit_channel_audience(
        &self,
        channel: Uuid,
        edit: &EditChannelRequest,
        users: &[Uuid],
    ) -> Result<ChannelAudience> {
        validate_user_ids(users)?;
        edit.validate()?;
        if edit.private == Some(false) {
            return Err(YappinError::validation(
                "private",
                "an allowlist requires a private channel",
            ));
        }
        let mut body = json!(edit);
        body["user_ids"] = json!(users);
        let result: ChannelAudience = serde_json::from_value(
            self.request(Method::PATCH, &format!("/channels/{channel}"), Some(body))
                .await?,
        )?;
        if result.channel.id != channel {
            return Err(unexpected_receipt(
                "channel edit returned a different channel",
            ));
        }
        Ok(result)
    }
}

impl<T: Api + ?Sized> NovemberApiExt for T {}

fn decode<T: serde::de::DeserializeOwned>(mut response: Value, key: &str) -> Result<T> {
    Ok(serde_json::from_value(response[key].take())?)
}

pub(crate) fn thread_name(name: Option<&str>) -> Result<()> {
    if name.is_some_and(|name| name.trim().is_empty() || name.chars().count() > 100) {
        return Err(YappinError::validation(
            "name",
            "expected 1 to 100 characters",
        ));
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
        return Err(YappinError::validation("emoji", "emoji is required"));
    }
    Ok(url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect())
}

pub(crate) fn validate_user_ids(users: &[Uuid]) -> Result<()> {
    if users.len() > 50 {
        return Err(YappinError::validation("user_ids", "maximum 50 members"));
    }
    let mut unique = std::collections::HashSet::new();
    if users.iter().any(|id| !unique.insert(id)) {
        return Err(YappinError::validation("user_ids", "duplicate member ID"));
    }
    Ok(())
}

pub(crate) fn validate_nonce(nonce: &str) -> Result<()> {
    if !(8..=64).contains(&nonce.len())
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(YappinError::validation(
            "nonce",
            "expected 8 to 64 ASCII letters, digits, hyphens or underscores",
        ));
    }
    Ok(())
}

fn unexpected_receipt(message: &str) -> YappinError {
    YappinError::Remote(crate::RemoteError {
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
