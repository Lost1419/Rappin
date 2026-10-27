//! Outbound channel webhooks and authenticated inbound event webhooks.
use crate::{CreateMessage, Event, Message, Result, YappinError};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;

/// Channel-webhook payload (distinct from a bot message).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<crate::Embed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_mentions: Option<serde_json::Value>,
    /// Discord-shaped link buttons; interactive callbacks are not supported by channel webhooks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<serde_json::Value>,
}
#[derive(Clone)]
pub struct WebhookClient {
    url: String,
    http: reqwest::Client,
}
impl WebhookClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("HTTP client"),
        }
    }
    async fn post<T: Serialize>(&self, message: &T, wait: bool) -> Result<Option<Message>> {
        let mut url = url::Url::parse(&self.url)
            .map_err(|_| YappinError::Config("invalid webhook URL".into()))?;
        let query: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| k != "wait")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(query)
            .append_pair("wait", if wait { "true" } else { "false" });
        let mut value = serde_json::to_value(message)?;
        crate::validation::validate_webhook(&value)?;
        // Channel webhooks use Discord's numeric link-button representation.
        if let Some(rows) = value["components"].as_array_mut() {
            for row in rows {
                row["type"] = 1.into();
                if let Some(buttons) = row["components"].as_array_mut() {
                    for button in buttons {
                        button["type"] = 2.into();
                        button["style"] = 5.into();
                        if let Some(emoji) = button["emoji"].as_str().map(String::from) {
                            button["emoji"] = serde_json::json!({"name":emoji});
                        }
                    }
                }
            }
        }
        let res = self
            .http
            .post(url)
            .json(&value)
            .send()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        let status = res.status();
        let header = res
            .headers()
            .get("Retry-After")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<f64>().ok());
        let bytes = res.bytes().await?;
        if !status.is_success() {
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            return Err(YappinError::Api {
                status: status.as_u16(),
                message: v["message"]
                    .as_str()
                    .unwrap_or("webhook request failed")
                    .into(),
                retry_after: header
                    .or(v["retry_after"].as_f64())
                    .filter(|v| v.is_finite() && *v >= 0.0),
            });
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        let v: serde_json::Value = serde_json::from_slice(&bytes)?;
        Ok(Some(serde_json::from_value(
            v.get("message").unwrap_or(&v).clone(),
        )?))
    }
    /// Requests wait=true so a successful send returns the posted message.
    pub async fn send(&self, message: &CreateMessage) -> Result<Message> {
        self.post(message, true).await?.ok_or_else(|| {
            YappinError::Config(
                "webhook accepted the message but ignored wait=true; do not resend".into(),
            )
        })
    }
    /// Fire-and-forget mode explicitly accepts successful 204 responses.
    pub async fn send_no_wait(&self, message: &WebhookMessage) -> Result<()> {
        self.post(message, false).await?;
        Ok(())
    }
    pub async fn send_with(&self, message: &WebhookMessage) -> Result<Option<Message>> {
        self.post(message, true).await
    }
    pub async fn send_content(&self, content: impl Into<String>) -> Result<Message> {
        self.send(&CreateMessage::new(content)).await
    }
    pub async fn send_embed(&self, embed: crate::Embed) -> Result<Message> {
        self.send(&CreateMessage::new("").embed(embed)).await
    }
}
/// Checks HMAC against the exact raw body before parsing. Signature comparison is constant-time.
/// The caller must also compare X-Yapper-Bot with its configured bot ID.
pub fn verify_event(secret: &str, signature: &str, body: &[u8]) -> Result<Event> {
    let hex = signature
        .strip_prefix("sha256=")
        .ok_or_else(|| YappinError::Config("invalid webhook signature".into()))?;
    if hex.len() != 64 {
        return Err(YappinError::Config("invalid webhook signature".into()));
    }
    let mut digest = [0u8; 32];
    for (i, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        digest[i] = std::str::from_utf8(chunk)
            .ok()
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .ok_or_else(|| YappinError::Config("invalid webhook signature".into()))?;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| YappinError::Config("invalid webhook secret".into()))?;
    mac.update(body);
    mac.verify_slice(&digest)
        .map_err(|_| YappinError::Config("invalid webhook signature".into()))?;
    Ok(serde_json::from_slice(body)?)
}
