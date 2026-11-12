//! # Example
//!
//! ```rust
//! let file = rappin::UploadFile::new("report.txt", &b"Hello"[..]);
//! file.validate().unwrap();
//! rappin::validate_uploads(&[file]).unwrap();
//! ```
//! API 1.2 version discovery, custom emojis, bot-owned webhooks and uploads.
use crate::{Result, YappinError};
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// Unauthenticated platform version-discovery response.
///
/// # Example
///
/// ```rust
/// let value: rappin::VersionInfo = serde_json::from_value(serde_json::json!({"api":"1.3.0","build":"example","sdk":{"js":"1.6.0","python":"1.6.0"},"packages":{"npm":"yapin","pypi":"yapin"}})).unwrap();
/// assert_eq!(value.api, "1.3.0");
/// ```
pub struct VersionInfo {
    /// Public API semantic version reported by the responding host.
    pub api: String,
    /// Build identifier reported by the responding host.
    pub build: String,
    /// Official JavaScript and Python SDK versions; this is not the Rappin crate version.
    pub sdk: SdkVersions,
    /// Official npm and PyPI package names.
    pub packages: SdkPackages,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// Versions of the official JavaScript and Python SDKs.
///
/// # Example
///
/// ```rust
/// let value: rappin::SdkVersions = serde_json::from_value(serde_json::json!({"js":"1.6.0","python":"1.6.0"})).unwrap();
/// assert_eq!(value.js, "1.6.0");
/// ```
pub struct SdkVersions {
    /// Official JavaScript SDK version.
    pub js: String,
    /// Official Python SDK version.
    pub python: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// Names of the official platform SDK packages.
///
/// # Example
///
/// ```rust
/// let value: rappin::SdkPackages = serde_json::from_value(serde_json::json!({"npm":"yapin","pypi":"yapin"})).unwrap();
/// assert_eq!(value.npm, "yapin");
/// ```
pub struct SdkPackages {
    /// Official npm package name.
    pub npm: String,
    /// Official PyPI package name.
    pub pypi: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Public platform changelog response.
///
/// # Example
///
/// ```rust
/// let value: rappin::ApiChangelog = serde_json::from_value(serde_json::json!({"api":"1.3.0","entries":[]})).unwrap();
/// assert_eq!(value.entries.len(), 0);
/// ```
pub struct ApiChangelog {
    /// Current API version reported with this changelog.
    pub api: String,
    /// Platform changelog entries in server-provided order.
    pub entries: Vec<ChangelogEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// One dated platform changelog entry.
///
/// # Example
///
/// ```rust
/// let value: rappin::ChangelogEntry = serde_json::from_value(serde_json::json!({"date":"2026-11-03","title":"Week of Nov 3","api":"1.3.0","sdk":"1.6.0","changes":[]})).unwrap();
/// assert_eq!(value.changes.len(), 0);
/// ```
pub struct ChangelogEntry {
    /// Date label supplied by the platform for this entry.
    pub date: String,
    /// Human-readable release/week heading.
    pub title: String,
    /// API version associated with the entry.
    pub api: String,
    /// Official platform SDK version associated with the entry.
    pub sdk: String,
    /// Changes described by this entry.
    pub changes: Vec<ApiChange>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// One platform change description with an optional rollout flag.
///
/// # Example
///
/// ```rust
/// let value: rappin::ApiChange = serde_json::from_value(serde_json::json!({"type":"added","text":"A new field"})).unwrap();
/// assert_eq!(value.flag, None);
/// ```
pub struct ApiChange {
    #[serde(rename = "type")]
    /// Platform change category, such as added, changed or fixed.
    pub kind: String,
    /// Human-readable description of the change.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Optional rollout flag associated with the described feature.
    pub flag: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Metadata for a server custom emoji.
///
/// # Example
///
/// ```rust
/// let value: rappin::CustomEmoji = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"wave","animated":false,"path":"wave.png","url":"https://example.com/wave.png","owner_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil(),"created_at":"2026-11-08T00:00:00Z"})).unwrap();
/// assert_eq!(value.animated, false);
/// ```
pub struct CustomEmoji {
    /// Custom emoji identifier.
    pub id: Uuid,
    /// Server emoji name.
    pub name: String,
    /// Whether the asset is animated.
    pub animated: bool,
    /// Stored emoji asset path.
    pub path: String,
    /// Absolute emoji asset URL.
    pub url: String,
    /// User recorded as owning the emoji.
    pub owner_id: Uuid,
    /// Server containing the emoji.
    pub server_id: Uuid,
    /// UTC emoji creation timestamp.
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Server emoji inventory and slot limit.
///
/// # Example
///
/// ```rust
/// let value: rappin::EmojisResponse = serde_json::from_value(serde_json::json!({"emojis":[],"limit":50})).unwrap();
/// assert_eq!(value.limit, 50);
/// ```
pub struct EmojisResponse {
    /// Custom emojis returned for the server.
    pub emojis: Vec<CustomEmoji>,
    /// Server emoji-slot limit reported by the API.
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Emoji creation outcome, including moderation-pending success.
///
/// # Example
///
/// ```rust
/// let value: rappin::EmojiCreateResult = serde_json::from_value(serde_json::json!({"emoji":null,"pending":true})).unwrap();
/// assert_eq!(value.pending, true);
/// ```
pub struct EmojiCreateResult {
    /// Created emoji when immediately available; may be absent pending moderation.
    pub emoji: Option<CustomEmoji>,
    #[serde(default)]
    /// Whether moderation is still pending. Pending is not a failed creation.
    pub pending: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// JSON emoji-creation payload. The decoded image bound is 2 MiB.
///
/// # Example
///
/// ```rust
/// let image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jSnkAAAAASUVORK5CYII=";
/// let body = rappin::CreateEmoji::new("wave", image);
/// body.validate().unwrap();
/// ```
pub struct CreateEmoji {
    /// Requested emoji name; local validation accepts 2 through 32 ASCII letters, digits or underscores.
    pub name: String,
    /// Base64 image bytes or a PNG/GIF/WebP data URL. This SDK currently validates a 2 MiB decoded maximum.
    pub image: String,
}
impl CreateEmoji {
    /// Create a JSON emoji payload without sending it or performing moderation.
    ///
    /// # Example
    ///
    /// ```rust
    /// let image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jSnkAAAAASUVORK5CYII=";
    /// let body = rappin::CreateEmoji::new("wave", image);
    /// body.validate().unwrap();
    /// ```
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
        }
    }
    /// Check the name, base64/data-URL form and current 2 MiB decoded SDK bound. This does not perform server image scanning.
    ///
    /// # Example
    ///
    /// ```rust
    /// let image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jSnkAAAAASUVORK5CYII=";
    /// let body = rappin::CreateEmoji::new("wave", image);
    /// body.validate().unwrap();
    /// assert!(rappin::CreateEmoji::new("bad name", "!").validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        validate_emoji_name(&self.name)?;
        let image = if self.image.starts_with("data:") {
            let (header, body) = self
                .image
                .split_once(',')
                .ok_or_else(|| YappinError::validation("image", "invalid image data URL"))?;
            if !matches!(
                header,
                "data:image/png;base64" | "data:image/gif;base64" | "data:image/webp;base64"
            ) {
                return Err(YappinError::validation(
                    "image",
                    "requires PNG, GIF or WebP base64",
                ));
            }
            body
        } else {
            &self.image
        };
        if image.len() > 2_796_204 {
            return Err(YappinError::validation(
                "image",
                "maximum decoded emoji image is 2 MiB",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image)
            .map_err(|_| YappinError::validation("image", "invalid base64"))?;
        if bytes.is_empty() || bytes.len() > 2_097_152 {
            return Err(YappinError::validation(
                "image",
                "requires 1 through 2097152 decoded bytes",
            ));
        }
        Ok(())
    }
}
/// Validate 2 through 32 ASCII letters, digits or underscores. The length is measured in bytes.
///
/// # Example
///
/// ```rust
/// rappin::validate_emoji_name("party_parrot").unwrap();
/// assert!(rappin::validate_emoji_name("party parrot").is_err());
/// ```
pub fn validate_emoji_name(name: &str) -> Result<()> {
    if !(2..=32).contains(&name.len())
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(YappinError::validation(
            "name",
            "emoji names require 2 to 32 letters, digits or _",
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Metadata for a bot-owned channel webhook.
///
/// # Example
///
/// ```rust
/// let value: rappin::BotWebhook = serde_json::from_value(serde_json::json!({"id":uuid::Uuid::nil(),"name":"Notifier","channel_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil(),"created_at":"2026-11-08T00:00:00Z"})).unwrap();
/// assert_eq!(value.posts, 0);
/// ```
pub struct BotWebhook {
    /// Webhook identifier.
    pub id: Uuid,
    /// Webhook display name.
    pub name: String,
    /// Channel receiving this webhook's posts.
    pub channel_id: Uuid,
    /// Server containing the webhook channel.
    pub server_id: Uuid,
    #[serde(default)]
    /// Optional stored avatar path.
    pub avatar_path: Option<String>,
    #[serde(default)]
    /// Creating actor, when returned.
    pub created_by: Option<Uuid>,
    #[serde(default)]
    /// Owning bot, when returned.
    pub bot_id: Option<Uuid>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    /// UTC time of the last use, when supplied.
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    /// Number of posts reported by the API.
    pub posts: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Collection of bot-owned webhook metadata.
///
/// # Example
///
/// ```rust
/// let value: rappin::BotWebhooksResponse = serde_json::from_value(serde_json::json!({"webhooks":[]})).unwrap();
/// assert_eq!(value.webhooks.len(), 0);
/// ```
pub struct BotWebhooksResponse {
    /// Bot-owned webhooks returned by the selected endpoint.
    pub webhooks: Vec<BotWebhook>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// Payload for creating a bot-owned channel webhook.
///
/// # Example
///
/// ```rust
/// let body = rappin::CreateBotWebhook::new("Notifier");
/// body.validate().unwrap();
/// ```
pub struct CreateBotWebhook {
    /// Requested webhook display name; local validation permits 1 through 80 characters and rejects whitespace-only names.
    pub name: String,
}
impl CreateBotWebhook {
    /// Create a webhook-name payload; authorization remains a platform check.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateBotWebhook::new("Notifier");
    /// body.validate().unwrap();
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
    /// Check the local name bounds. Reserved names and permissions remain server checks.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateBotWebhook::new("Notifier");
    /// body.validate().unwrap();
    /// assert!(rappin::CreateBotWebhook::new(" ").validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return Err(YappinError::validation(
                "name",
                "webhook name requires 1 to 80 characters",
            ));
        }
        // Reserved/staff names and channel permissions remain authoritative API checks.
        Ok(())
    }
}

/// Reusable immutable upload data. Never reads a filesystem path implicitly.
#[derive(Clone)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let file = rappin::UploadFile::new("report.txt", &b"Hello"[..]);
/// file.validate().unwrap();
/// ```
pub struct UploadFile {
    /// Filename with a supported extension; paths and control characters are rejected.
    pub name: String,
    /// Immutable in-memory file bytes; the SDK does not open a filesystem path from this value.
    pub data: Bytes,
}
impl std::fmt::Debug for UploadFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadFile")
            .field("name", &self.name)
            .field("bytes", &self.data.len())
            .finish()
    }
}
impl UploadFile {
    /// Wrap a filename and immutable bytes without reading any local path.
    ///
    /// # Example
    ///
    /// ```rust
    /// let file = rappin::UploadFile::new("report.txt", &b"Hello"[..]);
    /// file.validate().unwrap();
    /// ```
    pub fn new(name: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
        }
    }
    /// Check filename, extension and 1-byte through 10-MiB per-file size bounds. This does not scan or publish the file.
    ///
    /// # Example
    ///
    /// ```rust
    /// let file = rappin::UploadFile::new("report.txt", &b"Hello"[..]);
    /// file.validate().unwrap();
    /// assert!(rappin::UploadFile::new("../report.txt", &b"Hello"[..]).validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.name.len() > 255
            || self
                .name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err(YappinError::validation(
                "file.name",
                "use a filename, at most 255 bytes, without paths or control characters",
            ));
        }
        if self.data.is_empty() || self.data.len() > 10 * 1024 * 1024 {
            return Err(YappinError::validation(
                "file.data",
                "requires 1 byte through 10 MiB per file",
            ));
        }
        self.mime()?;
        Ok(())
    }
    pub(crate) fn mime(&self) -> Result<&'static str> {
        match self
            .name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => Ok("image/png"),
            "jpg" | "jpeg" => Ok("image/jpeg"),
            "gif" => Ok("image/gif"),
            "webp" => Ok("image/webp"),
            "mp4" => Ok("video/mp4"),
            "webm" => Ok("video/webm"),
            "mov" => Ok("video/quicktime"),
            "mp3" => Ok("audio/mpeg"),
            "ogg" => Ok("audio/ogg"),
            "wav" => Ok("audio/wav"),
            "m4a" => Ok("audio/mp4"),
            "txt" => Ok("text/plain"),
            "pdf" => Ok("application/pdf"),
            "zip" => Ok("application/zip"),
            _ => Err(YappinError::validation(
                "file.name",
                "unsupported attachment extension",
            )),
        }
    }
}
/// Validate one through four files and a combined size of at most 12 MiB. Each file also satisfies its own filename, extension and 10-MiB bound.
///
/// # Example
///
/// ```rust
/// let file = rappin::UploadFile::new("report.txt", &b"Hello"[..]);
/// file.validate().unwrap();
/// rappin::validate_uploads(&[file]).unwrap();
/// assert!(rappin::validate_uploads(&[]).is_err());
/// ```
pub fn validate_uploads(files: &[UploadFile]) -> Result<()> {
    if files.is_empty() || files.len() > 4 {
        return Err(YappinError::validation(
            "files",
            "requires 1 through 4 files",
        ));
    }
    let mut total = 0;
    for file in files {
        file.validate()?;
        total += file.data.len();
    }
    if total > 12 * 1024 * 1024 {
        return Err(YappinError::validation(
            "files",
            "maximum combined upload is 12 MiB",
        ));
    }
    Ok(())
}
/// Complete or partial HTTP-success upload response. Partial success must be
/// inspected even when the server answered 201; into_result retains receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// let result: rappin::UploadedMessages = serde_json::from_value(serde_json::json!({"message":rappin::Message::default()})).unwrap();
/// assert_eq!(result.into_result().unwrap().len(), 1);
/// ```
pub struct UploadedMessages {
    /// Primary confirmed message receipt.
    pub message: crate::Message,
    #[serde(default)]
    /// Per-file confirmed receipts, when supplied. Empty lists fall back to the primary receipt.
    pub messages: Vec<crate::Message>,
    #[serde(default)]
    /// Reported partial-upload error; inspect into_result before treating the workflow as complete.
    pub error: Option<String>,
}
impl UploadedMessages {
    /// Return confirmed messages, or a PartialWrite error retaining receipts when the server reports incomplete upload processing. This does not retry the upload.
    ///
    /// # Example
    ///
    /// ```rust
    /// let result: rappin::UploadedMessages = serde_json::from_value(serde_json::json!({"message":rappin::Message::default()})).unwrap();
    /// assert_eq!(result.into_result().unwrap().len(), 1);
    /// ```
    pub fn into_result(self) -> Result<Vec<crate::Message>> {
        self.into_result_for(0)
    }
    /// Also reject a missing/mismatched per-file receipt list. Zero means unknown count.
    /// # Example
    ///
    /// ```rust
    /// let result: rappin::UploadedMessages = serde_json::from_value(serde_json::json!({"message":rappin::Message::default()})).unwrap();
    /// let error = result.into_result_for(2).unwrap_err();
    /// assert!(matches!(error, rappin::YappinError::PartialWrite { .. }));
    /// ```
    pub fn into_result_for(mut self, expected: usize) -> Result<Vec<crate::Message>> {
        let confirmed = if self.messages.is_empty() {
            1
        } else {
            self.messages.len()
        };
        if expected > 0 && confirmed != expected && self.error.is_none() {
            self.error = Some("API did not confirm exactly one message per file".into());
        }
        let messages = if self.messages.is_empty() {
            vec![self.message]
        } else {
            self.messages
        };
        if let Some(message) = self.error {
            let failed_step = messages.len() + 1;
            let completed = messages
                .into_iter()
                .map(|message| serde_json::json!({"message":message}))
                .collect();
            return Err(YappinError::PartialWrite {
                completed,
                failed_step,
                source: Box::new(YappinError::Remote(crate::RemoteError {
                    kind: "upload_incomplete".into(),
                    message,
                    status: None,
                    retry_after: None,
                    details: [
                        ("outcome".into(), "unknown".into()),
                        ("retry_safety".into(), "do_not_replay_workflow".into()),
                    ]
                    .into(),
                })),
            });
        }
        Ok(messages)
    }
}
