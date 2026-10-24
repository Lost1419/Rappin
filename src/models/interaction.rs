use serde::{Deserialize, Serialize};

use crate::models::component::Component;
use crate::models::embed::Embed;
use crate::utils::IS_COMPONENTS_V2;

/// The type of interaction callback answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
pub struct InteractionCallback {
    #[serde(rename = "type")]
    pub kind: InteractionCallbackType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<Embed>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<Component>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}

impl InteractionCallback {
    pub fn reply(content: impl Into<String>) -> Self {
        Self {
            kind: InteractionCallbackType::Reply,
            content: Some(content.into()),
            embeds: None,
            components: None,
            flags: None,
        }
    }

    pub fn ephemeral(content: impl Into<String>) -> Self {
        Self {
            kind: InteractionCallbackType::Ephemeral,
            content: Some(content.into()),
            embeds: None,
            components: None,
            flags: None,
        }
    }

    pub fn update(content: impl Into<String>) -> Self {
        Self {
            kind: InteractionCallbackType::Update,
            content: Some(content.into()),
            embeds: None,
            components: None,
            flags: None,
        }
    }

    pub fn ack() -> Self {
        Self {
            kind: InteractionCallbackType::Ack,
            content: None,
            embeds: None,
            components: None,
            flags: None,
        }
    }

    pub fn reply_layout(components: impl IntoIterator<Item = impl Into<Component>>) -> Self {
        Self {
            kind: InteractionCallbackType::Reply,
            content: None,
            embeds: None,
            components: Some(components.into_iter().map(Into::into).collect()),
            flags: Some(IS_COMPONENTS_V2),
        }
    }

    pub fn update_layout(components: impl IntoIterator<Item = impl Into<Component>>) -> Self {
        Self {
            kind: InteractionCallbackType::Update,
            content: None,
            embeds: None,
            components: Some(components.into_iter().map(Into::into).collect()),
            flags: Some(IS_COMPONENTS_V2),
        }
    }

    pub fn embed(mut self, embed: Embed) -> Self {
        self.embeds.get_or_insert_with(Vec::new).push(embed);
        self
    }

    pub fn component(mut self, component: impl Into<Component>) -> Self {
        self.components.get_or_insert_with(Vec::new).push(component.into());
        self
    }
}

