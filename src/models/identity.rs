//! Opt-in UUID newtypes and tagged response views for API object identity.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! typed_id {
    ($name:ident) => {
        #[doc = concat!("Opt-in ", stringify!($name), " UUID. It carries no permission or provenance grant.\n\n```rust\nlet id = rappin::", stringify!($name), "::from(uuid::Uuid::nil());\nassert_eq!(uuid::Uuid::from(id), uuid::Uuid::nil());\n```")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl From<Uuid> for $name { fn from(id: Uuid) -> Self { Self(id) } }
        impl From<$name> for Uuid { fn from(id: $name) -> Self { id.0 } }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
        }
    };
}
pub use crate::commands::{ChannelId, UserId};
typed_id!(ServerId);
typed_id!(RoleId);
typed_id!(MessageId);
typed_id!(ThreadId);
typed_id!(CategoryId);
typed_id!(EmojiId);
typed_id!(StickerId);
typed_id!(WebhookId);
typed_id!(ScheduledEventId);

/// Typed response with the platform object discriminator retained independently.
///
/// ```rust
/// let tagged = rappin::ApiObject::<rappin::ResolvedObject>::decode(serde_json::json!({
///     "id":uuid::Uuid::nil(),"object":"channel"}), "channel")?;
/// assert_eq!(tagged.object, "channel");
/// # Ok::<(), rappin::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApiObject<T> {
    /// Platform kind such as user, channel, thread, role or message.
    pub object: String,
    /// Typed object payload, including its own retained discriminator if modeled.
    pub data: T,
}
impl<T: serde::de::DeserializeOwned> ApiObject<T> {
    /// Validate the received discriminator and decode the complete object payload.
    ///
    /// ```rust
    /// let tagged = rappin::ApiObject::<rappin::ResolvedObject>::decode(serde_json::json!({
    ///     "id":uuid::Uuid::nil(),"object":"user"}), "user")?;
    /// assert_eq!(tagged.data.object, "user");
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn decode(value: serde_json::Value, expected: &str) -> crate::Result<Self> {
        let object = value["object"]
            .as_str()
            .filter(|kind| *kind == expected)
            .ok_or_else(|| {
                crate::Error::validation(
                    "object",
                    "unexpected or missing platform object discriminator",
                )
            })?
            .to_owned();
        Ok(Self {
            object,
            data: serde_json::from_value(value)?,
        })
    }
}
