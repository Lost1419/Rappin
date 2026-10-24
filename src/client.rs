use std::sync::Arc;
use std::time::Duration;

use reqwest::{header, Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::cache::Cache;
use crate::error::{Result, YappinError};
use crate::models::*;
use crate::utils::DEFAULT_API_BASE;

/// Builder for constructing a custom [`Client`].
pub struct ClientBuilder {
    token: String,
    base_url: String,
    max_retries: u32,
    http_client: Option<reqwest::Client>,
    cache: Option<Arc<Cache>>,
}

impl ClientBuilder {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base_url: DEFAULT_API_BASE.to_string(),
            max_retries: 3,
            http_client: None,
            cache: None,
        }
    }

    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    pub fn cache(mut self, cache: Arc<Cache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn build(self) -> Client {
        let http = self.http_client.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(40))
                .build()
                .expect("failed to create default reqwest client")
        });

        let cache = self.cache.unwrap_or_else(|| Arc::new(Cache::new()));

        Client {
            inner: Arc::new(ClientInner {
                http,
                token: self.token,
                base_url: self.base_url,
                max_retries: self.max_retries,
                cache,
            }),
        }
    }
}

struct ClientInner {
    http: reqwest::Client,
    token: String,
    base_url: String,
    max_retries: u32,
    cache: Arc<Cache>,
}

/// Asynchronous REST client for the Yapin' Bot API.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

impl Client {
    /// Creates a new `Client` with default settings and the given bot token.
    pub fn new(token: impl Into<String>) -> Self {
        ClientBuilder::new(token).build()
    }

    /// Creates a builder to customize the client.
    pub fn builder(token: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(token)
    }

    /// Returns the configured bot token.
    pub fn token(&self) -> &str {
        &self.inner.token
    }

    /// Returns the base URL of the API.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Returns a reference to the client's internal in-memory cache.
    pub fn cache(&self) -> &Cache {
        &self.inner.cache
    }

    // -----------------------------------------------------------------------
    // Core HTTP request execution with automatic rate limit / retry handling
    // -----------------------------------------------------------------------

    async fn execute_with_retry<F, Fut>(&self, mut operation: F) -> Result<Response>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Response, reqwest::Error>>,
    {
        let mut retries = self.inner.max_retries;

        loop {
            let res = operation().await?;
            let status = res.status();

            if status.is_success() {
                return Ok(res);
            }

            if status == StatusCode::UNAUTHORIZED {
                return Err(YappinError::Unauthorized);
            }

            let is_rate_limited = status == StatusCode::TOO_MANY_REQUESTS;
            let is_busy = status == StatusCode::SERVICE_UNAVAILABLE;

            if is_rate_limited || is_busy {
                let retry_after_header = res
                    .headers()
                    .get("Retry-After")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<f64>().ok());

                let body_bytes = res.bytes().await.unwrap_or_default();
                let err_json: serde_json::Value =
                    serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null);

                let retry_after = retry_after_header
                    .or_else(|| err_json.get("retry_after").and_then(|v| v.as_f64()))
                    .unwrap_or(1.0);

                if retries > 0 && retry_after <= 60.0 {
                    warn!(
                        "Yapin' API {} (retry_after: {:.1}s), waiting...",
                        if is_busy {
                            "busy (503)"
                        } else {
                            "rate limited (429)"
                        },
                        retry_after
                    );
                    sleep(Duration::from_secs_f64(retry_after)).await;
                    retries -= 1;
                    continue;
                }

                return Err(YappinError::RateLimited { retry_after });
            }

            let body_bytes = res.bytes().await.unwrap_or_default();
            let err_json: serde_json::Value =
                serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null);

            let msg = err_json
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or_else(|| status.canonical_reason().unwrap_or("Unknown error"))
                .to_string();

            return Err(YappinError::Api {
                status: status.as_u16(),
                message: msg,
                retry_after: None,
            });
        }
    }

    /// Performs an arbitrary JSON request against the API.
    pub async fn request_json<T: DeserializeOwned, B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        let url = format!("{}{}", self.inner.base_url, path);
        debug!("{} {}", method, url);

        let body_data = match body {
            Some(b) => Some(serde_json::to_vec(b)?),
            None => None,
        };

        let response = self
            .execute_with_retry(|| {
                let mut req = self
                    .inner
                    .http
                    .request(method.clone(), &url)
                    .header(header::AUTHORIZATION, format!("Bot {}", self.inner.token))
                    .header(header::USER_AGENT, "rappin-rs/0.1.1");

                if let Some(ref data) = body_data {
                    req = req
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(data.clone());
                }

                req.send()
            })
            .await?;

        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            // Support endpoints returning 200/204 with no body
            let val = serde_json::from_str("{}")?;
            return Ok(val);
        }

        let parsed = serde_json::from_slice(&bytes)?;
        Ok(parsed)
    }

    /// Performs a raw binary request, e.g. for avatar and banner updates.
    pub async fn request_binary<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<T> {
        let url = format!("{}{}", self.inner.base_url, path);
        debug!("{} {} ({})", method, url, content_type);

        let response = self
            .execute_with_retry(|| {
                self.inner
                    .http
                    .request(method.clone(), &url)
                    .header(header::AUTHORIZATION, format!("Bot {}", self.inner.token))
                    .header(header::USER_AGENT, "yappin-rs/0.1.0")
                    .header(header::CONTENT_TYPE, content_type)
                    .body(bytes.clone())
                    .send()
            })
            .await?;

        let resp_bytes = response.bytes().await?;
        let parsed = serde_json::from_slice(&resp_bytes)?;
        Ok(parsed)
    }

    // -----------------------------------------------------------------------
    // Bot / Me endpoints
    // -----------------------------------------------------------------------

    /// Fetches the profile of the authenticated bot (`GET /me`).
    pub async fn get_me(&self) -> Result<BotProfile> {
        let profile = self
            .request_json::<BotProfile, ()>(Method::GET, "/me", None)
            .await?;
        self.inner.cache.set_me(profile.clone()).await;
        Ok(profile)
    }

    /// Updates the display name or biography of the bot (`PATCH /me`).
    pub async fn update_me(&self, req: &UpdateBotProfile) -> Result<BotProfile> {
        self.request_json(Method::PATCH, "/me", Some(req)).await
    }

    /// Sets the bot's avatar (`PUT /me/avatar`). Accepts image data and MIME type (e.g. `image/png`).
    pub async fn set_avatar(&self, image_data: Vec<u8>, mime_type: &str) -> Result<Image> {
        self.request_binary(Method::PUT, "/me/avatar", image_data, mime_type)
            .await
    }

    /// Removes the bot's avatar (`DELETE /me/avatar`).
    pub async fn delete_avatar(&self) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(Method::DELETE, "/me/avatar", None)
            .await?;
        Ok(())
    }

    /// Sets the bot's profile banner (`PUT /me/banner`). Accepts image data and MIME type.
    pub async fn set_banner(&self, image_data: Vec<u8>, mime_type: &str) -> Result<Image> {
        self.request_binary(Method::PUT, "/me/banner", image_data, mime_type)
            .await
    }

    /// Removes the bot's banner (`DELETE /me/banner`).
    pub async fn delete_banner(&self) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(Method::DELETE, "/me/banner", None)
            .await?;
        Ok(())
    }

    /// Updates the bot's status and activity text (`PUT /me/status`).
    pub async fn set_status(&self, status: BotStatus, activity: Option<&str>) -> Result<()> {
        let req = StatusUpdateRequest {
            status,
            activity: activity.map(|s| s.to_string()),
        };
        let _: OkResponse = self
            .request_json(Method::PUT, "/me/status", Some(&req))
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Commands endpoints
    // -----------------------------------------------------------------------

    /// Retrieves the list of slash commands currently registered for the bot (`GET /commands`).
    pub async fn get_commands(&self) -> Result<Vec<Command>> {
        #[derive(Deserialize)]
        struct Resp {
            commands: Vec<Command>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, "/commands", None)
            .await?;
        Ok(resp.commands)
    }

    /// Replaces the registered slash commands for this bot (`PUT /commands`, up to 100).
    pub async fn set_commands(&self, commands: &[Command]) -> Result<Vec<Command>> {
        #[derive(Serialize)]
        struct Req<'a> {
            commands: &'a [Command],
        }
        #[derive(Deserialize)]
        struct Resp {
            commands: Vec<Command>,
        }
        let resp: Resp = self
            .request_json(Method::PUT, "/commands", Some(&Req { commands }))
            .await?;
        Ok(resp.commands)
    }

    // -----------------------------------------------------------------------
    // Events endpoints
    // -----------------------------------------------------------------------

    /// Long-polls for events (`GET /events`).
    ///
    /// - `after`: Last event id seen (0 returns no events and newest id as `last`).
    /// - `wait`: Seconds to hold open waiting for events (max 25).
    /// - `max_age`: Skip events older than this many seconds (max 86400).
    pub async fn get_events(
        &self,
        after: Option<u64>,
        wait: Option<u32>,
        max_age: Option<u32>,
    ) -> Result<EventsResponse> {
        let mut path = "/events?".to_string();
        if let Some(a) = after {
            path.push_str(&format!("after={}&", a));
        }
        if let Some(w) = wait {
            path.push_str(&format!("wait={}&", w));
        }
        if let Some(m) = max_age {
            path.push_str(&format!("max_age={}&", m));
        }
        let clean_path = path.trim_end_matches('&').trim_end_matches('?');
        self.request_json::<EventsResponse, ()>(Method::GET, clean_path, None)
            .await
    }

    // -----------------------------------------------------------------------
    // Server endpoints
    // -----------------------------------------------------------------------

    /// Lists all servers the bot is currently in (`GET /servers`).
    pub async fn get_servers(&self) -> Result<Vec<Server>> {
        #[derive(Deserialize)]
        struct Resp {
            servers: Vec<Server>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, "/servers", None)
            .await?;
        self.inner.cache.update_servers(resp.servers.clone()).await;
        Ok(resp.servers)
    }

    /// Fetches details for a specific server (`GET /servers/{id}`).
    pub async fn get_server(&self, server_id: Uuid) -> Result<Server> {
        #[derive(Deserialize)]
        struct Resp {
            server: Server,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/servers/{}", server_id), None)
            .await?;
        self.inner.cache.update_server(resp.server.clone()).await;
        Ok(resp.server)
    }

    /// Fetches roles defined on a server (`GET /servers/{id}/roles`).
    pub async fn get_roles(&self, server_id: Uuid) -> Result<Vec<Role>> {
        #[derive(Deserialize)]
        struct Resp {
            roles: Vec<Role>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/servers/{}/roles", server_id), None)
            .await?;
        Ok(resp.roles)
    }

    /// Fetches members of a server (`GET /servers/{id}/members`).
    pub async fn get_members(
        &self,
        server_id: Uuid,
        limit: Option<u32>,
        after: Option<Uuid>,
    ) -> Result<Vec<Member>> {
        let mut path = format!("/servers/{}/members?", server_id);
        if let Some(l) = limit {
            path.push_str(&format!("limit={}&", l));
        }
        if let Some(a) = after {
            path.push_str(&format!("after={}&", a));
        }
        let clean = path.trim_end_matches('&').trim_end_matches('?');
        #[derive(Deserialize)]
        struct Resp {
            members: Vec<Member>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, clean, None)
            .await?;
        Ok(resp.members)
    }

    /// Fetches one member with roles and timeout status (`GET /servers/{id}/members/{uid}`).
    pub async fn get_member(&self, server_id: Uuid, user_id: Uuid) -> Result<MemberDetail> {
        #[derive(Deserialize)]
        struct Resp {
            member: MemberDetail,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(
                Method::GET,
                &format!("/servers/{}/members/{}", server_id, user_id),
                None,
            )
            .await?;
        Ok(resp.member)
    }

    /// Sets or clears a member's nickname (`PATCH /servers/{id}/members/{uid}`).
    pub async fn set_nickname(&self, server_id: Uuid, user_id: Uuid, nickname: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Req<'a> {
            nickname: &'a str,
        }
        let _: OkResponse = self
            .request_json(
                Method::PATCH,
                &format!("/servers/{}/members/{}", server_id, user_id),
                Some(&Req { nickname }),
            )
            .await?;
        Ok(())
    }

    /// Gives a role to a server member (`PUT /servers/{id}/members/{uid}/roles/{rid}`).
    pub async fn add_role(&self, server_id: Uuid, user_id: Uuid, role_id: Uuid) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::PUT,
                &format!(
                    "/servers/{}/members/{}/roles/{}",
                    server_id, user_id, role_id
                ),
                None,
            )
            .await?;
        Ok(())
    }

    /// Removes a role from a server member (`DELETE /servers/{id}/members/{uid}/roles/{rid}`).
    pub async fn remove_role(&self, server_id: Uuid, user_id: Uuid, role_id: Uuid) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::DELETE,
                &format!(
                    "/servers/{}/members/{}/roles/{}",
                    server_id, user_id, role_id
                ),
                None,
            )
            .await?;
        Ok(())
    }

    /// Kicks a member from a server (`POST /servers/{id}/kick`).
    pub async fn kick_member(&self, server_id: Uuid, user_id: Uuid) -> Result<()> {
        #[derive(Serialize)]
        struct Req {
            user_id: Uuid,
        }
        let _: OkResponse = self
            .request_json(
                Method::POST,
                &format!("/servers/{}/kick", server_id),
                Some(&Req { user_id }),
            )
            .await?;
        Ok(())
    }

    /// Lists active bans in a server (`GET /servers/{id}/bans`).
    pub async fn get_bans(&self, server_id: Uuid) -> Result<Vec<Ban>> {
        #[derive(Deserialize)]
        struct Resp {
            bans: Vec<Ban>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/servers/{}/bans", server_id), None)
            .await?;
        Ok(resp.bans)
    }

    /// Bans a user from a server (`POST /servers/{id}/bans`).
    pub async fn ban_member(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        reason: Option<&str>,
    ) -> Result<()> {
        let req = BanRequest {
            user_id,
            reason: reason.map(|s| s.to_string()),
        };
        let _: OkResponse = self
            .request_json(
                Method::POST,
                &format!("/servers/{}/bans", server_id),
                Some(&req),
            )
            .await?;
        Ok(())
    }

    /// Unbans a user from a server (`DELETE /servers/{id}/bans/{uid}`).
    pub async fn unban_member(&self, server_id: Uuid, user_id: Uuid) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::DELETE,
                &format!("/servers/{}/bans/{}", server_id, user_id),
                None,
            )
            .await?;
        Ok(())
    }

    /// Times out a server member (`POST /servers/{id}/timeouts`). 0 minutes clears timeout.
    pub async fn timeout_member(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        minutes: u32,
        reason: Option<&str>,
    ) -> Result<()> {
        let req = TimeoutRequest {
            user_id,
            minutes,
            reason: reason.map(|s| s.to_string()),
        };
        let _: OkResponse = self
            .request_json(
                Method::POST,
                &format!("/servers/{}/timeouts", server_id),
                Some(&req),
            )
            .await?;
        Ok(())
    }

    /// Creates a category in a server (`POST /servers/{id}/categories`).
    pub async fn create_category(
        &self,
        server_id: Uuid,
        name: &str,
        position: Option<i32>,
    ) -> Result<Category> {
        let req = CreateCategoryRequest {
            name: name.to_string(),
            position,
        };
        #[derive(Deserialize)]
        struct Resp {
            category: Category,
        }
        let resp: Resp = self
            .request_json(
                Method::POST,
                &format!("/servers/{}/categories", server_id),
                Some(&req),
            )
            .await?;
        Ok(resp.category)
    }

    /// Renames a category (`PATCH /servers/{id}/categories/{cid}`).
    pub async fn edit_category(
        &self,
        server_id: Uuid,
        category_id: Uuid,
        name: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Req<'a> {
            name: &'a str,
        }
        let _: OkResponse = self
            .request_json(
                Method::PATCH,
                &format!("/servers/{}/categories/{}", server_id, category_id),
                Some(&Req { name }),
            )
            .await?;
        Ok(())
    }

    /// Deletes a category (`DELETE /servers/{id}/categories/{cid}`).
    pub async fn delete_category(&self, server_id: Uuid, category_id: Uuid) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::DELETE,
                &format!("/servers/{}/categories/{}", server_id, category_id),
                None,
            )
            .await?;
        Ok(())
    }

    /// Lists channels and categories in a server (`GET /servers/{id}/channels`).
    pub async fn get_server_channels(
        &self,
        server_id: Uuid,
        channel_type: Option<ChannelKind>,
    ) -> Result<ServerChannelsResponse> {
        let path = match channel_type {
            Some(ChannelKind::Text) => format!("/servers/{}/channels?type=text", server_id),
            Some(ChannelKind::Voice) => format!("/servers/{}/channels?type=voice", server_id),
            None => format!("/servers/{}/channels", server_id),
        };
        let resp = self
            .request_json::<ServerChannelsResponse, ()>(Method::GET, &path, None)
            .await?;
        self.inner
            .cache
            .update_channels(resp.channels.clone())
            .await;
        Ok(resp)
    }

    /// Creates a new channel in a server (`POST /servers/{id}/channels`).
    pub async fn create_channel(
        &self,
        server_id: Uuid,
        req: &CreateChannelRequest,
    ) -> Result<Channel> {
        #[derive(Deserialize)]
        struct Resp {
            channel: Channel,
        }
        let resp: Resp = self
            .request_json(
                Method::POST,
                &format!("/servers/{}/channels", server_id),
                Some(req),
            )
            .await?;
        self.inner.cache.update_channel(resp.channel.clone()).await;
        Ok(resp.channel)
    }

    /// Retrieves current active voice channels and users in a server (`GET /servers/{id}/voice`).
    pub async fn get_voice_states(&self, server_id: Uuid) -> Result<Vec<VoiceChannel>> {
        #[derive(Deserialize)]
        struct Resp {
            voice: Vec<VoiceChannel>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/servers/{}/voice", server_id), None)
            .await?;
        Ok(resp.voice)
    }

    /// Moves a member who is in voice to another voice channel (`POST /servers/{id}/voice/move`).
    pub async fn move_voice_member(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        channel_id: Uuid,
    ) -> Result<MoveMemberResponse> {
        let req = MoveMemberRequest::new(user_id, channel_id);
        self.request_json(
            Method::POST,
            &format!("/servers/{}/voice/move", server_id),
            Some(&req),
        )
        .await
    }

    /// Disconnects a user from voice in a server (`POST /servers/{id}/voice/disconnect`).
    pub async fn disconnect_voice_member(
        &self,
        server_id: Uuid,
        user_id: Uuid,
    ) -> Result<DisconnectMemberResponse> {
        let req = DisconnectMemberRequest::new(user_id);
        self.request_json(
            Method::POST,
            &format!("/servers/{}/voice/disconnect", server_id),
            Some(&req),
        )
        .await
    }

    /// Obtains a LiveKit ticket to join a server voice channel (`POST /servers/{id}/voice/join`).
    pub async fn join_voice(&self, server_id: Uuid, channel_id: Uuid) -> Result<VoiceTicket> {
        #[derive(Serialize)]
        struct Req {
            channel_id: Uuid,
        }
        self.request_json(
            Method::POST,
            &format!("/servers/{}/voice/join", server_id),
            Some(&Req { channel_id }),
        )
        .await
    }

    /// Removes the bot from voice in a server (`POST /servers/{id}/voice/leave`).
    pub async fn leave_voice(&self, server_id: Uuid) -> Result<()> {
        let _: serde_json::Value = self
            .request_json::<serde_json::Value, ()>(
                Method::POST,
                &format!("/servers/{}/voice/leave", server_id),
                None,
            )
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Channels endpoints
    // -----------------------------------------------------------------------

    /// Lists all channels the bot can see across all its servers (`GET /channels`).
    pub async fn get_channels(&self, channel_type: Option<ChannelKind>) -> Result<Vec<Channel>> {
        let path = match channel_type {
            Some(ChannelKind::Text) => "/channels?type=text".to_string(),
            Some(ChannelKind::Voice) => "/channels?type=voice".to_string(),
            None => "/channels".to_string(),
        };
        #[derive(Deserialize)]
        struct Resp {
            channels: Vec<Channel>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &path, None)
            .await?;
        self.inner
            .cache
            .update_channels(resp.channels.clone())
            .await;
        Ok(resp.channels)
    }

    /// Fetches a single channel by ID (`GET /channels/{id}`).
    pub async fn get_channel(&self, channel_id: Uuid) -> Result<ChannelRow> {
        #[derive(Deserialize)]
        struct Resp {
            channel: ChannelRow,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/channels/{}", channel_id), None)
            .await?;
        Ok(resp.channel)
    }

    /// Modifies a channel (`PATCH /channels/{id}`).
    pub async fn edit_channel(
        &self,
        channel_id: Uuid,
        req: &EditChannelRequest,
    ) -> Result<Channel> {
        #[derive(Deserialize)]
        struct Resp {
            channel: Channel,
        }
        let resp: Resp = self
            .request_json(
                Method::PATCH,
                &format!("/channels/{}", channel_id),
                Some(req),
            )
            .await?;
        self.inner.cache.update_channel(resp.channel.clone()).await;
        Ok(resp.channel)
    }

    /// Deletes a channel (`DELETE /channels/{id}`).
    pub async fn delete_channel(&self, channel_id: Uuid) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::DELETE,
                &format!("/channels/{}", channel_id),
                None,
            )
            .await?;
        self.inner.cache.remove_channel(&channel_id).await;
        Ok(())
    }

    /// Fetches recent messages from a channel (`GET /channels/{id}/messages`).
    pub async fn get_messages(
        &self,
        channel_id: Uuid,
        limit: Option<u32>,
        before: Option<Uuid>,
        after: Option<Uuid>,
    ) -> Result<MessagesResponse> {
        let mut path = format!("/channels/{}/messages?", channel_id);
        if let Some(l) = limit {
            path.push_str(&format!("limit={}&", l));
        }
        if let Some(b) = before {
            path.push_str(&format!("before={}&", b));
        }
        if let Some(a) = after {
            path.push_str(&format!("after={}&", a));
        }
        let clean = path.trim_end_matches('&').trim_end_matches('?');
        self.request_json::<MessagesResponse, ()>(Method::GET, clean, None)
            .await
    }

    /// Fetches a single message by ID (`GET /channels/{id}/messages/{mid}`).
    pub async fn get_message(
        &self,
        channel_id: Uuid,
        message_id: Uuid,
    ) -> Result<MessageDetailResponse> {
        self.request_json::<MessageDetailResponse, ()>(
            Method::GET,
            &format!("/channels/{}/messages/{}", channel_id, message_id),
            None,
        )
        .await
    }

    /// Posts a message into a channel (`POST /channels/{id}/messages`).
    pub async fn send_message(&self, channel_id: Uuid, message: &CreateMessage) -> Result<Message> {
        #[derive(Deserialize)]
        struct Resp {
            message: Message,
        }
        let resp: Resp = self
            .request_json(
                Method::POST,
                &format!("/channels/{}/messages", channel_id),
                Some(message),
            )
            .await?;
        Ok(resp.message)
    }

    /// Convenience shortcut to post a Components V2 layout message.
    pub async fn send_layout(
        &self,
        channel_id: Uuid,
        components: impl IntoIterator<Item = impl Into<Component>>,
        reply_to: Option<Uuid>,
    ) -> Result<Message> {
        let msg = CreateMessage::layout(components, reply_to);
        self.send_message(channel_id, &msg).await
    }

    /// Edits an existing message sent by the bot (`PATCH /channels/{id}/messages/{mid}`).
    pub async fn edit_message(
        &self,
        channel_id: Uuid,
        message_id: Uuid,
        edit: &EditMessage,
    ) -> Result<Message> {
        #[derive(Deserialize)]
        struct Resp {
            message: Message,
        }
        let resp: Resp = self
            .request_json(
                Method::PATCH,
                &format!("/channels/{}/messages/{}", channel_id, message_id),
                Some(edit),
            )
            .await?;
        Ok(resp.message)
    }

    /// Convenience shortcut to update a Components V2 layout message.
    pub async fn edit_layout(
        &self,
        channel_id: Uuid,
        message_id: Uuid,
        components: impl IntoIterator<Item = impl Into<Component>>,
    ) -> Result<Message> {
        let edit = EditMessage::layout(components);
        self.edit_message(channel_id, message_id, &edit).await
    }

    /// Deletes a message (`DELETE /channels/{id}/messages/{mid}`).
    pub async fn delete_message(&self, channel_id: Uuid, message_id: Uuid) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::DELETE,
                &format!("/channels/{}/messages/{}", channel_id, message_id),
                None,
            )
            .await?;
        Ok(())
    }

    /// Deletes up to 100 messages at once (`POST /channels/{id}/messages/bulk-delete`).
    pub async fn bulk_delete_messages(
        &self,
        channel_id: Uuid,
        message_ids: &[Uuid],
    ) -> Result<BulkDeleteResponse> {
        let req = BulkDeleteRequest::new(message_ids.iter().copied());
        self.request_json(
            Method::POST,
            &format!("/channels/{}/messages/bulk-delete", channel_id),
            Some(&req),
        )
        .await
    }

    /// Pins a message in a channel (`PUT /channels/{id}/messages/{mid}/pin`).
    pub async fn pin_message(&self, channel_id: Uuid, message_id: Uuid) -> Result<bool> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            pinned: bool,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(
                Method::PUT,
                &format!("/channels/{}/messages/{}/pin", channel_id, message_id),
                None,
            )
            .await?;
        Ok(resp.pinned)
    }

    /// Unpins a message in a channel (`DELETE /channels/{id}/messages/{mid}/pin`).
    pub async fn unpin_message(&self, channel_id: Uuid, message_id: Uuid) -> Result<bool> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            pinned: bool,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(
                Method::DELETE,
                &format!("/channels/{}/messages/{}/pin", channel_id, message_id),
                None,
            )
            .await?;
        Ok(resp.pinned)
    }

    /// Retrieves all reactions on a message (`GET /channels/{id}/messages/{mid}/reactions`).
    pub async fn get_reactions(&self, channel_id: Uuid, message_id: Uuid) -> Result<Vec<Reaction>> {
        #[derive(Deserialize)]
        struct Resp {
            reactions: Vec<Reaction>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(
                Method::GET,
                &format!("/channels/{}/messages/{}/reactions", channel_id, message_id),
                None,
            )
            .await?;
        Ok(resp.reactions)
    }

    /// Adds a reaction to a message (`PUT /channels/{id}/messages/{mid}/reactions/{emoji}`).
    pub async fn add_reaction(
        &self,
        channel_id: Uuid,
        message_id: Uuid,
        emoji: &str,
    ) -> Result<()> {
        let encoded: String = url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect();
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::PUT,
                &format!(
                    "/channels/{}/messages/{}/reactions/{}",
                    channel_id, message_id, encoded
                ),
                None,
            )
            .await?;
        Ok(())
    }

    /// Removes the bot's reaction from a message (`DELETE /channels/{id}/messages/{mid}/reactions/{emoji}`).
    pub async fn remove_reaction(
        &self,
        channel_id: Uuid,
        message_id: Uuid,
        emoji: &str,
    ) -> Result<()> {
        let encoded: String = url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect();
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(
                Method::DELETE,
                &format!(
                    "/channels/{}/messages/{}/reactions/{}",
                    channel_id, message_id, encoded
                ),
                None,
            )
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Interactions endpoints
    // -----------------------------------------------------------------------

    /// Responds to an interaction / button click (`POST /interactions/{id}/{token}/callback`).
    pub async fn respond_interaction(
        &self,
        interaction_id: &str,
        token: &str,
        callback: &InteractionCallback,
    ) -> Result<()> {
        let _: serde_json::Value = self
            .request_json(
                Method::POST,
                &format!("/interactions/{}/{}/callback", interaction_id, token),
                Some(callback),
            )
            .await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Users endpoints
    // -----------------------------------------------------------------------

    /// Fetches a user's public profile (`GET /users/{id}`).
    pub async fn get_user(&self, user_id: Uuid) -> Result<UserDetail> {
        #[derive(Deserialize)]
        struct Resp {
            user: UserDetail,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/users/{}", user_id), None)
            .await?;
        Ok(resp.user)
    }

    /// Opens or finds an existing direct message channel with a user (`POST /users/{id}/dm`).
    pub async fn create_dm(&self, user_id: Uuid) -> Result<Uuid> {
        #[derive(Deserialize)]
        struct Resp {
            channel_id: Uuid,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::POST, &format!("/users/{}/dm", user_id), None)
            .await?;
        Ok(resp.channel_id)
    }
}
