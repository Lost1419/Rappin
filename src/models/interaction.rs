//! Interaction responses built from the shared message patch model. Creating a payload does not send or acknowledge an interaction.
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
    /// Shared content, embed, component and flag patch. Its fields serialize at
    /// the callback's top level, retaining omission and explicit null clearing.
    #[serde(flatten)]
    pub message: crate::EditMessage,
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
            message: crate::EditMessage::new().content(content),
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
            message: crate::EditMessage::new().content(content),
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
            message: crate::EditMessage::new().content(content),
        }
    }

    /// Acknowledge without posting text. Use context helpers to coordinate response ownership.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::ack();
    /// assert_eq!(callback.message.content, None);
    /// ```
    pub fn ack() -> Self {
        Self {
            kind: InteractionCallbackType::Ack,
            message: crate::EditMessage::new(),
        }
    }

    /// Create a public Components V2 response.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::reply_layout([rappin::TextDisplay::new("Welcome")]);
    /// assert_eq!(callback.message.flags, Some(rappin::IS_COMPONENTS_V2));
    /// ```
    pub fn reply_layout(components: impl IntoIterator<Item = impl Into<Component>>) -> Self {
        Self {
            kind: InteractionCallbackType::Reply,
            message: crate::EditMessage::layout(components),
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
            message: crate::EditMessage::layout(components),
        }
    }

    /// Append an embed to the callback payload.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").embed(rappin::Embed::new().title("Result"));
    /// assert_eq!(callback.message.embeds.as_ref().unwrap().as_ref().unwrap().len(), 1);
    /// ```
    pub fn embed(mut self, embed: Embed) -> Self {
        self.message
            .embeds
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
    /// assert_eq!(callback.message.components.as_ref().unwrap().as_ref().unwrap().len(), 1);
    /// ```
    pub fn component(mut self, component: impl Into<Component>) -> Self {
        self.message
            .components
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
    /// assert_eq!(callback.message.content, Some(None));
    /// ```
    pub fn clear_content(mut self) -> Self {
        self.message.content = Some(None);
        self
    }
    /// Explicitly clear existing embeds.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").clear_embeds();
    /// assert_eq!(callback.message.embeds, Some(None));
    /// ```
    pub fn clear_embeds(mut self) -> Self {
        self.message.embeds = Some(None);
        self
    }
    /// Explicitly clear existing controls.
    ///
    /// # Example
    ///
    /// ```rust
    /// let callback = rappin::InteractionCallback::update("Updated").clear_components();
    /// assert_eq!(callback.message.components, Some(None));
    /// ```
    pub fn clear_components(mut self) -> Self {
        self.message.components = Some(None);
        self
    }
}
