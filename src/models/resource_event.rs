//! Channel, thread and category lifecycle events with lossless platform data.
use crate::{Event, Result, YappinError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// API 1.4 resource lifecycle notification kind.
///
/// ```rust
/// assert_eq!(rappin::ResourceEventKind::ChannelCreate.as_str(), "channel_create");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceEventKind {
    /// A text or voice channel was created or restored.
    ChannelCreate,
    /// A text or voice channel was updated.
    ChannelUpdate,
    /// A channel was deleted or moved to trash.
    ChannelDelete,
    /// A thread was created.
    ThreadCreate,
    /// A thread was updated.
    ThreadUpdate,
    /// A thread was deleted.
    ThreadDelete,
    /// A category was created.
    CategoryCreate,
    /// A category was updated.
    CategoryUpdate,
    /// A category was deleted.
    CategoryDelete,
}
impl ResourceEventKind {
    /// Original transport event name, separate from the resource's own type.
    ///
    /// ```rust
    /// assert_eq!(rappin::ResourceEventKind::ThreadUpdate.as_str(), "thread_update");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChannelCreate => "channel_create",
            Self::ChannelUpdate => "channel_update",
            Self::ChannelDelete => "channel_delete",
            Self::ThreadCreate => "thread_create",
            Self::ThreadUpdate => "thread_update",
            Self::ThreadDelete => "thread_delete",
            Self::CategoryCreate => "category_create",
            Self::CategoryUpdate => "category_update",
            Self::CategoryDelete => "category_delete",
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        [
            Self::ChannelCreate,
            Self::ChannelUpdate,
            Self::ChannelDelete,
            Self::ThreadCreate,
            Self::ThreadUpdate,
            Self::ThreadDelete,
            Self::CategoryCreate,
            Self::CategoryUpdate,
            Self::CategoryDelete,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == value)
    }
    pub(crate) fn object(self) -> &'static str {
        match self {
            Self::ChannelCreate | Self::ChannelUpdate | Self::ChannelDelete => "channel",
            Self::ThreadCreate | Self::ThreadUpdate | Self::ThreadDelete => "thread",
            Self::CategoryCreate | Self::CategoryUpdate | Self::CategoryDelete => "category",
        }
    }
}

/// Validated resource identity and unchanged event data, including future fields.
///
/// ```rust
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":42,
///     "type":"channel_update","data":{"id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil(),
///     "object":"channel","type":"voice"}}))?;
/// let resource = rappin::ResourceEvent::from_event(&event)?.unwrap();
/// assert_eq!(resource.data["type"], "voice");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResourceEvent {
    /// Original lifecycle event kind.
    pub kind: ResourceEventKind,
    /// Resource ID from the authenticated event payload.
    pub id: Uuid,
    /// Containing server from the event payload.
    pub server_id: Uuid,
    /// Complete original data, retaining channel type and additional fields.
    pub data: Value,
}
impl ResourceEvent {
    /// Decode a known lifecycle event and reject inconsistent resource identity.
    /// Unrelated event names return None.
    ///
    /// ```rust
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"type":"test","data":{}}))?;
    /// assert!(rappin::ResourceEvent::from_event(&event)?.is_none());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn from_event(event: &Event) -> Result<Option<Self>> {
        let Some(kind) = ResourceEventKind::parse(&event.kind) else {
            return Ok(None);
        };
        let id = event.data["id"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok());
        let server = event.data["server_id"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok());
        let (Some(id), Some(server_id)) = (id, server) else {
            return Err(YappinError::validation(
                "event",
                "resource event requires UUID resource and server identities",
            ));
        };
        if event.data["object"] != kind.object()
            || event.data.get("channel_id").is_some_and(|channel| {
                if kind.object() == "category" {
                    !channel.is_null()
                } else {
                    channel.as_str().and_then(|s| s.parse::<Uuid>().ok()) != Some(id)
                }
            })
        {
            return Err(YappinError::validation(
                "event",
                "resource event has inconsistent object or channel identity",
            ));
        }
        Ok(Some(Self {
            kind,
            id,
            server_id,
            data: event.data.clone(),
        }))
    }
}
