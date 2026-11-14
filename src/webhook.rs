//! Outbound channel webhooks and authenticated inbound event webhooks.
use crate::{CreateMessage, Error, Event, Message, Result};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;

/// Channel-webhook payload (distinct from a bot message).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::webhook::WebhookMessage;
/// let body = WebhookMessage { content: Some("Build complete".into()), username: Some("Build service".into()), ..Default::default() };
/// assert_eq!(serde_json::to_value(body)?["content"], "Build complete");
/// # Ok::<(), serde_json::Error>(())
/// ```
pub struct WebhookMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional channel-webhook text; validate the complete payload before sending.
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional per-message display name override, subject to platform rules.
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Optional per-message avatar URL override.
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Embed cards for this webhook post.
    pub embeds: Vec<crate::Embed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Platform mention-control object; keep application mention policy explicit.
    pub allowed_mentions: Option<serde_json::Value>,
    /// Discord-shaped link buttons; interactive callbacks are not supported by channel webhooks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<serde_json::Value>,
}

/// Public channel-webhook metadata. Credential fields returned by the server
/// are deliberately ignored; this value is safe to inspect with `Debug`.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// let info: rappin::webhook::WebhookInfo = serde_json::from_value(serde_json::json!({"id":"example-webhook","name":"Build service","token":"discarded-example-credential"}))?;
/// assert_eq!(info.name.as_deref(), Some("Build service"));
/// assert!(!format!("{info:?}").contains("discarded-example-credential"));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub struct WebhookInfo {
    /// Webhook ID as supplied by the public webhook endpoint.
    pub id: String,
    #[serde(default)]
    /// Optional webhook display name.
    pub name: Option<String>,
    #[serde(default)]
    /// Destination channel, when included.
    pub channel_id: Option<uuid::Uuid>,
    #[serde(default)]
    /// Compatibility guild/server ID, when included.
    pub guild_id: Option<uuid::Uuid>,
    #[serde(default)]
    /// Destination server, when included.
    pub server_id: Option<uuid::Uuid>,
    #[serde(default)]
    /// Optional webhook avatar metadata.
    pub avatar: Option<String>,
    #[serde(rename = "type", default)]
    /// Numeric webhook type, when included.
    pub kind: Option<u8>,
}
#[derive(Clone)]
/// Separate credential-URL channel-webhook client; it does not use a bot token or participate in bot event intake.
///
/// # Example
///
/// ```rust
/// use rappin::webhook::WebhookClient;
/// fn configured(credential_url: String) -> WebhookClient {
///     // Load the URL from protected configuration; its path contains a credential.
///     WebhookClient::new(credential_url)
/// }
/// ```
pub struct WebhookClient {
    url: String,
    http: reqwest::Client,
}
impl WebhookClient {
    /// Configure a credential-bearing webhook URL; no request occurs until a method is awaited, and redirects are disabled.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::webhook::WebhookClient;
    /// fn configured(credential_url: String) -> WebhookClient {
    ///     // Load the URL from protected configuration; its path contains a credential.
    ///     WebhookClient::new(credential_url)
    /// }
    /// ```
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client"),
        }
    }
    fn request_url(&self, wait: Option<bool>) -> Result<url::Url> {
        let mut url =
            url::Url::parse(&self.url).map_err(|_| Error::Config("invalid webhook URL".into()))?;
        let mut destination = url.clone();
        destination.set_query(None);
        crate::security::endpoint(destination.as_str(), false)?;
        let query: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| k != "wait")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.set_query(None);
        if !query.is_empty() || wait.is_some() {
            let mut pairs = url.query_pairs_mut();
            pairs.extend_pairs(query);
            if let Some(wait) = wait {
                pairs.append_pair("wait", if wait { "true" } else { "false" });
            }
        }
        Ok(url)
    }

    /// Inspect a webhook using its credential-bearing URL. This never posts a
    /// message. Returned metadata excludes the webhook token.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{webhook::WebhookClient, Result};
    /// async fn use_webhook(webhook: &WebhookClient) -> Result<()> {
    ///     let receipt = webhook.get_info().await?;
    ///     // Reconcile ambiguous sends; this helper never retries a POST automatically.
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_info(&self) -> Result<WebhookInfo> {
        let response = self
            .http
            .get(self.request_url(None)?)
            .send()
            .await
            .map_err(|error| Error::Http(error.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Api {
                status: status.as_u16(),
                message: "webhook inspection failed".into(),
                retry_after: response
                    .headers()
                    .get("Retry-After")
                    .and_then(|header| header.to_str().ok())
                    .and_then(crate::rate_limits::retry_after)
                    .map(|delay| delay.as_secs_f64()),
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| Error::Http(error.without_url()))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Error::Config("invalid webhook metadata response".into()))
    }

    async fn post<T: Serialize>(&self, message: &T, wait: bool) -> Result<Option<Message>> {
        let url = self.request_url(Some(wait))?;
        let mut value = serde_json::to_value(message)?;
        crate::utils::strip_proxy_metadata(&mut value);
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
            .map_err(|e| Error::Http(e.without_url()))?;
        let status = res.status();
        let header = res
            .headers()
            .get("Retry-After")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<f64>().ok());
        let bytes = res.bytes().await?;
        if !status.is_success() {
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            return Err(Error::Api {
                status: status.as_u16(),
                message: "webhook request failed".into(),
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{webhook::WebhookClient, Result};
    /// async fn use_webhook(webhook: &WebhookClient) -> Result<()> {
    ///     let receipt = webhook.send(&rappin::CreateMessage::new("Build complete")).await?;
    ///     // Reconcile ambiguous sends; this helper never retries a POST automatically.
    ///     Ok(())
    /// }
    /// ```
    pub async fn send(&self, message: &CreateMessage) -> Result<Message> {
        self.post(message, true).await?.ok_or_else(|| {
            Error::Config(
                "webhook accepted the message but ignored wait=true; do not resend".into(),
            )
        })
    }
    /// Fire-and-forget mode explicitly accepts successful 204 responses.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{webhook::WebhookClient, Result};
    /// async fn use_webhook(webhook: &WebhookClient) -> Result<()> {
    ///     let receipt = webhook.send_no_wait(&rappin::webhook::WebhookMessage { content: Some("Build complete".into()), ..Default::default() }).await?;
    ///     // Reconcile ambiguous sends; this helper never retries a POST automatically.
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_no_wait(&self, message: &WebhookMessage) -> Result<()> {
        self.post(message, false).await?;
        Ok(())
    }
    /// Post with wait=true, returning None if the platform accepts without a message receipt; do not resend automatically.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{webhook::WebhookClient, Result};
    /// async fn use_webhook(webhook: &WebhookClient) -> Result<()> {
    ///     let receipt = webhook.send_with(&rappin::webhook::WebhookMessage { content: Some("Build complete".into()), ..Default::default() }).await?;
    ///     // Reconcile ambiguous sends; this helper never retries a POST automatically.
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_with(&self, message: &WebhookMessage) -> Result<Option<Message>> {
        self.post(message, true).await
    }
    /// Send text with wait=true and require a message receipt.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{webhook::WebhookClient, Result};
    /// async fn use_webhook(webhook: &WebhookClient) -> Result<()> {
    ///     let receipt = webhook.send_content("Build complete").await?;
    ///     // Reconcile ambiguous sends; this helper never retries a POST automatically.
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_content(&self, content: impl Into<String>) -> Result<Message> {
        self.send(&CreateMessage::new(content)).await
    }
    /// Send one embed with wait=true and require a message receipt.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{webhook::WebhookClient, Result};
    /// async fn use_webhook(webhook: &WebhookClient) -> Result<()> {
    ///     let receipt = webhook.send_embed(rappin::Embed::new().title("Build complete")).await?;
    ///     // Reconcile ambiguous sends; this helper never retries a POST automatically.
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_embed(&self, embed: crate::Embed) -> Result<Message> {
        self.send(&CreateMessage::new("").embed(embed)).await
    }
}
/// Checks HMAC against the exact raw body before parsing. Signature comparison is constant-time.
/// The caller must also compare X-Yapper-Bot with its configured bot ID.
/// # Example
///
/// ```rust
/// use hmac::{Hmac, Mac, KeyInit};
/// use sha2::Sha256;
/// use rappin::webhook::{verify_bot_event, verify_event};
/// let secret = "unused-example-webhook-secret";
/// let body = br#"{"id":42,"type":"test","data":{}}"#;
/// let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
/// mac.update(body);
/// let digest = mac.finalize().into_bytes();
/// let signature = format!("sha256={}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
/// let event = verify_event(secret, &signature, body)?;
/// assert_eq!(event.id, 42);
/// let bot = uuid::Uuid::from_u128(1);
/// let verified = verify_bot_event(secret, bot, &bot.to_string(), &signature, body)?;
/// assert_eq!(verified.bot_id(), bot);
/// assert_eq!(verified.event().id, 42);
/// // Real receivers must verify the exact raw bytes and compare the actual X-Yapper-Bot header.
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn verify_event(secret: &str, signature: &str, body: &[u8]) -> Result<Event> {
    let hex = signature
        .strip_prefix("sha256=")
        .ok_or_else(|| Error::Config("invalid webhook signature".into()))?;
    if hex.len() != 64 {
        return Err(Error::Config("invalid webhook signature".into()));
    }
    let mut digest = [0u8; 32];
    for (i, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        digest[i] = std::str::from_utf8(chunk)
            .ok()
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .ok_or_else(|| Error::Config("invalid webhook signature".into()))?;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| Error::Config("invalid webhook secret".into()))?;
    mac.update(body);
    mac.verify_slice(&digest)
        .map_err(|_| Error::Config("invalid webhook signature".into()))?;
    Ok(serde_json::from_slice(body)?)
}

/// An event whose raw-body signature and authenticated bot header were checked.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use hmac::{Hmac, Mac, KeyInit};
/// use sha2::Sha256;
/// use rappin::webhook::{verify_bot_event, verify_event};
/// let secret = "unused-example-webhook-secret";
/// let body = br#"{"id":42,"type":"test","data":{}}"#;
/// let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
/// mac.update(body);
/// let digest = mac.finalize().into_bytes();
/// let signature = format!("sha256={}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
/// let event = verify_event(secret, &signature, body)?;
/// assert_eq!(event.id, 42);
/// let bot = uuid::Uuid::from_u128(1);
/// let verified = verify_bot_event(secret, bot, &bot.to_string(), &signature, body)?;
/// assert_eq!(verified.bot_id(), bot);
/// assert_eq!(verified.event().id, 42);
/// // Real receivers must verify the exact raw bytes and compare the actual X-Yapper-Bot header.
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct VerifiedEvent {
    pub(crate) bot_id: uuid::Uuid,
    pub(crate) event: Event,
}
impl VerifiedEvent {
    /// Authenticated bot identity established by signature/header verification.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hmac::{Hmac, Mac, KeyInit};
    /// use sha2::Sha256;
    /// use rappin::webhook::{verify_bot_event, verify_event};
    /// let secret = "unused-example-webhook-secret";
    /// let body = br#"{"id":42,"type":"test","data":{}}"#;
    /// let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    /// mac.update(body);
    /// let digest = mac.finalize().into_bytes();
    /// let signature = format!("sha256={}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    /// let event = verify_event(secret, &signature, body)?;
    /// assert_eq!(event.id, 42);
    /// let bot = uuid::Uuid::from_u128(1);
    /// let verified = verify_bot_event(secret, bot, &bot.to_string(), &signature, body)?;
    /// assert_eq!(verified.bot_id(), bot);
    /// assert_eq!(verified.event().id, 42);
    /// // Real receivers must verify the exact raw bytes and compare the actual X-Yapper-Bot header.
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bot_id(&self) -> uuid::Uuid {
        self.bot_id
    }
    /// Borrow the authenticated original event without altering identity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hmac::{Hmac, Mac, KeyInit};
    /// use sha2::Sha256;
    /// use rappin::webhook::{verify_bot_event, verify_event};
    /// let secret = "unused-example-webhook-secret";
    /// let body = br#"{"id":42,"type":"test","data":{}}"#;
    /// let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    /// mac.update(body);
    /// let digest = mac.finalize().into_bytes();
    /// let signature = format!("sha256={}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    /// let event = verify_event(secret, &signature, body)?;
    /// assert_eq!(event.id, 42);
    /// let bot = uuid::Uuid::from_u128(1);
    /// let verified = verify_bot_event(secret, bot, &bot.to_string(), &signature, body)?;
    /// assert_eq!(verified.bot_id(), bot);
    /// assert_eq!(verified.event().id, 42);
    /// // Real receivers must verify the exact raw bytes and compare the actual X-Yapper-Bot header.
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn event(&self) -> &Event {
        &self.event
    }
}
/// Verify both signature and identity before admitting to shared event ingress.
/// # Example
///
/// ```rust
/// use hmac::{Hmac, Mac, KeyInit};
/// use sha2::Sha256;
/// use rappin::webhook::{verify_bot_event, verify_event};
/// let secret = "unused-example-webhook-secret";
/// let body = br#"{"id":42,"type":"test","data":{}}"#;
/// let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
/// mac.update(body);
/// let digest = mac.finalize().into_bytes();
/// let signature = format!("sha256={}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
/// let event = verify_event(secret, &signature, body)?;
/// assert_eq!(event.id, 42);
/// let bot = uuid::Uuid::from_u128(1);
/// let verified = verify_bot_event(secret, bot, &bot.to_string(), &signature, body)?;
/// assert_eq!(verified.bot_id(), bot);
/// assert_eq!(verified.event().id, 42);
/// // Real receivers must verify the exact raw bytes and compare the actual X-Yapper-Bot header.
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn verify_bot_event(
    secret: &str,
    expected_bot: uuid::Uuid,
    received_bot: &str,
    signature: &str,
    body: &[u8],
) -> Result<VerifiedEvent> {
    if expected_bot.is_nil() || received_bot.parse::<uuid::Uuid>().ok() != Some(expected_bot) {
        return Err(Error::validation(
            "webhook.bot_id",
            "authenticated bot identity does not match",
        ));
    }
    Ok(VerifiedEvent {
        bot_id: expected_bot,
        event: verify_event(secret, signature, body)?,
    })
}
