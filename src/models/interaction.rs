//! Legacy interaction response payloads. Use the extended UI API for selects and modal forms. Creating a payload does not send or acknowledge an interaction.
//!
//! # Example
//!
//! ```rust
//! let callback = rappin::InteractionCallback::ack();
//! assert_eq!(callback.kind, rappin::InteractionCallbackType::Ack);
//! ```
use serde::{Deserialize, Serialize};

use crate::models::component::Component;
use crate::models::embed::Embed;
use crate::utils::IS_COMPONENTS_V2;

/// The type of interaction callback answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// # Example
///
/// ```rust
/// use rappin::InteractionCallbackType;
/// assert_eq!(serde_json::to_value(InteractionCallbackType::Ack).unwrap(), "ack");
/// ```
pub enum InteractionCallbackType {
    /// A new public message in the channel.
    Reply,
    /// An ephemeral message visible only to the interacting user.
    Ephemeral,
    /// Update/edit the message containing the clicked component.
    Update,
    /// Acknowledge the interaction without posting a message ("got it").
    Ack,
}

/// Request payload to answer a button or interaction click.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let callback = rappin::InteractionCallback::ephemeral("Only you can see this");
/// assert_eq!(callback.kind, rappin::InteractionCallbackType::Ephemeral);
/// ```
pub struct InteractionCallback {
    #[serde(rename = "type")]
    /// Callback action: public reply, private reply, update, or acknowledgement.
    pub kind: InteractionCallbackType,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    /// Content patch: omission retains current content; `Some(None)` explicitly clears it.
    pub content: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    /// Embed patch: omission retains embeds; `Some(None)` explicitly clears them.
    pub embeds: Option<Option<Vec<Embed>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    /// Component patch: omission retains controls; `Some(None)` explicitly clears them.
    pub components: Option<Option<Vec<Component>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Message flags for this callback, including 32768 for Components V2 layouts.
    pub flags: Option<u32>,
}

impl InteractionCallback {
    /// Create a public message response payload.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::reply("Hello");
    /// assert_eq!(callback.kind, rappin::InteractionCallbackType::Reply);
    /// ```
    pub fn reply(content: impl Into<String>) -> Self {
        Self {
            kind: InteractionCallbackType::Reply,
            content: Some(Some(content.into())),
            embeds: None,
            components: None,
            flags: None,
        }
    }

    /// Create a response visible only to the interacting user.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::ephemeral("Only you");
    /// assert_eq!(callback.kind, rappin::InteractionCallbackType::Ephemeral);
    /// ```
    pub fn ephemeral(content: impl Into<String>) -> Self {
        Self {
            kind: InteractionCallbackType::Ephemeral,
            content: Some(Some(content.into())),
            embeds: None,
            components: None,
            flags: None,
        }
    }

    /// Create an update to the message containing the clicked component.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated");
    /// assert_eq!(callback.kind, rappin::InteractionCallbackType::Update);
    /// ```
    pub fn update(content: impl Into<String>) -> Self {
        Self {
            kind: InteractionCallbackType::Update,
            content: Some(Some(content.into())),
            embeds: None,
            components: None,
            flags: None,
        }
    }

    /// Acknowledge without posting text. Use context helpers to coordinate response ownership.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::ack();
    /// assert_eq!(callback.content, None);
    /// ```
    pub fn ack() -> Self {
        Self {
            kind: InteractionCallbackType::Ack,
            content: None,
            embeds: None,
            components: None,
            flags: None,
        }
    }

    /// Create a public Components V2 response.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::reply_layout([rappin::TextDisplay::new("Welcome")]);
    /// assert_eq!(callback.flags, Some(rappin::IS_COMPONENTS_V2));
    /// ```
    pub fn reply_layout(components: impl IntoIterator<Item = impl Into<Component>>) -> Self {
        Self {
            kind: InteractionCallbackType::Reply,
            content: None,
            embeds: None,
            components: Some(Some(components.into_iter().map(Into::into).collect())),
            flags: Some(IS_COMPONENTS_V2),
        }
    }

    /// Create a Components V2 update for the interaction's original message.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update_layout([rappin::TextDisplay::new("Updated")]);
    /// assert_eq!(callback.kind, rappin::InteractionCallbackType::Update);
    /// ```
    pub fn update_layout(components: impl IntoIterator<Item = impl Into<Component>>) -> Self {
        Self {
            kind: InteractionCallbackType::Update,
            content: None,
            embeds: None,
            components: Some(Some(components.into_iter().map(Into::into).collect())),
            flags: Some(IS_COMPONENTS_V2),
        }
    }

    /// Append an embed to the callback payload.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").embed(rappin::Embed::new().title("Result"));
    /// assert_eq!(callback.embeds.as_ref().unwrap().as_ref().unwrap().len(), 1);
    /// ```
    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds
            .get_or_insert_with(|| Some(Vec::new()))
            .get_or_insert_with(Vec::new)
            .push(embed);
        self
    }

    /// Append a component to the callback payload.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").component(rappin::ActionRow::new([rappin::Button::primary("Next", "next")]));
    /// assert_eq!(callback.components.as_ref().unwrap().as_ref().unwrap().len(), 1);
    /// ```
    pub fn component(mut self, component: impl Into<Component>) -> Self {
        self.components
            .get_or_insert_with(|| Some(Vec::new()))
            .get_or_insert_with(Vec::new)
            .push(component.into());
        self
    }
}

impl InteractionCallback {
    /// Explicitly clear existing message text.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").clear_content();
    /// assert_eq!(callback.content, Some(None));
    /// ```
    pub fn clear_content(mut self) -> Self {
        self.content = Some(None);
        self
    }
    /// Explicitly clear existing embeds.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").clear_embeds();
    /// assert_eq!(callback.embeds, Some(None));
    /// ```
    pub fn clear_embeds(mut self) -> Self {
        self.embeds = Some(None);
        self
    }
    /// Explicitly clear existing controls.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").clear_components();
    /// assert_eq!(callback.components, Some(None));
    /// ```
    pub fn clear_components(mut self) -> Self {
        self.components = Some(None);
        self
    }
}
