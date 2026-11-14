//! # Example
//!
//! ```rust
//! use rappin::{SelectMenu, SelectOption};
//! let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
//! let body = rappin::CreateMessage::new("Choose fruit").component(menu.row());
//! body.validate().unwrap();
//! ```
//! Additive API 1.3 payloads; legacy Component/Button/InteractionCallback remain unchanged.
use super::*;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Emoji text, or the platform's structured custom-emoji reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::EmojiRef;
/// let emoji = EmojiRef::custom(uuid::Uuid::nil(), "wave", false);
/// assert_eq!(emoji.text(), format!("<:wave:{}>", uuid::Uuid::nil()));
/// ```
pub enum EmojiRef {
    /// Unicode emoji or a textual emoji reference.
    Text(#[doc = "Text payload carried by this variant."] String),
    /// Structured custom-emoji identity; usability still depends on platform server membership rules.
    Custom {
        /// Custom emoji identifier.
        id: Uuid,
        /// Custom emoji name.
        name: String,
        #[serde(default)]
        /// Whether to use the animated textual reference form.
        animated: bool,
    },
}
impl EmojiRef {
    /// Construct a structured emoji reference; this does not authorize using an emoji from another server.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmojiRef;
    /// let emoji = EmojiRef::custom(uuid::Uuid::nil(), "wave", false);
    /// assert_eq!(emoji.text(), format!("<:wave:{}>", uuid::Uuid::nil()));
    /// ```
    pub fn custom(id: Uuid, name: impl Into<String>, animated: bool) -> Self {
        Self::Custom {
            id,
            name: name.into(),
            animated,
        }
    }
    /// Format the emoji as text, preserving whether a custom reference is animated.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmojiRef;
    /// let emoji = EmojiRef::custom(uuid::Uuid::nil(), "wave", false);
    /// assert_eq!(emoji.text(), format!("<:wave:{}>", uuid::Uuid::nil()));
    /// ```
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
/// One application-defined option in a select menu.
///
/// # Example
///
/// ```rust
/// let option = rappin::SelectOption::new("Apple", "apple").emoji("🍎");
/// assert_eq!(option.value, "apple");
/// ```
pub struct SelectOption {
    /// Visible option caption, limited to 100 characters by UI validation.
    pub label: String,
    /// Application-defined value delivered when the option is selected.
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional secondary option text.
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional text or structured custom emoji.
    pub emoji: Option<EmojiRef>,
    #[serde(default)]
    /// Whether this option is initially selected.
    pub default: bool,
}
impl SelectOption {
    /// Create an option without description, emoji or initial selection.
    ///
    /// # Example
    ///
    /// ```rust
    /// let option = rappin::SelectOption::new("Apple", "apple").emoji("🍎");
    /// assert_eq!(option.value, "apple");
    /// ```
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            description: None,
            emoji: None,
            default: false,
        }
    }
    /// Attach a text or structured custom emoji.
    ///
    /// # Example
    ///
    /// ```rust
    /// let option = rappin::SelectOption::new("Apple", "apple").emoji("🍎");
    /// assert_eq!(option.value, "apple");
    /// ```
    pub fn emoji(mut self, emoji: impl Into<EmojiRef>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// A dropdown menu that must occupy its own action row.
///
/// # Example
///
/// ```rust
/// use rappin::{SelectMenu, SelectOption};
/// let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
/// assert_eq!(menu.options.len(), 1);
/// ```
pub struct SelectMenu {
    #[serde(rename = "type")]
    /// Wire discriminator; constructors use `select`.
    pub kind: String,
    /// Application identifier returned with selection events.
    pub custom_id: String,
    /// Available options; validation requires 1 through 25.
    pub options: Vec<SelectOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional prompt shown before a value is selected.
    pub placeholder: Option<String>,
    /// Minimum number of selections, checked against the available option count.
    pub min_values: u8,
    /// Maximum number of selections, checked against the available option count.
    pub max_values: u8,
    /// Whether the client disables the menu.
    pub disabled: bool,
}
impl SelectMenu {
    /// Create an enabled single-selection menu with no options. Add options before validating or sending.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{SelectMenu, SelectOption};
    /// let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
    /// assert_eq!((menu.min_values, menu.max_values), (1, 1));
    /// ```
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
    /// Append an option, preserving insertion order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{SelectMenu, SelectOption};
    /// let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
    /// assert_eq!(menu.options[0].value, "apple");
    /// ```
    pub fn option(mut self, option: SelectOption) -> Self {
        self.options.push(option);
        self
    }
    /// Wrap this menu in a dedicated action row.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{SelectMenu, SelectOption};
    /// let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
    /// let row = menu.row();
    /// assert_eq!(row.as_json()["components"][0]["custom_id"], "fruit");
    /// ```
    pub fn row(self) -> Component {
        Component::row([Component::Raw(json!(self))])
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
/// Single-line or paragraph text-entry rendering for a modal field.
///
/// # Example
///
/// ```rust
/// let style = rappin::TextInputStyle::Paragraph;
/// assert_eq!(serde_json::to_value(style).unwrap(), "paragraph");
/// ```
pub enum TextInputStyle {
    /// Single-line text entry.
    Short,
    /// Multi-line text entry.
    Paragraph,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// A text-entry field in a modal form.
///
/// # Example
///
/// ```rust
/// let mut field = rappin::TextInput::short("name", "Display name");
/// field.max_length = Some(32);
/// assert!(field.required);
/// ```
pub struct TextInput {
    #[serde(rename = "type")]
    /// Wire discriminator; constructors use `text_input`.
    pub kind: String,
    /// Application field ID used to retrieve the submitted value.
    pub custom_id: String,
    /// Visible field heading.
    pub label: String,
    /// Single-line or paragraph input style.
    pub style: TextInputStyle,
    /// Whether a value must be provided; constructors default to true.
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional explanatory field text.
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional hint shown before the user enters text.
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional initial field value.
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional minimum text length checked by modal validation.
    pub min_length: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional maximum text length checked by modal validation.
    pub max_length: Option<u16>,
}
impl TextInput {
    /// Create a required single-line input with no initial value or length bounds.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut field = rappin::TextInput::short("name", "Display name");
    /// field.max_length = Some(32);
    /// assert!(field.required);
    /// ```
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
    /// Create a required multi-line input.
    ///
    /// # Example
    ///
    /// ```rust
    /// let field = rappin::TextInput::paragraph("comment", "Comment");
    /// assert_eq!(field.style, rappin::TextInputStyle::Paragraph);
    /// ```
    pub fn paragraph(custom_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            style: TextInputStyle::Paragraph,
            ..Self::short(custom_id, label)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// A labeled select menu embedded in a modal form.
///
/// # Example
///
/// ```rust
/// use rappin::{SelectMenu, SelectOption};
/// let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
/// let field = rappin::ModalSelect::new("Fruit", menu);
/// assert!(field.required);
/// ```
pub struct ModalSelect {
    #[serde(flatten)]
    /// Flattened select-menu definition used as a modal input.
    pub select: SelectMenu,
    /// Visible field heading for the modal select.
    pub label: String,
    /// Whether this modal input must be supplied.
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional explanation for the modal select.
    pub description: Option<String>,
}
impl ModalSelect {
    /// Create a required labeled select input.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{SelectMenu, SelectOption};
    /// let menu = SelectMenu::new("fruit").option(SelectOption::new("Apple", "apple"));
    /// let field = rappin::ModalSelect::new("Fruit", menu);
    /// assert!(field.required);
    /// ```
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
/// A text or select field accepted by a modal.
///
/// # Example
///
/// ```rust
/// let input: rappin::ModalInput = rappin::TextInput::short("name", "Name").into();
/// assert!(matches!(input, rappin::ModalInput::Text(_)));
/// ```
pub enum ModalInput {
    /// A text-entry field.
    Text(#[doc = "Text payload carried by this variant."] TextInput),
    /// A select-menu field with its own label.
    Select(#[doc = "Typed select input definition."] ModalSelect),
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
/// A modal form opened as an eligible initial interaction response.
///
/// # Example
///
/// ```rust
/// use rappin::{Modal, TextInput};
/// let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Your comment"));
/// modal.validate().unwrap();
/// ```
pub struct Modal {
    /// Application identifier delivered with the form submission.
    pub custom_id: String,
    /// Modal heading; validation limits it to 45 characters.
    pub title: String,
    /// Modal inputs in display order; validation requires 1 through 5.
    pub components: Vec<ModalInput>,
}
impl Modal {
    /// Create an empty form; add at least one input before opening it.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Modal, TextInput};
    /// let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Your comment"));
    /// modal.validate().unwrap();
    /// ```
    pub fn new(custom_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            custom_id: custom_id.into(),
            title: title.into(),
            components: vec![],
        }
    }
    /// Append a text or select input to the form.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Modal, TextInput};
    /// let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Your comment"));
    /// modal.validate().unwrap();
    /// ```
    pub fn input(mut self, input: impl Into<ModalInput>) -> Self {
        self.components.push(input.into());
        self
    }
    /// Check form IDs, title, input count and field bounds without sending a response.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Modal, TextInput};
    /// let modal = Modal::new("feedback", "Feedback").input(TextInput::paragraph("comment", "Your comment"));
    /// modal.validate().unwrap();
    /// assert!(rappin::Modal::new("empty", "Empty").validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        crate::validation::validate_modal(&json!(self))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
/// One submitted modal field value.
///
/// # Example
///
/// ```rust
/// let value: rappin::ModalValue = serde_json::from_str(r#"["apple"]"#).unwrap();
/// assert!(matches!(value, rappin::ModalValue::Selected(_)));
/// ```
pub enum ModalValue {
    /// Text submitted for one field.
    Text(#[doc = "Text payload carried by this variant."] String),
    /// Option values selected for one modal field.
    Selected(#[doc = "Selected option values delivered by the interaction."] Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
/// Typed values delivered with select or modal-submit interactions.
///
/// # Example
///
/// ```rust
/// let values: rappin::InteractionValues = serde_json::from_str(r#"{"name":"River"}"#).unwrap();
/// assert!(matches!(values, rappin::InteractionValues::Submitted(_)));
/// ```
pub enum InteractionValues {
    /// Selected values from a standalone menu.
    Selected(#[doc = "Selected option values delivered by the interaction."] Vec<String>),
    /// Modal field IDs mapped to text or selected values.
    Submitted(
        #[doc = "Submitted modal field IDs and their typed values."] BTreeMap<String, ModalValue>,
    ),
}
/// Extended event view. Credential Debug output delegates to the redacted base.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"interaction","data":{"id":"example","token":"unused-example-credential","type":"select","custom_id":"fruit","values":["apple"]}})).unwrap();
/// let interaction = rappin::UiInteraction::from_event(&event).unwrap();
/// assert!(matches!(interaction.values, Some(rappin::InteractionValues::Selected(_))));
/// ```
pub struct UiInteraction {
    #[serde(flatten)]
    /// Base interaction identity, callback credential, scope and deadline metadata.
    pub interaction: InteractionEvent,
    #[serde(default)]
    /// Selected option values or submitted modal fields, depending on interaction kind.
    pub values: Option<InteractionValues>,
    #[serde(default)]
    /// Command name when the platform delivers a slash command as an interaction.
    pub command: Option<String>,
    #[serde(default)]
    /// Raw command arguments when included in the interaction event.
    pub args: Option<String>,
}
impl From<InteractionEvent> for UiInteraction {
    fn from(interaction: InteractionEvent) -> Self {
        Self {
            interaction,
            values: None,
            command: None,
            args: None,
        }
    }
}
impl UiInteraction {
    /// Decode a select, modal or command interaction from a raw event. Rejects mismatched select/modal values and preserves the base credential and deadline metadata.
    ///
    /// # Example
    ///
    /// ```rust
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"interaction","data":{"id":"example","token":"unused-example-credential","type":"select","custom_id":"fruit","values":["apple"]}})).unwrap();
    /// let interaction = rappin::UiInteraction::from_event(&event).unwrap();
    /// assert!(matches!(interaction.values, Some(rappin::InteractionValues::Selected(_))));
    /// ```
    pub fn from_event(event: &crate::Event) -> Result<Self> {
        if !matches!(event.kind.as_str(), "interaction" | "command") {
            return Err(Error::validation(
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
                Error::validation("values", "select requires selected values"),
            ),
            Some("modal_submit")
                if !matches!(value.values, Some(InteractionValues::Submitted(_))) =>
            {
                Err(Error::validation(
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
/// One custom emoji discovered in a message.
///
/// # Example
///
/// ```rust
/// let emoji: rappin::FoundEmoji = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"wave","animated":false,"url":"https://example.com/wave.png","in_server":true,"text":"wave"})).unwrap();
/// assert!(emoji.in_server);
/// ```
pub struct FoundEmoji {
    /// Custom emoji identifier.
    pub id: Uuid,
    /// Emoji name returned by the platform.
    pub name: String,
    /// Whether the emoji asset is animated.
    pub animated: bool,
    /// Image URL returned by emoji discovery.
    pub url: String,
    /// Whether the emoji belongs to the queried message's server according to the API.
    pub in_server: bool,
    /// Platform-provided textual reference for this emoji.
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// Result of a scoped message-emoji lookup.
///
/// # Example
///
/// ```rust
/// let result: rappin::MessageEmojis = serde_json::from_str(r#"{"emojis":[]}"#).unwrap();
/// assert!(result.emojis.is_empty());
/// ```
pub struct MessageEmojis {
    /// Custom emojis identified in the selected message.
    pub emojis: Vec<FoundEmoji>,
}

impl crate::collectors::Collectible for UiInteraction {
    fn from_event(event: &crate::Event) -> Option<Self> {
        Self::from_event(event).ok()
    }
}

impl Button {
    /// Preserve the legacy string field while accepting a typed emoji reference.
    /// # Example
    ///
    /// ```rust
    /// let emoji = rappin::EmojiRef::custom(uuid::Uuid::nil(), "wave", false);
    /// let button = rappin::Button::primary("Wave", "wave").with_emoji_ref(emoji);
    /// assert_eq!(button.emoji.as_deref(), Some(format!("<:wave:{}>", uuid::Uuid::nil()).as_str()));
    /// ```
    pub fn with_emoji_ref(self, emoji: impl Into<EmojiRef>) -> Self {
        self.with_emoji(emoji.into().text())
    }
}
