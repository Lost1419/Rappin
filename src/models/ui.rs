//! Additive API 1.3 payloads; legacy Component/Button/InteractionCallback remain unchanged.
use super::*;
use crate::{Result, YappinError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Emoji text, or the platform's structured custom-emoji reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum EmojiRef {
    Text(String),
    Custom {
        id: Uuid,
        name: String,
        #[serde(default)]
        animated: bool,
    },
}
impl EmojiRef {
    pub fn custom(id: Uuid, name: impl Into<String>, animated: bool) -> Self {
        Self::Custom {
            id,
            name: name.into(),
            animated,
        }
    }
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Custom { id, name, animated } => {
                format!("<{}:{name}:{id}>", if *animated { "a" } else { "" })
            }
        }
    }
}
impl From<String> for EmojiRef {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for EmojiRef {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<EmojiRef>,
    #[serde(default)]
    pub default: bool,
}
impl SelectOption {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            description: None,
            emoji: None,
            default: false,
        }
    }
    pub fn emoji(mut self, emoji: impl Into<EmojiRef>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectMenu {
    #[serde(rename = "type")]
    pub kind: String,
    pub custom_id: String,
    pub options: Vec<SelectOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub min_values: u8,
    pub max_values: u8,
    pub disabled: bool,
}
impl SelectMenu {
    pub fn new(custom_id: impl Into<String>) -> Self {
        Self {
            kind: "select".into(),
            custom_id: custom_id.into(),
            options: vec![],
            placeholder: None,
            min_values: 1,
            max_values: 1,
            disabled: false,
        }
    }
    pub fn option(mut self, option: SelectOption) -> Self {
        self.options.push(option);
        self
    }
    pub fn row(self) -> UiComponent {
        UiComponent::row([UiComponent(json!(self))])
    }
}
/// A lossless component tree supporting classic and V2 buttons/select rows.
/// Deserialization preserves unknown fields; outbound requests validate the tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UiComponent(Value);
impl UiComponent {
    pub fn row(children: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self(
            json!({"type":"row","components":children.into_iter().map(Into::into).collect::<Vec<Self>>()}),
        )
    }
    pub fn container(children: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self(
            json!({"type":17,"components":children.into_iter().map(Into::into).collect::<Vec<Self>>()}),
        )
    }
    pub fn button(button: Button, emoji: impl Into<EmojiRef>) -> Self {
        let mut value = json!(button);
        value["emoji"] = json!(emoji.into());
        Self(value)
    }
    pub fn as_json(&self) -> &Value {
        &self.0
    }
}
impl From<Component> for UiComponent {
    fn from(value: Component) -> Self {
        Self(json!(value))
    }
}
impl From<Button> for UiComponent {
    fn from(value: Button) -> Self {
        Self(json!(value))
    }
}
impl From<SelectMenu> for UiComponent {
    fn from(value: SelectMenu) -> Self {
        Self(json!(value))
    }
}

/// Create/edit body for new UI trees, with explicit patch nulls. Old payload types
/// keep their existing fields and exhaustive enums.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UiMessageBody {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    pub content: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    pub embeds: Option<Option<Vec<Embed>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::utils::deserialize_patch"
    )]
    pub components: Option<Option<Vec<UiComponent>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Uuid>,
}
impl UiMessageBody {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: Some(Some(content.into())),
            ..Self::default()
        }
    }
    pub fn layout(components: impl IntoIterator<Item = impl Into<UiComponent>>) -> Self {
        Self {
            components: Some(Some(components.into_iter().map(Into::into).collect())),
            flags: Some(crate::utils::IS_COMPONENTS_V2),
            ..Self::default()
        }
    }
    pub fn component(mut self, component: impl Into<UiComponent>) -> Self {
        self.components
            .get_or_insert_with(|| Some(vec![]))
            .get_or_insert_with(Vec::new)
            .push(component.into());
        self
    }
    pub fn validate(&self, creating: bool) -> Result<()> {
        if !creating && self.reply_to.is_some() {
            return Err(YappinError::validation(
                "reply_to",
                "reply target cannot be changed by an edit",
            ));
        }
        crate::validation::validate_message(&json!(self), creating)
    }
}
/// Lossless response view for messages containing components not representable by
/// the legacy Component enum. Extra message fields remain available in `extra`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UiMessage {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub components: Option<Vec<UiComponent>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum TextInputStyle {
    Short,
    Paragraph,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TextInput {
    #[serde(rename = "type")]
    pub kind: String,
    pub custom_id: String,
    pub label: String,
    pub style: TextInputStyle,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
}
impl TextInput {
    pub fn short(custom_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            kind: "text_input".into(),
            custom_id: custom_id.into(),
            label: label.into(),
            style: TextInputStyle::Short,
            required: true,
            description: None,
            placeholder: None,
            value: None,
            min_length: None,
            max_length: None,
        }
    }
    pub fn paragraph(custom_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            style: TextInputStyle::Paragraph,
            ..Self::short(custom_id, label)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModalSelect {
    #[serde(flatten)]
    pub select: SelectMenu,
    pub label: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
impl ModalSelect {
    pub fn new(label: impl Into<String>, select: SelectMenu) -> Self {
        Self {
            select,
            label: label.into(),
            required: true,
            description: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum ModalInput {
    Text(TextInput),
    Select(ModalSelect),
}
impl From<TextInput> for ModalInput {
    fn from(value: TextInput) -> Self {
        Self::Text(value)
    }
}
impl From<ModalSelect> for ModalInput {
    fn from(value: ModalSelect) -> Self {
        Self::Select(value)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Modal {
    pub custom_id: String,
    pub title: String,
    pub components: Vec<ModalInput>,
}
impl Modal {
    pub fn new(custom_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            custom_id: custom_id.into(),
            title: title.into(),
            components: vec![],
        }
    }
    pub fn input(mut self, input: impl Into<ModalInput>) -> Self {
        self.components.push(input.into());
        self
    }
    pub fn validate(&self) -> Result<()> {
        crate::validation::validate_modal(&json!(self))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum ModalValue {
    Text(String),
    Selected(Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum InteractionValues {
    Selected(Vec<String>),
    Submitted(BTreeMap<String, ModalValue>),
}
/// Extended event view. Credential Debug output delegates to the redacted base.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UiInteraction {
    #[serde(flatten)]
    pub interaction: InteractionEvent,
    #[serde(default)]
    pub values: Option<InteractionValues>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<String>,
}
impl UiInteraction {
    pub fn from_event(event: &crate::Event) -> Result<Self> {
        if !matches!(event.kind.as_str(), "interaction" | "command") {
            return Err(YappinError::validation(
                "event",
                "requires interaction or command",
            ));
        }
        let mut value: Self = serde_json::from_value(event.data.clone())?;
        if event.kind == "command" {
            value.interaction.kind = Some("command".into());
        }
        value.interaction.created_at = value.interaction.created_at.or(event.created_at);
        match value.interaction.kind.as_deref() {
            Some("select") if !matches!(value.values, Some(InteractionValues::Selected(_))) => Err(
                YappinError::validation("values", "select requires selected values"),
            ),
            Some("modal_submit")
                if !matches!(value.values, Some(InteractionValues::Submitted(_))) =>
            {
                Err(YappinError::validation(
                    "values",
                    "modal_submit requires submitted fields",
                ))
            }
            _ => Ok(value),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FoundEmoji {
    pub id: Uuid,
    pub name: String,
    pub animated: bool,
    pub url: String,
    pub in_server: bool,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MessageEmojis {
    pub emojis: Vec<FoundEmoji>,
}

impl crate::collectors::Collectible for UiInteraction {
    fn from_event(event: &crate::Event) -> Option<Self> {
        Self::from_event(event).ok()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UiMessagesResponse {
    pub messages: Vec<UiMessage>,
    #[serde(default)]
    pub authors: Vec<User>,
}

impl Button {
    /// Preserve the legacy string field while accepting a typed emoji reference.
    pub fn with_emoji_ref(self, emoji: impl Into<EmojiRef>) -> Self {
        self.with_emoji(emoji.into().text())
    }
}
