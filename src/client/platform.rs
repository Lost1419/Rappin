use super::*;
use crate::ApiExt;
impl Client {
    /// Unauthenticated version/changelog reads using the configured transport and failover.
    pub(crate) async fn public_json(&self, path: &str) -> Result<serde_json::Value> {
        if path != "/version" && !path.starts_with("/changelog?") {
            return Err(Error::validation("path", "not a public metadata endpoint"));
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
            .map_err(|e| Error::Http(e.without_url()))?;
        serde_json::from_slice(&data)
            .map_err(|e| crate::security::json_error(e, &[self.inner.token.as_str()]))
    }
    /// Multipart API access, confined to typed message/emoji upload endpoints.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, UploadFile, Result};
    /// async fn upload(client: &Client, channel: uuid::Uuid, file: UploadFile) -> Result<serde_json::Value> {
    ///     client.request_upload(&format!("/channels/{channel}/messages"), serde_json::json!({"content":"Report"}), &[file]).await
    /// }
    /// ```
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
            return Err(Error::validation("path", "unsupported upload endpoint"));
        }
        if emoji {
            let name = body["name"]
                .as_str()
                .ok_or_else(|| Error::validation("name", "emoji name required"))?;
            validate_emoji_name(name)?;
            if files.len() != 1
                || files[0].data.len() > 2_097_152
                || !matches!(files[0].mime()?, "image/png" | "image/gif" | "image/webp")
            {
                return Err(Error::validation(
                    "files",
                    "emoji requires one PNG/GIF/WebP up to 2 MiB",
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
            .map_err(|e| Error::Http(e.without_url()))?;
        let value = serde_json::from_slice(&data)
            .map_err(|e| crate::security::json_error(e, &[self.inner.token.as_str()]))?;
        Ok(self.redact_credentials(value))
    }
    /// Read public API/build metadata and official SDK versions without Authorization. The response does not describe the installed Rappin version.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.get_api_version().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_api_version(&self) -> Result<VersionInfo> {
        ApiExt::get_api_version(self).await
    }
    /// Read the public platform changelog, requesting 1 through 50 entries.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.get_api_changelog(10).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_api_changelog(&self, limit: u32) -> Result<ApiChangelog> {
        ApiExt::get_api_changelog(self, limit).await
    }
    /// Read server custom-emoji inventory and its slot limit.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid) -> Result<()> {
    ///     let response = client.get_emojis(server).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_emojis(
        &self,
        server: impl Into<crate::ServerId> + Send,
    ) -> Result<EmojisResponse> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        ApiExt::get_emojis(self, server).await
    }
    /// Create a server emoji from a validated base64/data-URL image. Local validation currently caps decoded images at 2 MiB; moderation may return a pending result.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid, body: &CreateEmoji) -> Result<()> {
    ///     let response = client.create_emoji(server, body).await?;
    ///     // Propagate partial errors; inspect moderation-pending success where applicable.
    ///     Ok(())
    /// }
    /// ```
    pub async fn create_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        body: &CreateEmoji,
    ) -> Result<EmojiCreateResult> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        ApiExt::create_emoji(self, server, body).await
    }
    /// Upload one PNG/GIF/WebP emoji through the shared multipart transport. The current SDK cap is 2 MiB, and pending moderation is not a failed creation.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid, file: &UploadFile) -> Result<()> {
    ///     let response = client.upload_emoji(server, "wave", file).await?;
    ///     // Propagate partial errors; inspect moderation-pending success where applicable.
    ///     Ok(())
    /// }
    /// ```
    pub async fn upload_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        name: &str,
        file: &UploadFile,
    ) -> Result<EmojiCreateResult> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);

        ApiExt::upload_emoji(self, server, name, file).await
    }
    /// Rename a server emoji after validating its name. The platform enforces Manage Emoji and resource scope.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid, emoji: Uuid) -> Result<()> {
    ///     let response = client.edit_emoji(server, emoji, "wave").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn edit_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        emoji: impl Into<crate::EmojiId> + Send,
        name: &str,
    ) -> Result<EmojiCreateResult> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let emoji: crate::EmojiId = emoji.into();
        let emoji = Uuid::from(emoji);

        ApiExt::edit_emoji(self, server, emoji, name).await
    }
    /// Delete the selected server emoji. An uncertain result must be reconciled rather than replayed automatically.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid, emoji: Uuid) -> Result<()> {
    ///     let response = client.delete_emoji(server, emoji).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete_emoji(
        &self,
        server: impl Into<crate::ServerId> + Send,
        emoji: impl Into<crate::EmojiId> + Send,
    ) -> Result<()> {
        let server: crate::ServerId = server.into();
        let server = Uuid::from(server);
        let emoji: crate::EmojiId = emoji.into();
        let emoji = Uuid::from(emoji);

        ApiExt::delete_emoji(self, server, emoji).await
    }
    /// Read bot-owned webhooks visible in the selected channel.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel: Uuid) -> Result<()> {
    ///     let response = client.get_channel_webhooks(channel).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_channel_webhooks(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
    ) -> Result<BotWebhooksResponse> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        ApiExt::get_channel_webhooks(self, channel).await
    }
    /// Read webhooks owned by the authenticated bot.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.get_bot_webhooks().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_bot_webhooks(&self) -> Result<BotWebhooksResponse> {
        ApiExt::get_bot_webhooks(self).await
    }
    /// Create a bot-owned webhook in a channel after validating its display name.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel: Uuid) -> Result<()> {
    ///     let response = client.create_bot_webhook(channel, &CreateBotWebhook::new("Notifier")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn create_bot_webhook(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        body: &CreateBotWebhook,
    ) -> Result<BotWebhook> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        ApiExt::create_bot_webhook(self, channel, body).await
    }
    /// Delete a bot-owned webhook by ID. The API enforces ownership and channel permissions.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, webhook: Uuid) -> Result<()> {
    ///     let response = client.delete_bot_webhook(webhook).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete_bot_webhook(
        &self,
        webhook: impl Into<crate::WebhookId> + Send,
    ) -> Result<()> {
        let webhook: crate::WebhookId = webhook.into();
        let webhook = Uuid::from(webhook);

        ApiExt::delete_bot_webhook(self, webhook).await
    }
    /// Post through a bot-owned webhook using bot authentication, preserving confirmed message data on success.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, webhook: Uuid) -> Result<()> {
    ///     let response = client.post_bot_webhook(webhook, &WebhookMessage { content: Some("Service is online".into()), ..Default::default() }).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn post_bot_webhook(
        &self,
        webhook: impl Into<crate::WebhookId> + Send,
        body: &crate::webhook::WebhookMessage,
    ) -> Result<Message> {
        let webhook: crate::WebhookId = webhook.into();
        let webhook = Uuid::from(webhook);

        ApiExt::post_bot_webhook(self, webhook, body).await
    }
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel: Uuid, files: &[UploadFile]) -> Result<()> {
    ///     let response = client.send_files(channel, &CreateMessage::new("Hello from the bot"), files).await?;
    ///     // Propagate partial errors; inspect moderation-pending success where applicable.
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_files(
        &self,
        channel: impl Into<crate::ChannelId> + Send,
        body: &CreateMessage,
        files: &[UploadFile],
    ) -> Result<Vec<Message>> {
        let channel: crate::ChannelId = channel.into();
        let channel = Uuid::from(channel);

        ApiExt::send_files(self, channel, body, files).await
    }
}
