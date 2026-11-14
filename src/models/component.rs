//! Button rows and Components V2 layout builders. Select menus and modal inputs use the extended `ui` model module.
//!
//! # Example
//!
//! ```rust
//! use rappin::{Container, TextDisplay};
//! let panel = Container::new().accent(0x5865F2).add(TextDisplay::new("Welcome"));
//! assert_eq!(panel.components.len(), 1);
//! ```
use serde::{Deserialize, Serialize};

/// Button style options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
/// # Example
///
/// ```rust
/// use rappin::ButtonStyle;
/// let style: ButtonStyle = serde_json::from_str("3").unwrap();
/// assert_eq!(style, ButtonStyle::Success);
/// ```
pub enum ButtonStyle {
    /// A prominent primary action.
    Primary,
    /// A neutral secondary action.
    Secondary,
    /// A positive or confirm action.
    Success,
    /// A destructive or cancel action.
    Danger,
    /// A URL link; it opens a page instead of dispatching a button callback.
    Link,
}

/// An interactive button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::Button;
/// let button = Button::success("Approve", "approval:yes");
/// assert_eq!(button.custom_id.as_deref(), Some("approval:yes"));
/// ```
pub struct Button {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "component_kind")]
    /// Wire component discriminator; constructors set it to `button`.
    pub kind: String,
    /// Visual style; link buttons use a URL instead of a custom callback ID.
    pub style: ButtonStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional visible button caption.
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional text emoji displayed beside the caption.
    pub emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Application callback identifier used to match button handlers.
    pub custom_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Destination URL for a link-style button.
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// Whether the client disables the control; defaults to false.
    pub disabled: bool,
}

impl Button {
    /// Create an enabled button with the selected style. Supply a label or emoji and the matching custom ID or link URL before sending.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Button, ButtonStyle};
    /// let button = Button::new(ButtonStyle::Primary).with_label("Open").with_custom_id("open");
    /// assert!(!button.disabled);
    /// ```
    pub fn new(style: ButtonStyle) -> Self {
        Self {
            kind: "button".to_string(),
            style,
            label: None,
            emoji: None,
            custom_id: None,
            url: None,
            disabled: false,
        }
    }

    /// Create an enabled primary button with a caption and application callback ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Button, ButtonStyle};
    /// let button = Button::primary("Choose", "choice:one");
    /// assert_eq!(button.style, ButtonStyle::Primary);
    /// assert_eq!(button.custom_id.as_deref(), Some("choice:one"));
    /// ```
    pub fn primary(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Primary)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    /// Create an enabled secondary button with a caption and application callback ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Button, ButtonStyle};
    /// let button = Button::secondary("Choose", "choice:one");
    /// assert_eq!(button.style, ButtonStyle::Secondary);
    /// assert_eq!(button.custom_id.as_deref(), Some("choice:one"));
    /// ```
    pub fn secondary(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Secondary)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    /// Create an enabled success button with a caption and application callback ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Button, ButtonStyle};
    /// let button = Button::success("Choose", "choice:one");
    /// assert_eq!(button.style, ButtonStyle::Success);
    /// assert_eq!(button.custom_id.as_deref(), Some("choice:one"));
    /// ```
    pub fn success(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Success)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    /// Create an enabled danger button with a caption and application callback ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Button, ButtonStyle};
    /// let button = Button::danger("Choose", "choice:one");
    /// assert_eq!(button.style, ButtonStyle::Danger);
    /// assert_eq!(button.custom_id.as_deref(), Some("choice:one"));
    /// ```
    pub fn danger(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Danger)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    /// Create a link button. Clicking it opens the URL and does not dispatch a custom-ID handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// let button = rappin::Button::link("Website", "https://example.com");
    /// assert!(button.custom_id.is_none());
    /// assert_eq!(button.url.as_deref(), Some("https://example.com"));
    /// ```
    pub fn link(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Link).with_label(label).with_url(url)
    }

    /// Set the visible caption.
    ///
    /// # Example
    ///
    /// ```rust
    /// let button = rappin::Button::primary("Continue", "continue").with_label("Continue");
    /// assert_eq!(button.label.as_deref(), Some("Continue"));
    /// ```
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set a text emoji beside the caption. Use the extended UI types for structured custom emoji references.
    ///
    /// # Example
    ///
    /// ```rust
    /// let button = rappin::Button::primary("Continue", "continue").with_emoji("✅");
    /// assert_eq!(button.emoji.as_deref(), Some("✅"));
    /// ```
    pub fn with_emoji(mut self, emoji: impl Into<String>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }

    /// Set the application callback identifier. It grants no permission by itself.
    ///
    /// # Example
    ///
    /// ```rust
    /// let button = rappin::Button::primary("Continue", "continue").with_custom_id("action:continue");
    /// assert_eq!(button.custom_id.as_deref(), Some("action:continue"));
    /// ```
    pub fn with_custom_id(mut self, custom_id: impl Into<String>) -> Self {
        self.custom_id = Some(custom_id.into());
        self
    }

    /// Set the URL used by a link-style button. This setter does not change the button style.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Button, ButtonStyle};
    /// let button = Button::new(ButtonStyle::Link).with_label("Website").with_url("https://example.com");
    /// assert_eq!(button.url.as_deref(), Some("https://example.com"));
    /// ```
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Enable or disable client interaction with this button.
    ///
    /// # Example
    ///
    /// ```rust
    /// let button = rappin::Button::primary("Continue", "continue").disabled(true);
    /// assert_eq!(button.disabled, true);
    /// ```
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// A row of buttons (up to 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{ActionRow, Button};
/// let row = ActionRow::new([Button::primary("Next", "page:next")]);
/// assert_eq!(row.components.len(), 1);
/// ```
pub struct ActionRow {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "component_kind")]
    /// Wire component discriminator; constructors set it to `row`.
    pub kind: String,
    /// Buttons in display order; message validation allows at most five.
    pub components: Vec<Button>,
}

impl ActionRow {
    /// Collect buttons into one action row. Message validation checks the maximum of five buttons.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{ActionRow, Button};
    /// let row = ActionRow::new([Button::primary("Next", "page:next")]);
    /// assert_eq!(row.components.len(), 1);
    /// ```
    pub fn new(buttons: impl IntoIterator<Item = Button>) -> Self {
        Self {
            kind: "row".to_string(),
            components: buttons.into_iter().collect(),
        }
    }

    /// Append a button in place. The resulting message must still satisfy row-size limits.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{ActionRow, Button};
    /// let mut row = ActionRow::new([]);
    /// row.add_button(Button::primary("Next", "next"));
    /// assert_eq!(row.components.len(), 1);
    /// ```
    pub fn add_button(&mut self, button: Button) {
        self.components.push(button);
    }
}

/// Generic message component (ActionRow or Components V2 LayoutComponent).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
/// # Example
///
/// ```rust
/// use rappin::{Component, ActionRow, Button};
/// let component: Component = ActionRow::new([Button::primary("Next", "page:next")]).into();
/// assert!(matches!(component, Component::Row(_)));
/// ```
pub enum Component {
    /// A legacy action row of buttons.
    Row(#[doc = "Action-row payload containing its controls."] ActionRow),
    /// A Components V2 layout element.
    V2(#[doc = "Components V2 layout payload."] LayoutComponent),
    /// Lossless response tree, including selects and future component fields.
    Raw(#[doc = "Original component JSON; outbound validation still applies."] serde_json::Value),
}

impl Component {
    /// Collect buttons or a select menu into a validated outbound action row.
    ///
    /// ```rust
    /// let row = rappin::Component::row([rappin::Button::primary("Next", "next")]);
    /// assert_eq!(row.as_json()["type"], "row");
    /// ```
    pub fn row(children: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self::Raw(
            serde_json::json!({"type":"row","components":children.into_iter().map(Into::into).collect::<Vec<Self>>()}),
        )
    }

    /// Collect components into a Components V2 container.
    ///
    /// ```rust
    /// let panel = rappin::Component::container([rappin::TextDisplay::new("Hello")]);
    /// assert_eq!(panel.as_json()["type"], 17);
    /// ```
    pub fn container(children: impl IntoIterator<Item = impl Into<Self>>) -> Self {
        Self::Raw(
            serde_json::json!({"type":17,"components":children.into_iter().map(Into::into).collect::<Vec<Self>>()}),
        )
    }

    /// Create a button with a Unicode or custom emoji reference.
    ///
    /// ```rust
    /// let button = rappin::Component::button(rappin::Button::primary("Go", "go"), "🦀");
    /// assert_eq!(button.as_json()["emoji"], "🦀");
    /// ```
    pub fn button(button: Button, emoji: impl Into<crate::EmojiRef>) -> Self {
        let mut value = serde_json::json!(button);
        value["emoji"] = serde_json::json!(emoji.into());
        Self::Raw(value)
    }

    /// Return the complete component JSON, including unknown response fields.
    ///
    /// ```rust
    /// let row = rappin::Component::row([rappin::Button::primary("Go", "go")]);
    /// assert_eq!(row.as_json()["components"][0]["custom_id"], "go");
    /// ```
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!(self)
    }
}

impl From<Button> for Component {
    fn from(button: Button) -> Self {
        Self::Raw(serde_json::json!(button))
    }
}
impl From<crate::SelectMenu> for Component {
    fn from(select: crate::SelectMenu) -> Self {
        Self::Raw(serde_json::json!(select))
    }
}

impl From<ActionRow> for Component {
    fn from(row: ActionRow) -> Self {
        Component::Row(row)
    }
}

impl From<LayoutComponent> for Component {
    fn from(lc: LayoutComponent) -> Self {
        Component::V2(lc)
    }
}

impl From<Container> for Component {
    fn from(c: Container) -> Self {
        Component::V2(LayoutComponent::Container(c))
    }
}

impl From<Section> for Component {
    fn from(s: Section) -> Self {
        Component::V2(LayoutComponent::Section(s))
    }
}

impl From<TextDisplay> for Component {
    fn from(t: TextDisplay) -> Self {
        Component::V2(LayoutComponent::Text(t))
    }
}

impl From<ThumbnailComponent> for Component {
    fn from(t: ThumbnailComponent) -> Self {
        Component::V2(LayoutComponent::Thumbnail(t))
    }
}

impl From<MediaGallery> for Component {
    fn from(g: MediaGallery) -> Self {
        Component::V2(LayoutComponent::Gallery(g))
    }
}

impl From<Separator> for Component {
    fn from(s: Separator) -> Self {
        Component::V2(LayoutComponent::Separator(s))
    }
}

impl From<FileDisplay> for Component {
    fn from(f: FileDisplay) -> Self {
        Component::V2(LayoutComponent::File(f))
    }
}

// ---------------------------------------------------------------------------
// Components V2 definitions (types 1, 2, 9, 10, 11, 12, 13, 14, 17)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Media source shared by thumbnails and gallery items.
///
/// # Example
///
/// ```rust
/// let image = rappin::MediaUrl { url: "https://example.com/photo.png".into() };
/// assert!(image.url.ends_with(".png"));
/// ```
pub struct MediaUrl {
    /// Original media source URL, such as an HTTPS image or attachment URL.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// File URL wrapper used by a file-display component.
///
/// # Example
///
/// ```rust
/// let file = rappin::FileUrl { url: "https://example.com/report.txt".into() };
/// assert!(file.url.ends_with(".txt"));
/// ```
pub struct FileUrl {
    /// File source URL used by a file display.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One image or media item in a Components V2 gallery.
///
/// # Example
///
/// ```rust
/// let item = rappin::MediaItem::new("https://example.com/photo.png").with_description("A sunset");
/// assert_eq!(item.description.as_deref(), Some("A sunset"));
/// ```
pub struct MediaItem {
    /// Image or media source for this gallery item.
    pub media: MediaUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional accessible description for this media item.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// Whether clients conceal this item until the user reveals it.
    pub spoiler: bool,
}

impl MediaItem {
    /// Create a non-spoiler media item without a description.
    ///
    /// # Example
    ///
    /// ```rust
    /// let item = rappin::MediaItem::new("https://example.com/photo.png").with_description("A sunset");
    /// assert_eq!(item.description.as_deref(), Some("A sunset"));
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            media: MediaUrl { url: url.into() },
            description: None,
            spoiler: false,
        }
    }

    /// Set the accessible description for this gallery item.
    ///
    /// # Example
    ///
    /// ```rust
    /// let item = rappin::MediaItem::new("https://example.com/photo.png").with_description("A sunset");
    /// assert_eq!(item.description.as_deref(), Some("A sunset"));
    /// ```
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Choose whether to conceal the item until a user reveals it.
    ///
    /// # Example
    ///
    /// ```rust
    /// let item = rappin::MediaItem::new("https://example.com/photo.png").spoiler(true);
    /// assert!(item.spoiler);
    /// ```
    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
/// A button or thumbnail attached to a Components V2 section.
///
/// # Example
///
/// ```rust
/// use rappin::{SectionAccessory, ThumbnailComponent};
/// let accessory: SectionAccessory = ThumbnailComponent::new("https://example.com/photo.png").into();
/// assert!(matches!(accessory, SectionAccessory::Thumbnail(_)));
/// ```
pub enum SectionAccessory {
    /// An interactive or link button.
    Button(#[doc = "Button payload used as an accessory."] Button),
    /// A thumbnail image.
    Thumbnail(#[doc = "Thumbnail accessory payload."] ThumbnailComponent),
}

impl From<Button> for SectionAccessory {
    fn from(b: Button) -> Self {
        SectionAccessory::Button(b)
    }
}

impl From<ThumbnailComponent> for SectionAccessory {
    fn from(t: ThumbnailComponent) -> Self {
        SectionAccessory::Thumbnail(t)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Components V2 thumbnail with optional accessible description and spoiler flag.
///
/// # Example
///
/// ```rust
/// let thumbnail = rappin::ThumbnailComponent::new("https://example.com/photo.png");
/// assert_eq!(thumbnail.kind, 11);
/// ```
pub struct ThumbnailComponent {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 11 for thumbnails.
    pub kind: u8, // 11
    /// Thumbnail image source.
    pub media: MediaUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional accessible thumbnail description.
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// Whether to conceal the thumbnail as a spoiler.
    pub spoiler: bool,
}

impl ThumbnailComponent {
    /// Create a non-spoiler thumbnail without a description.
    ///
    /// # Example
    ///
    /// ```rust
    /// let thumbnail = rappin::ThumbnailComponent::new("https://example.com/photo.png");
    /// assert_eq!(thumbnail.kind, 11);
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            kind: 11,
            media: MediaUrl { url: url.into() },
            description: None,
            spoiler: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A Components V2 block of text.
///
/// # Example
///
/// ```rust
/// let text = rappin::TextDisplay::new("# Results");
/// assert_eq!(text.content, "# Results");
/// ```
pub struct TextDisplay {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 10 for text displays.
    pub kind: u8, // 10
    /// Text rendered by the client in this layout component.
    pub content: String,
}

impl TextDisplay {
    /// Create a block of layout text.
    ///
    /// # Example
    ///
    /// ```rust
    /// let text = rappin::TextDisplay::new("# Results");
    /// assert_eq!(text.content, "# Results");
    /// ```
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            kind: 10,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A Components V2 text section with an optional side accessory.
///
/// # Example
///
/// ```rust
/// use rappin::{Section, Button};
/// let section = Section::new("Confirm your choice").accessory(Button::success("Confirm", "confirm"));
/// assert!(section.accessory.is_some());
/// ```
pub struct Section {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 9 for sections.
    pub kind: u8, // 9
    /// Text displays rendered in this section, in insertion order.
    pub components: Vec<TextDisplay>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional button or thumbnail shown beside the section text.
    pub accessory: Option<SectionAccessory>,
}

impl Section {
    /// Create a section containing one text display.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Section, Button};
    /// let section = Section::new("Confirm your choice").accessory(Button::success("Confirm", "confirm"));
    /// assert!(section.accessory.is_some());
    /// ```
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            kind: 9,
            components: vec![TextDisplay::new(text)],
            accessory: None,
        }
    }

    /// Append a text display to this section.
    ///
    /// # Example
    ///
    /// ```rust
    /// let section = rappin::Section::new("Heading").add_text("Details");
    /// assert_eq!(section.components.len(), 2);
    /// ```
    pub fn add_text(mut self, text: impl Into<String>) -> Self {
        self.components.push(TextDisplay::new(text));
        self
    }

    /// Attach or replace the side button or thumbnail.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Section, Button};
    /// let section = Section::new("Confirm your choice").accessory(Button::success("Confirm", "confirm"));
    /// assert!(section.accessory.is_some());
    /// ```
    pub fn accessory(mut self, accessory: impl Into<SectionAccessory>) -> Self {
        self.accessory = Some(accessory.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A Components V2 gallery of media items.
///
/// # Example
///
/// ```rust
/// use rappin::{MediaGallery, MediaItem};
/// let gallery = MediaGallery::new([MediaItem::new("https://example.com/photo.png")]);
/// assert_eq!(gallery.items.len(), 1);
/// ```
pub struct MediaGallery {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 12 for media galleries.
    pub kind: u8, // 12
    /// Gallery media items, in display order.
    pub items: Vec<MediaItem>,
}

impl MediaGallery {
    /// Collect gallery items in display order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{MediaGallery, MediaItem};
    /// let gallery = MediaGallery::new([MediaItem::new("https://example.com/photo.png")]);
    /// assert_eq!(gallery.items.len(), 1);
    /// ```
    pub fn new(items: impl IntoIterator<Item = MediaItem>) -> Self {
        Self {
            kind: 12,
            items: items.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A visible divider or spacing-only Components V2 separator.
///
/// # Example
///
/// ```rust
/// let line = rappin::Separator::divider();
/// assert!(line.divider);
/// assert!(!rappin::Separator::space(true).divider);
/// ```
pub struct Separator {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 14 for separators.
    pub kind: u8, // 14
    /// Whether to show a visible dividing line instead of spacing only.
    pub divider: bool,
    /// Spacing size: 1 for small or 2 for large.
    pub spacing: u8, // 1 (small) or 2 (large)
}

impl Separator {
    /// Create a visible divider with small spacing.
    ///
    /// # Example
    ///
    /// ```rust
    /// let line = rappin::Separator::divider();
    /// assert!(line.divider);
    /// assert!(!rappin::Separator::space(true).divider);
    /// ```
    pub fn divider() -> Self {
        Self {
            kind: 14,
            divider: true,
            spacing: 1,
        }
    }

    /// Create spacing without a line. Pass true for large spacing or false for small spacing.
    ///
    /// # Example
    ///
    /// ```rust
    /// assert_eq!(rappin::Separator::space(false).spacing, 1);
    /// assert_eq!(rappin::Separator::space(true).spacing, 2);
    /// ```
    pub fn space(large: bool) -> Self {
        Self {
            kind: 14,
            divider: false,
            spacing: if large { 2 } else { 1 },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A Components V2 file reference.
///
/// # Example
///
/// ```rust
/// let file = rappin::FileDisplay::new("https://example.com/report.txt");
/// assert_eq!(file.file.url, "https://example.com/report.txt");
/// ```
pub struct FileDisplay {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 13 for file displays.
    pub kind: u8, // 13
    /// File source displayed by this component.
    pub file: FileUrl,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// Whether to conceal the file as a spoiler.
    pub spoiler: bool,
}

impl FileDisplay {
    /// Create a non-spoiler file display.
    ///
    /// # Example
    ///
    /// ```rust
    /// let file = rappin::FileDisplay::new("https://example.com/report.txt");
    /// assert_eq!(file.file.url, "https://example.com/report.txt");
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            kind: 13,
            file: FileUrl { url: url.into() },
            spoiler: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A Components V2 group of layout components. Send it through a layout message with flag 32768.
///
/// # Example
///
/// ```rust
/// use rappin::{Container, TextDisplay};
/// let panel = Container::new().accent(0x5865F2).add(TextDisplay::new("Welcome"));
/// assert_eq!(panel.components.len(), 1);
/// ```
pub struct Container {
    #[serde(rename = "type")]
    /// Numeric wire discriminator, 17 for containers.
    pub kind: u8, // 17
    /// Child layout components in display order.
    pub components: Vec<LayoutComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional packed 24-bit accent color.
    pub accent_color: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// Whether clients conceal the container as a spoiler.
    pub spoiler: bool,
}

impl Container {
    /// Create an empty layout container without an accent color.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Container, TextDisplay};
    /// let panel = Container::new().accent(0x5865F2).add(TextDisplay::new("Welcome"));
    /// assert_eq!(panel.components.len(), 1);
    /// ```
    pub fn new() -> Self {
        Self {
            kind: 17,
            components: Vec::new(),
            accent_color: None,
            spoiler: false,
        }
    }

    /// Set the container accent color as a packed 24-bit integer.
    ///
    /// # Example
    ///
    /// ```rust
    /// let panel = rappin::Container::new().accent(0x5865F2);
    /// assert_eq!(panel.accent_color, Some(0x5865F2));
    /// ```
    pub fn accent(mut self, color: u32) -> Self {
        self.accent_color = Some(color);
        self
    }

    /// Choose whether clients conceal this container as a spoiler.
    ///
    /// # Example
    ///
    /// ```rust
    /// let panel = rappin::Container::new().spoiler(true);
    /// assert!(panel.spoiler);
    /// ```
    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }

    #[allow(clippy::should_implement_trait)]
    /// Append a child layout component, retaining insertion order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Container, TextDisplay};
    /// let panel = Container::new().accent(0x5865F2).add(TextDisplay::new("Welcome"));
    /// assert_eq!(panel.components.len(), 1);
    /// ```
    pub fn add(mut self, component: impl Into<LayoutComponent>) -> Self {
        self.components.push(component.into());
        self
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

/// Components V2 layout component variant.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
/// # Example
///
/// ```rust
/// use rappin::{LayoutComponent, TextDisplay};
/// let component: LayoutComponent = TextDisplay::new("Welcome").into();
/// assert!(matches!(component, LayoutComponent::Text(_)));
/// ```
pub enum LayoutComponent {
    /// A nested component group.
    Container(#[doc = "Container layout payload."] Container),
    /// Text with an optional button or thumbnail accessory.
    Section(#[doc = "Section layout payload."] Section),
    /// A standalone text display.
    Text(#[doc = "Text payload carried by this variant."] TextDisplay),
    /// A thumbnail image.
    Thumbnail(#[doc = "Thumbnail accessory payload."] ThumbnailComponent),
    /// A gallery of media items.
    Gallery(#[doc = "Media-gallery layout payload."] MediaGallery),
    /// A dividing line or spacing.
    Separator(#[doc = "Separator layout payload."] Separator),
    /// A file reference.
    File(#[doc = "File component payload."] FileDisplay),
    /// A legacy row of buttons embedded in a layout.
    Row(#[doc = "Action-row payload containing its controls."] ActionRow),
}

impl From<Container> for LayoutComponent {
    fn from(c: Container) -> Self {
        LayoutComponent::Container(c)
    }
}

impl From<Section> for LayoutComponent {
    fn from(s: Section) -> Self {
        LayoutComponent::Section(s)
    }
}

impl From<TextDisplay> for LayoutComponent {
    fn from(t: TextDisplay) -> Self {
        LayoutComponent::Text(t)
    }
}

impl From<ThumbnailComponent> for LayoutComponent {
    fn from(t: ThumbnailComponent) -> Self {
        LayoutComponent::Thumbnail(t)
    }
}

impl From<MediaGallery> for LayoutComponent {
    fn from(g: MediaGallery) -> Self {
        LayoutComponent::Gallery(g)
    }
}

impl From<Separator> for LayoutComponent {
    fn from(s: Separator) -> Self {
        LayoutComponent::Separator(s)
    }
}

impl From<FileDisplay> for LayoutComponent {
    fn from(f: FileDisplay) -> Self {
        LayoutComponent::File(f)
    }
}

fn component_kind<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v.as_i64() {
        Some(1) => Ok("row".into()),
        Some(2) => Ok("button".into()),
        _ => v
            .as_str()
            .map(String::from)
            .ok_or_else(|| serde::de::Error::custom("invalid component type")),
    }
}
impl<'de> Deserialize<'de> for ButtonStyle {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match (v.as_str(), v.as_i64()) {
            (Some("primary"), _) | (_, Some(1)) => Ok(Self::Primary),
            (Some("secondary"), _) | (_, Some(2)) => Ok(Self::Secondary),
            (Some("success"), _) | (_, Some(3)) => Ok(Self::Success),
            (Some("danger"), _) | (_, Some(4)) => Ok(Self::Danger),
            (Some("link"), _) | (_, Some(5)) => Ok(Self::Link),
            _ => Err(serde::de::Error::custom("invalid button style")),
        }
    }
}
impl<'de> Deserialize<'de> for Component {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        let known = if v["type"] == "row" || v["type"] == 1 {
            serde_json::from_value(v.clone()).map(Self::Row)
        } else {
            serde_json::from_value(v.clone()).map(Self::V2)
        };
        Ok(match known {
            Ok(component) if serde_json::json!(&component) == v => component,
            _ => Self::Raw(v),
        })
    }
}
impl<'de> Deserialize<'de> for LayoutComponent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        let kind = v["type"].as_i64();
        let result = match kind {
            Some(17) => serde_json::from_value(v).map(Self::Container),
            Some(9) => serde_json::from_value(v).map(Self::Section),
            Some(10) => serde_json::from_value(v).map(Self::Text),
            Some(11) => serde_json::from_value(v).map(Self::Thumbnail),
            Some(12) => serde_json::from_value(v).map(Self::Gallery),
            Some(14) => serde_json::from_value(v).map(Self::Separator),
            Some(13) => serde_json::from_value(v).map(Self::File),
            Some(1) => serde_json::from_value(v).map(Self::Row),
            _ if v["type"] == "row" => serde_json::from_value(v).map(Self::Row),
            _ => return Err(serde::de::Error::custom("unknown layout component type")),
        };
        result.map_err(serde::de::Error::custom)
    }
}
