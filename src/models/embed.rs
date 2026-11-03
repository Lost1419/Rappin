use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Color representation for an Embed (either a 24-bit integer 0x000000 - 0xFFFFFF or hex string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum EmbedColor {
    Int(u32),
}

impl EmbedColor {
    pub const RED: u32 = 0xED4245;
    pub const GREEN: u32 = 0x57F287;
    pub const BLUE: u32 = 0x5865F2;
    pub const YELLOW: u32 = 0xFEE75C;
    pub const PURPLE: u32 = 0x9B59B6;
    pub const ORANGE: u32 = 0xE67E22;
    pub const GREY: u32 = 0x95A5A6;

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
pub struct EmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

impl EmbedField {
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
pub struct EmbedAuthor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_icon_url: Option<String>,
}

impl EmbedAuthor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: None,
            icon_url: None,
            proxy_icon_url: None,
        }
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }
}

/// Footer details in an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmbedFooter {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_icon_url: Option<String>,
}

impl EmbedFooter {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon_url: None,
            proxy_icon_url: None,
        }
    }

    pub fn icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }
}

/// Image or thumbnail asset in an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmbedMedia {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
}

impl EmbedMedia {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            proxy_url: None,
        }
    }
}

/// A rich embed card for messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Embed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<EmbedColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<EmbedField>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<EmbedMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<EmbedMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

impl Embed {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn color(mut self, color: impl Into<EmbedColor>) -> Self {
        self.color = Some(color.into());
        self
    }

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

    pub fn author(mut self, author: EmbedAuthor) -> Self {
        self.author = Some(author);
        self
    }

    pub fn footer(mut self, footer: EmbedFooter) -> Self {
        self.footer = Some(footer);
        self
    }

    pub fn thumbnail(mut self, url: impl Into<String>) -> Self {
        self.thumbnail = Some(EmbedMedia::new(url));
        self
    }

    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(EmbedMedia::new(url));
        self
    }

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
