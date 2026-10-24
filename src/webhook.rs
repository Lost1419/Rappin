use std::time::Duration;

use reqwest::{header, StatusCode};

use crate::error::{Result, YappinError};
use crate::models::embed::Embed;
use crate::models::message::{CreateMessage, Message};

/// A standalone client to send messages and embeds into a Yapin' channel via Webhooks.
///
/// Webhooks require no bot accounts or long-polling and are ideal for CI/CD pipelines,
/// alerting services, and external integrations.
#[derive(Clone)]
pub struct WebhookClient {
    url: String,
    http: reqwest::Client,
}

impl WebhookClient {
    /// Creates a new WebhookClient with a given webhook URL.
    pub fn new(url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to construct reqwest client");

        Self {
            url: url.into(),
            http,
        }
    }

    /// Sends a message payload through the webhook.
    pub async fn send(&self, message: &CreateMessage) -> Result<Message> {
        let res = self
            .http
            .post(&self.url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::USER_AGENT, "rappin-webhook/0.1.0")
            .json(message)
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(YappinError::Api {
                status: status.as_u16(),
                message: if body.is_empty() {
                    status.to_string()
                } else {
                    body
                },
                retry_after: None,
            });
        }

        // Parse returned message if body contains it
        let bytes = res.bytes().await?;
        if bytes.is_empty() {
            return Err(YappinError::Api {
                status: StatusCode::NO_CONTENT.as_u16(),
                message: "Webhook sent with no returned message body".to_string(),
                retry_after: None,
            });
        }

        #[derive(serde::Deserialize)]
        struct Resp {
            message: Option<Message>,
            #[serde(flatten)]
            direct_message: Option<Message>,
        }

        let parsed: Resp = serde_json::from_slice(&bytes)?;
        parsed
            .message
            .or(parsed.direct_message)
            .ok_or_else(|| YappinError::Config("Webhook returned unrecognized format".to_string()))
    }

    /// Sends simple text content through the webhook.
    pub async fn send_content(&self, content: impl Into<String>) -> Result<Message> {
        self.send(&CreateMessage::new(content)).await
    }

    /// Sends a rich embed card through the webhook.
    pub async fn send_embed(&self, embed: Embed) -> Result<Message> {
        self.send(&CreateMessage::new("").embed(embed)).await
    }
}
