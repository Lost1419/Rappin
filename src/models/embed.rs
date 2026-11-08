//! Embed cards, fields and response image metadata with outbound validation.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Color representation for an Embed (either a 24-bit integer 0x000000 - 0xFFFFFF or hex string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
/// # Example
///
/// ```rust
/// use rappin::EmbedColor;
/// let color = EmbedColor::from_hex("#5865F2").unwrap();
/// assert_eq!(serde_json::to_value(color).unwrap(), 0x5865F2);
/// ```
pub enum EmbedColor {
    /// Packed red/green/blue value. Values above `0xFFFFFF` are rejected when decoding or validating a message.
    Int(#[doc = "Packed numeric color value."] u32),
}

impl EmbedColor {
    /// Packed 24-bit red accent (`0xED4245`).
    pub const RED: u32 = 0xED4245;
    /// Packed 24-bit green accent (`0x57F287`).
    pub const GREEN: u32 = 0x57F287;
    /// Packed 24-bit blue accent (`0x5865F2`).
    pub const BLUE: u32 = 0x5865F2;
    /// Packed 24-bit yellow accent (`0xFEE75C`).
    pub const YELLOW: u32 = 0xFEE75C;
    /// Packed 24-bit purple accent (`0x9B59B6`).
    pub const PURPLE: u32 = 0x9B59B6;
    /// Packed 24-bit orange accent (`0xE67E22`).
    pub const ORANGE: u32 = 0xE67E22;
    /// Packed 24-bit grey accent (`0x95A5A6`).
    pub const GREY: u32 = 0x95A5A6;

    /// Parse a hexadecimal color, with or without a leading `#`. Returns `None` for invalid input or values outside 24 bits.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedColor;
    /// let color = EmbedColor::from_hex("#5865F2").unwrap();
    /// assert_eq!(serde_json::to_value(color).unwrap(), 0x5865F2);
    /// assert!(EmbedColor::from_hex("#1000000").is_none());
    /// ```
    pub fn from_hex(hex: &str) -> Option<Self> {
        let clean = hex.trim_start_matches('#');
        u32::from_str_radix(clean, 16)
            .ok()
            .filter(|v| *v <= 0xffffff)
            .map(EmbedColor::Int)
    }
}

impl From<u32> for EmbedColor {
    fn from(val: u32) -> Self {
        EmbedColor::Int(val)
    }
}

/// A field within a rich embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::EmbedField;
/// let field = EmbedField::new("Players", "4", true);
/// assert!(field.inline);
/// ```
pub struct EmbedField {
    /// Field heading; message validation limits it to 256 characters.
    pub name: String,
    /// Field body; message validation limits it to 1,024 characters.
    pub value: String,
    #[serde(default)]
    /// Whether the client may arrange this field beside adjacent fields.
    pub inline: bool,
}

impl EmbedField {
    /// Create a named field and select whether it is inline.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedField;
    /// let field = EmbedField::new("Players", "4", true);
    /// assert!(field.inline);
    /// ```
    pub fn new(name: impl Into<String>, value: impl Into<String>, inline: bool) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            inline,
        }
    }
}

/// Author details in an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::EmbedAuthor;
/// let author = EmbedAuthor::new("Game host").url("https://example.com");
/// assert_eq!(author.name, "Game host");
/// ```
pub struct EmbedAuthor {
    /// Display name shown in the author heading.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional link opened from the author name.
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Original author icon URL supplied by the application.
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Server-provided image-proxy metadata; omitted when resending this embed.
    pub proxy_icon_url: Option<String>,
}

impl EmbedAuthor {
    /// Create an author heading without links or icons.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedAuthor;
    /// let author = EmbedAuthor::new("Game host").url("https://example.com");
    /// assert_eq!(author.name, "Game host");
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: None,
            icon_url: None,
            proxy_icon_url: None,
        }
    }

    /// Set the author-name link.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedAuthor;
    /// let author = EmbedAuthor::new("Host").url("https://example.com");
    /// assert_eq!(author.url.as_deref(), Some("https://example.com"));
    /// ```
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the original author icon URL.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedAuthor;
    /// let author = EmbedAuthor::new("Host").icon_url("https://example.com/icon.png");
    /// assert_eq!(author.icon_url.as_deref(), Some("https://example.com/icon.png"));
    /// ```
    pub fn icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }
}

/// Footer details in an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::EmbedFooter;
/// let footer = EmbedFooter::new("Page 1 of 3");
/// assert_eq!(footer.text, "Page 1 of 3");
/// ```
pub struct EmbedFooter {
    /// Footer text below the card body.
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional original footer icon URL.
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Server-provided proxy icon URL; clear it before reusing response data as a request.
    pub proxy_icon_url: Option<String>,
}

impl EmbedFooter {
    /// Create a footer without an icon.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedFooter;
    /// let footer = EmbedFooter::new("Page 1 of 3");
    /// assert_eq!(footer.text, "Page 1 of 3");
    /// ```
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon_url: None,
            proxy_icon_url: None,
        }
    }

    /// Set the original footer icon URL.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedFooter;
    /// let footer = EmbedFooter::new("Host").icon_url("https://example.com/icon.png");
    /// assert_eq!(footer.icon_url.as_deref(), Some("https://example.com/icon.png"));
    /// ```
    pub fn icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }
}

/// Image or thumbnail asset in an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::EmbedMedia;
/// let image = EmbedMedia::new("https://example.com/card.png");
/// assert!(image.proxy_url.is_none());
/// ```
pub struct EmbedMedia {
    /// Original image URL supplied in the message request.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Response-only proxy image URL; it is not an application-supplied image source.
    pub proxy_url: Option<String>,
}

impl EmbedMedia {
    /// Create an image reference without response-only proxy metadata.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::EmbedMedia;
    /// let image = EmbedMedia::new("https://example.com/card.png");
    /// assert!(image.proxy_url.is_none());
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            proxy_url: None,
        }
    }
}

/// A rich embed card for messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{Embed, EmbedColor};
/// let card = Embed::new().title("Results").color(EmbedColor::GREEN).field("Score", "42", true);
/// assert_eq!(card.fields.as_ref().unwrap().len(), 1);
/// ```
pub struct Embed {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional card heading, limited to 256 characters by message validation.
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional card body, limited to 4,096 characters by message validation.
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional link associated with the card heading.
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional 24-bit accent color.
    pub color: Option<EmbedColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional field list; message validation permits at most 25 fields per embed.
    pub fields: Option<Vec<EmbedField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional author heading and icon.
    pub author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional footer text and icon.
    pub footer: Option<EmbedFooter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional thumbnail shown beside the card body.
    pub thumbnail: Option<EmbedMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional full-width image attached to the card.
    pub image: Option<EmbedMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional UTC timestamp attached to the card.
    pub timestamp: Option<DateTime<Utc>>,
}

impl Embed {
    /// Create an empty embed. Add text, fields or media before sending it.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Embed, EmbedColor};
    /// let card = Embed::new().title("Results").color(EmbedColor::GREEN).field("Score", "42", true);
    /// assert_eq!(card.fields.as_ref().unwrap().len(), 1);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the card heading.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().title("Results");
    /// assert_eq!(card.title.as_deref(), Some("Results"));
    /// ```
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the card body.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().description("Round complete");
    /// assert_eq!(card.description.as_deref(), Some("Round complete"));
    /// ```
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the card heading link.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().url("https://example.com/results");
    /// assert_eq!(card.url.as_deref(), Some("https://example.com/results"));
    /// ```
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the accent color from a packed integer or EmbedColor.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().color(0x5865F2);
    /// assert_eq!(card.color, Some(rappin::EmbedColor::Int(0x5865F2)));
    /// ```
    pub fn color(mut self, color: impl Into<EmbedColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Append a named field; existing fields retain their order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().field("Score", "42", true);
    /// assert_eq!(card.fields.as_ref().unwrap()[0].value.as_str(), "42");
    /// ```
    pub fn field(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        inline: bool,
    ) -> Self {
        let field = EmbedField::new(name, value, inline);
        self.fields.get_or_insert_with(Vec::new).push(field);
        self
    }

    /// Replace the author heading.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().author(rappin::EmbedAuthor::new("Host"));
    /// assert_eq!(card.author.as_ref().unwrap().name.as_str(), "Host");
    /// ```
    pub fn author(mut self, author: EmbedAuthor) -> Self {
        self.author = Some(author);
        self
    }

    /// Replace the footer.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().footer(rappin::EmbedFooter::new("Page 1"));
    /// assert_eq!(card.footer.as_ref().unwrap().text.as_str(), "Page 1");
    /// ```
    pub fn footer(mut self, footer: EmbedFooter) -> Self {
        self.footer = Some(footer);
        self
    }

    /// Set the original thumbnail URL.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().thumbnail("https://example.com/thumb.png");
    /// assert_eq!(card.thumbnail.as_ref().unwrap().url.as_str(), "https://example.com/thumb.png");
    /// ```
    pub fn thumbnail(mut self, url: impl Into<String>) -> Self {
        self.thumbnail = Some(EmbedMedia::new(url));
        self
    }

    /// Set the original full-width image URL.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().image("https://example.com/image.png");
    /// assert_eq!(card.image.as_ref().unwrap().url.as_str(), "https://example.com/image.png");
    /// ```
    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(EmbedMedia::new(url));
        self
    }

    /// Attach a UTC timestamp.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Embed;
    /// let card = Embed::new().timestamp(chrono::Utc::now());
    /// assert_eq!(card.timestamp.is_some(), true);
    /// ```
    pub fn timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
}

impl<'de> Deserialize<'de> for EmbedColor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let color = value
            .as_u64()
            .filter(|v| *v <= 0xffffff)
            .map(|v| Self::Int(v as u32))
            .or_else(|| value.as_str().and_then(Self::from_hex));
        color.ok_or_else(|| serde::de::Error::custom("expected a 24-bit number or hex color"))
    }
}

impl Embed {
    pub(crate) fn has_proxy_metadata(&self) -> bool {
        self.image.as_ref().is_some_and(|m| m.proxy_url.is_some())
            || self
                .thumbnail
                .as_ref()
                .is_some_and(|m| m.proxy_url.is_some())
            || self
                .author
                .as_ref()
                .is_some_and(|m| m.proxy_icon_url.is_some())
            || self
                .footer
                .as_ref()
                .is_some_and(|m| m.proxy_icon_url.is_some())
    }
    /// Remove response-only image-proxy URLs while preserving the original image URLs. This lets a fetched embed be reused in a new request.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Embed, EmbedMedia};
    /// let mut card = Embed::new().image("https://example.com/card.png");
    /// card.image.as_mut().unwrap().proxy_url = Some("https://example.com/proxy".into());
    /// card.clear_proxy_metadata();
    /// assert!(card.image.as_ref().unwrap().proxy_url.is_none());
    /// assert_eq!(card.image.as_ref().unwrap().url, "https://example.com/card.png");
    /// ```
    pub fn clear_proxy_metadata(&mut self) {
        if let Some(m) = &mut self.image {
            m.proxy_url = None;
        }
        if let Some(m) = &mut self.thumbnail {
            m.proxy_url = None;
        }
        if let Some(a) = &mut self.author {
            a.proxy_icon_url = None;
        }
        if let Some(f) = &mut self.footer {
            f.proxy_icon_url = None;
        }
    }
}
