//! API 1.2 version discovery, custom emojis, bot-owned webhooks and uploads.
use crate::{Result, YappinError};
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VersionInfo {
    pub api: String,
    pub build: String,
    pub sdk: SdkVersions,
    pub packages: SdkPackages,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SdkVersions {
    pub js: String,
    pub python: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SdkPackages {
    pub npm: String,
    pub pypi: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApiChangelog {
    pub api: String,
    pub entries: Vec<ChangelogEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChangelogEntry {
    pub date: String,
    pub title: String,
    pub api: String,
    pub sdk: String,
    pub changes: Vec<ApiChange>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ApiChange {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CustomEmoji {
    pub id: Uuid,
    pub name: String,
    pub animated: bool,
    pub path: String,
    pub url: String,
    pub owner_id: Uuid,
    pub server_id: Uuid,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmojisResponse {
    pub emojis: Vec<CustomEmoji>,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmojiCreateResult {
    pub emoji: Option<CustomEmoji>,
    #[serde(default)]
    pub pending: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateEmoji {
    pub name: String,
    pub image: String,
}
impl CreateEmoji {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
        }
    }
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
        if image.len() > 1_398_104 {
            return Err(YappinError::validation(
                "image",
                "maximum decoded emoji image is 1 MiB",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image)
            .map_err(|_| YappinError::validation("image", "invalid base64"))?;
        if bytes.is_empty() || bytes.len() > 1_048_576 {
            return Err(YappinError::validation(
                "image",
                "requires 1 through 1048576 decoded bytes",
            ));
        }
        Ok(())
    }
}
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
pub struct BotWebhook {
    pub id: Uuid,
    pub name: String,
    pub channel_id: Uuid,
    pub server_id: Uuid,
    #[serde(default)]
    pub avatar_path: Option<String>,
    #[serde(default)]
    pub created_by: Option<Uuid>,
    #[serde(default)]
    pub bot_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub posts: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BotWebhooksResponse {
    pub webhooks: Vec<BotWebhook>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateBotWebhook {
    pub name: String,
}
impl CreateBotWebhook {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
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
pub struct UploadFile {
    pub name: String,
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
    pub fn new(name: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
        }
    }
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
pub struct UploadedMessages {
    pub message: crate::Message,
    #[serde(default)]
    pub messages: Vec<crate::Message>,
    #[serde(default)]
    pub error: Option<String>,
}
impl UploadedMessages {
    pub fn into_result(self) -> Result<Vec<crate::Message>> {
        self.into_result_for(0)
    }
    /// Also reject a missing/mismatched per-file receipt list. Zero means unknown count.
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
