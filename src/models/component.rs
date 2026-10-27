use serde::{Deserialize, Serialize};

/// Button style options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Success,
    Danger,
    Link,
}

/// An interactive button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "component_kind")]
    pub kind: String,
    pub style: ButtonStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

impl Button {
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

    pub fn primary(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Primary)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    pub fn secondary(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Secondary)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    pub fn success(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Success)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    pub fn danger(label: impl Into<String>, custom_id: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Danger)
            .with_label(label)
            .with_custom_id(custom_id)
    }

    pub fn link(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::new(ButtonStyle::Link).with_label(label).with_url(url)
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_emoji(mut self, emoji: impl Into<String>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }

    pub fn with_custom_id(mut self, custom_id: impl Into<String>) -> Self {
        self.custom_id = Some(custom_id.into());
        self
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// A row of buttons (up to 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRow {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "component_kind")]
    pub kind: String,
    pub components: Vec<Button>,
}

impl ActionRow {
    pub fn new(buttons: impl IntoIterator<Item = Button>) -> Self {
        Self {
            kind: "row".to_string(),
            components: buttons.into_iter().collect(),
        }
    }

    pub fn add_button(&mut self, button: Button) {
        self.components.push(button);
    }
}

/// Generic message component (ActionRow or Components V2 LayoutComponent).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Component {
    Row(ActionRow),
    V2(LayoutComponent),
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
pub struct MediaUrl {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileUrl {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub media: MediaUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spoiler: bool,
}

impl MediaItem {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            media: MediaUrl { url: url.into() },
            description: None,
            spoiler: false,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SectionAccessory {
    Button(Button),
    Thumbnail(ThumbnailComponent),
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
pub struct ThumbnailComponent {
    #[serde(rename = "type")]
    pub kind: u8, // 11
    pub media: MediaUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spoiler: bool,
}

impl ThumbnailComponent {
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
pub struct TextDisplay {
    #[serde(rename = "type")]
    pub kind: u8, // 10
    pub content: String,
}

impl TextDisplay {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            kind: 10,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    #[serde(rename = "type")]
    pub kind: u8, // 9
    pub components: Vec<TextDisplay>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessory: Option<SectionAccessory>,
}

impl Section {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            kind: 9,
            components: vec![TextDisplay::new(text)],
            accessory: None,
        }
    }

    pub fn add_text(mut self, text: impl Into<String>) -> Self {
        self.components.push(TextDisplay::new(text));
        self
    }

    pub fn accessory(mut self, accessory: impl Into<SectionAccessory>) -> Self {
        self.accessory = Some(accessory.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaGallery {
    #[serde(rename = "type")]
    pub kind: u8, // 12
    pub items: Vec<MediaItem>,
}

impl MediaGallery {
    pub fn new(items: impl IntoIterator<Item = MediaItem>) -> Self {
        Self {
            kind: 12,
            items: items.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Separator {
    #[serde(rename = "type")]
    pub kind: u8, // 14
    pub divider: bool,
    pub spacing: u8, // 1 (small) or 2 (large)
}

impl Separator {
    pub fn divider() -> Self {
        Self {
            kind: 14,
            divider: true,
            spacing: 1,
        }
    }

    pub fn space(large: bool) -> Self {
        Self {
            kind: 14,
            divider: false,
            spacing: if large { 2 } else { 1 },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDisplay {
    #[serde(rename = "type")]
    pub kind: u8, // 13
    pub file: FileUrl,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spoiler: bool,
}

impl FileDisplay {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            kind: 13,
            file: FileUrl { url: url.into() },
            spoiler: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Container {
    #[serde(rename = "type")]
    pub kind: u8, // 17
    pub components: Vec<LayoutComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spoiler: bool,
}

impl Container {
    pub fn new() -> Self {
        Self {
            kind: 17,
            components: Vec::new(),
            accent_color: None,
            spoiler: false,
        }
    }

    pub fn accent(mut self, color: u32) -> Self {
        self.accent_color = Some(color);
        self
    }

    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }

    #[allow(clippy::should_implement_trait)]
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
pub enum LayoutComponent {
    Container(Container),
    Section(Section),
    Text(TextDisplay),
    Thumbnail(ThumbnailComponent),
    Gallery(MediaGallery),
    Separator(Separator),
    File(FileDisplay),
    Row(ActionRow),
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
        if v["type"] == "row" || v["type"] == 1 {
            serde_json::from_value(v)
                .map(Self::Row)
                .map_err(serde::de::Error::custom)
        } else {
            serde_json::from_value(v)
                .map(Self::V2)
                .map_err(serde::de::Error::custom)
        }
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
