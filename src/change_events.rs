//! Opt-in parsing of message and server notifications from authenticated event data.

use crate::Event;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// Message identity shared by edit and deletion notifications.
///
/// ```
/// let change: rappin::MessageChange = serde_json::from_value(serde_json::json!({
///     "id": uuid::Uuid::nil(), "channel_id": uuid::Uuid::nil(),
///     "server_id": uuid::Uuid::nil(), "author_id": uuid::Uuid::nil()
/// })).unwrap();
/// assert_eq!(change.id, uuid::Uuid::nil());
/// ```
pub struct MessageChange {
    /// Message ID.
    pub id: Uuid,
    /// Channel that contained the message.
    pub channel_id: Uuid,
    /// Owning server.
    pub server_id: Uuid,
    /// Original author.
    pub author_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// An edit to a message visible to the bot.
///
/// ```
/// assert!(serde_json::from_str::<rappin::MessageUpdate>("{}").is_err());
/// ```
pub struct MessageUpdate {
    /// Edited message identity.
    #[serde(flatten)]
    pub message: MessageChange,
    /// Edited content.
    pub content: String,
    /// Server edit time, when available.
    pub edited_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the author is a bot.
    pub author_bot: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// A batch of deleted messages within one channel.
///
/// ```
/// assert!(serde_json::from_str::<rappin::MessageDeleteBulk>("{}").is_err());
/// ```
pub struct MessageDeleteBulk {
    /// Deleted message IDs.
    pub ids: Vec<Uuid>,
    /// Channel that contained the messages.
    pub channel_id: Uuid,
    /// Owning server.
    pub server_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// A server whose bot membership changed.
///
/// ```
/// let change: rappin::BotServerChange = serde_json::from_value(serde_json::json!({
///     "server_id": uuid::Uuid::nil()
/// })).unwrap();
/// assert_eq!(change.server_id, uuid::Uuid::nil());
/// ```
pub struct BotServerChange {
    /// Server ID.
    pub server_id: Uuid,
    /// Server name when reported.
    pub name: Option<String>,
    /// Member count when reported.
    pub member_count: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// Message or bot membership change decoded from an original event.
///
/// ```
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({
///     "id":1,"type":"server_remove","data":{"server_id":uuid::Uuid::nil()}
/// })).unwrap();
/// assert!(matches!(event.parse_change().unwrap(), Some(rappin::ChangeEvent::ServerRemove(_))));
/// ```
pub enum ChangeEvent {
    /// Visible message edited by another author.
    MessageUpdate(#[doc = "Original message edit."] MessageUpdate),
    /// One visible message deleted.
    MessageDelete(#[doc = "Original message identity."] MessageChange),
    /// Multiple messages deleted.
    MessageDeleteBulk(#[doc = "Original deletion batch."] MessageDeleteBulk),
    /// Bot added to a server.
    ServerAdd(#[doc = "Original server membership addition."] BotServerChange),
    /// Bot removed from a server.
    ServerRemove(#[doc = "Original server membership removal."] BotServerChange),
}

impl Event {
    /// Decode a November API change notification while retaining existing dispatch behavior.
    /// Available for raw webhook, polling and gateway events.
    ///
    /// ```
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({
    ///     "id":1,"type":"server_add","data":{"server_id":uuid::Uuid::nil()}
    /// })).unwrap();
    /// assert!(matches!(event.parse_change().unwrap(), Some(rappin::ChangeEvent::ServerAdd(_))));
    /// ```
    pub fn parse_change(&self) -> Result<Option<ChangeEvent>, serde_json::Error> {
        let change = match self.kind.as_str() {
            "message_update" => ChangeEvent::MessageUpdate(MessageUpdate::deserialize(&self.data)?),
            "message_delete" => ChangeEvent::MessageDelete(MessageChange::deserialize(&self.data)?),
            "message_delete_bulk" => {
                ChangeEvent::MessageDeleteBulk(MessageDeleteBulk::deserialize(&self.data)?)
            }
            "server_add" => ChangeEvent::ServerAdd(BotServerChange::deserialize(&self.data)?),
            "server_remove" => ChangeEvent::ServerRemove(BotServerChange::deserialize(&self.data)?),
            _ => return Ok(None),
        };
        Ok(Some(change))
    }
}
