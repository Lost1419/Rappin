use super::*;
use crate::ApiExt;
impl Client {
    /// Unauthenticated version/changelog reads using the configured transport and failover.
    pub(crate) async fn public_json(&self, path: &str) -> Result<serde_json::Value> {
        if path != "/version" && !path.starts_with("/changelog?") {
            return Err(YappinError::validation(
                "path",
                "not a public metadata endpoint",
            ));
        }
        let response = self
            .execute_with_retry(&Method::GET, path, |url| {
                self.inner
                    .http
                    .get(url)
                    .header(
                        header::USER_AGENT,
                        concat!("rappin-rs/", env!("CARGO_PKG_VERSION")),
                    )
                    .send()
            })
            .await?;
        let data = response
            .bytes()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        serde_json::from_slice(&data)
            .map_err(|e| crate::security::json_error(e, &[self.inner.token.as_str()]))
    }
    /// Multipart API access, confined to typed message/emoji upload endpoints.
    pub async fn request_upload(
        &self,
        path: &str,
        body: serde_json::Value,
        files: &[UploadFile],
    ) -> Result<serde_json::Value> {
        validate_uploads(files)?;
        let parts: Vec<_> = path.split('/').collect();
        let emoji =
            matches!(parts.as_slice(),["","servers",id,"emojis"] if Uuid::parse_str(id).is_ok());
        let message =
            matches!(parts.as_slice(),["","channels",id,"messages"] if Uuid::parse_str(id).is_ok());
        if !emoji && !message {
            return Err(YappinError::validation(
                "path",
                "unsupported upload endpoint",
            ));
        }
        if emoji {
            let name = body["name"]
                .as_str()
                .ok_or_else(|| YappinError::validation("name", "emoji name required"))?;
            validate_emoji_name(name)?;
            if files.len() != 1
                || files[0].data.len() > 1_048_576
                || !matches!(files[0].mime()?, "image/png" | "image/gif" | "image/webp")
            {
                return Err(YappinError::validation(
                    "files",
                    "emoji requires one PNG/GIF/WebP up to 1 MiB",
                ));
            }
        } else {
            crate::validation::validate_message(&body, false)?;
        }
        let authorization = crate::security::authorization("Bot", self.inner.token.as_str())?;
        let payload = serde_json::to_string(&body)?;
        let response = self
            .execute_with_retry(&Method::POST, path, |url| {
                let mut form = if emoji {
                    reqwest::multipart::Form::new()
                        .text("name", body["name"].as_str().unwrap_or_default().to_owned())
                } else {
                    reqwest::multipart::Form::new().text("payload_json", payload.clone())
                };
                for (index, file) in files.iter().enumerate() {
                    // Bytes::clone is reference-counted; retries do not copy file buffers.
                    let part = reqwest::multipart::Part::stream_with_length(
                        reqwest::Body::from(file.data.clone()),
                        file.data.len() as u64,
                    )
                    .file_name(file.name.clone());
                    let part = part
                        .mime_str(file.mime().expect("validated mime"))
                        .expect("static MIME");
                    form = form.part(
                        if emoji {
                            "file".into()
                        } else {
                            format!("files[{index}]")
                        },
                        part,
                    );
                }
                self.inner
                    .http
                    .post(url)
                    .header(header::AUTHORIZATION, authorization.clone())
                    .header(
                        header::USER_AGENT,
                        concat!("rappin-rs/", env!("CARGO_PKG_VERSION")),
                    )
                    .multipart(form)
                    .send()
            })
            .await?;
        let data = response
            .bytes()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        let value = serde_json::from_slice(&data)
            .map_err(|e| crate::security::json_error(e, &[self.inner.token.as_str()]))?;
        Ok(self.redact_credentials(value))
    }
    pub async fn get_api_version(&self) -> Result<VersionInfo> {
        ApiExt::get_api_version(self).await
    }
    pub async fn get_api_changelog(&self, limit: u32) -> Result<ApiChangelog> {
        ApiExt::get_api_changelog(self, limit).await
    }
    pub async fn get_emojis(&self, server: Uuid) -> Result<EmojisResponse> {
        ApiExt::get_emojis(self, server).await
    }
    pub async fn create_emoji(
        &self,
        server: Uuid,
        body: &CreateEmoji,
    ) -> Result<EmojiCreateResult> {
        ApiExt::create_emoji(self, server, body).await
    }
    pub async fn upload_emoji(
        &self,
        server: Uuid,
        name: &str,
        file: &UploadFile,
    ) -> Result<EmojiCreateResult> {
        ApiExt::upload_emoji(self, server, name, file).await
    }
    pub async fn edit_emoji(
        &self,
        server: Uuid,
        emoji: Uuid,
        name: &str,
    ) -> Result<EmojiCreateResult> {
        ApiExt::edit_emoji(self, server, emoji, name).await
    }
    pub async fn delete_emoji(&self, server: Uuid, emoji: Uuid) -> Result<()> {
        ApiExt::delete_emoji(self, server, emoji).await
    }
    pub async fn get_channel_webhooks(&self, channel: Uuid) -> Result<BotWebhooksResponse> {
        ApiExt::get_channel_webhooks(self, channel).await
    }
    pub async fn get_bot_webhooks(&self) -> Result<BotWebhooksResponse> {
        ApiExt::get_bot_webhooks(self).await
    }
    pub async fn create_bot_webhook(
        &self,
        channel: Uuid,
        body: &CreateBotWebhook,
    ) -> Result<BotWebhook> {
        ApiExt::create_bot_webhook(self, channel, body).await
    }
    pub async fn delete_bot_webhook(&self, webhook: Uuid) -> Result<()> {
        ApiExt::delete_bot_webhook(self, webhook).await
    }
    pub async fn post_bot_webhook(
        &self,
        webhook: Uuid,
        body: &crate::webhook::WebhookMessage,
    ) -> Result<Message> {
        ApiExt::post_bot_webhook(self, webhook, body).await
    }
    pub async fn send_files(
        &self,
        channel: Uuid,
        body: &CreateMessage,
        files: &[UploadFile],
    ) -> Result<Vec<Message>> {
        ApiExt::send_files(self, channel, body, files).await
    }
}
