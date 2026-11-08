//! Authenticated REST client, optional gateway JSON routing and client-wide observations. Client clones share credentials, caches, rate-limit state and interaction-response ownership. Low-level requests bypass capability authorization; the caller retains responsibility for requester policy.
//!
//! # Example
//!
//! ```rust
//! use rappin::Client;
//! fn configure(token: String) -> Client {
//!     Client::builder(token).max_retries(0).build()
//! }
//! ```
use std::sync::Arc;
use std::time::Duration;

use reqwest::{header, Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::cache::Cache;
use crate::error::{Result, YappinError};
use crate::models::*;
use crate::utils::DEFAULT_API_BASE;

tokio::task_local! { static SINGLE_ATTEMPT: (); static SINGLE_ATTEMPT_STATUS: std::cell::Cell<Option<u16>>; }
tokio::task_local! { static READ_WAIT_BUDGET: ReadWaitBudget; }
/// Shared only by the attempts of one availability read, never by handlers.
#[derive(Clone)]
pub(crate) struct ReadWaitBudget {
    limit: Duration,
    remaining: Arc<std::sync::Mutex<Duration>>,
}
impl ReadWaitBudget {
    pub(crate) fn remaining(&self) -> Duration {
        *self.remaining.lock().expect("read wait budget")
    }
    pub(crate) fn exceeded(&self, source: YappinError) -> YappinError {
        YappinError::RetryWaitExceeded {
            waited: self.limit.saturating_sub(self.remaining()),
            budget: self.limit,
            source: Box::new(source),
        }
    }
    pub(crate) fn charge(&self, delay: Duration) {
        let mut remaining = self.remaining.lock().expect("read wait budget");
        *remaining = remaining.saturating_sub(delay);
    }
    pub(crate) async fn read<F: std::future::Future>(&self, future: F) -> F::Output {
        READ_WAIT_BUDGET
            .scope(self.clone(), single_attempt(future))
            .await
    }
}
pub(crate) async fn single_attempt<F: std::future::Future>(future: F) -> F::Output {
    SINGLE_ATTEMPT.scope((), future).await
}
pub(crate) fn single_attempt_active() -> bool {
    SINGLE_ATTEMPT.try_with(|()| ()).is_ok()
}

mod platform;
mod server_features;

/// Builder for constructing a custom [`Client`].
/// # Example
///
/// ```rust
/// use rappin::Client;
/// fn configure(token: String) -> Client {
///     Client::builder(token).max_retries(0).build()
/// }
/// ```
pub struct ClientBuilder {
    token: String,
    base_url: String,
    max_retries: u32,
    max_retry_wait: Duration,
    http_client: Option<reqwest::Client>,
    cache: Option<Arc<Cache>>,
    lookup_backend: Option<Arc<dyn crate::cache::LookupCache>>,
    api_fallback: bool,
}

impl ClientBuilder {
    /// Start a client builder using a privately obtained bot token. It does not authenticate until a request is made.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn configure(token: String) -> rappin::Client {
    ///     rappin::ClientBuilder::new(token).build()
    /// }
    /// ```
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base_url: DEFAULT_API_BASE.to_string(),
            max_retries: 3,
            max_retry_wait: Duration::from_secs(30),
            http_client: None,
            cache: None,
            lookup_backend: None,
            api_fallback: true,
        }
    }

    /// Select the API base URL and remove trailing slashes. A custom base disables default-origin failover.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).base_url("https://www.yapin.gg/api/bot/v1").build()
    /// }
    /// ```
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }

    /// Set the transport retry-attempt budget. Zero permits no extra attempts; this does not make uncertain writes safe to replay.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).max_retries(0).build()
    /// }
    /// ```
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Total server-advised/shared rate-limit waiting per request, default 30 s.
    /// This is separate from HTTP latency and the retry-attempt limit.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).max_retry_wait(std::time::Duration::from_secs(15)).build()
    /// }
    /// ```
    pub fn max_retry_wait(mut self, budget: Duration) -> Self {
        self.max_retry_wait = budget;
        self
    }

    /// Allow the default Yapin API to fail over to its documented website origin.
    /// Custom base URLs never fail over. Ambiguous writes are never replayed.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).api_fallback(false).build()
    /// }
    /// ```
    pub fn api_fallback(mut self, enabled: bool) -> Self {
        self.api_fallback = enabled;
        self
    }

    /// Use an application-owned lookup cache, such as Redis or a SQL database.
    /// Takes precedence over the built-in member/role cache; fresh requests still
    /// reach the API. The backend owns expiration, persistence and error policy.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, LookupCache};
    /// use std::sync::Arc;
    /// fn configure(token: String, backend: Arc<dyn LookupCache>) -> Client {
    ///     Client::builder(token).lookup_cache_backend(backend).build()
    /// }
    /// ```
    pub fn lookup_cache_backend(mut self, backend: Arc<dyn crate::cache::LookupCache>) -> Self {
        self.lookup_backend = Some(backend);
        self
    }

    /// Advanced trusted transport override. Configure redirect::Policy::none(), TLS
    /// certificate validation, and safe logging. Custom transports can access credentials.
    /// # Example
    ///
    /// ```rust
    /// fn configure(token: String) -> rappin::Result<rappin::Client> {
    ///     let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
    ///     Ok(rappin::Client::builder(token).http_client(http).build())
    /// }
    /// ```
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    /// Use an existing shared cache for this bot identity. Sharing profile state between unrelated bot identities is application-controlled and can produce incorrect snapshots.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).cache(std::sync::Arc::new(rappin::Cache::new())).build()
    /// }
    /// ```
    pub fn cache(mut self, cache: Arc<Cache>) -> Self {
        self.cache = Some(cache);
        self
    }
    /// Enable the built-in bounded member/role lookup cache with the selected capacity and TTL. Prefer fresh data for authorization.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).lookup_cache(rappin::CachePolicy::new(256, std::time::Duration::from_secs(30))).build()
    /// }
    /// ```
    pub fn lookup_cache(mut self, policy: crate::cache::CachePolicy) -> Self {
        self.cache = Some(Arc::new(Cache::with_lookup_policy(policy)));
        self
    }

    /// Construct a Client with shared cache, event hub and request telemetry.
    ///
    /// # Panics
    ///
    /// Panics if the default reqwest client cannot be created. Supply a prebuilt HTTP client to handle its construction error yourself.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Client;
    /// fn configure(token: String) -> Client {
    ///     Client::builder(token).max_retries(0).build()
    /// }
    /// ```
    pub fn build(self) -> Client {
        let http = self.http_client.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(40))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("failed to create default reqwest client")
        });

        let cache = self.cache.unwrap_or_else(|| Arc::new(Cache::new()));

        Client {
            inner: Arc::new(ClientInner {
                http,
                token: self.token,
                failover: crate::failover::Failover::new(&self.base_url, self.api_fallback),
                base_url: self.base_url,
                max_retries: self.max_retries,
                max_retry_wait: self.max_retry_wait,
                cache,
                lookup_backend: self.lookup_backend,
                events: crate::collectors::EventHub::default(),
                rate_limits: crate::rate_limits::RateLimits::default(),
                requests: tokio::sync::watch::channel(None).0,
                request_telemetry: Default::default(),
                interactions: crate::interactions::Registry::default(),
                auto_ack_tasks: std::sync::Mutex::new(Vec::new()),
                auto_ack_slots: Arc::new(tokio::sync::Semaphore::new(128)),
                #[cfg(feature = "gateway")]
                gateway_requests: Default::default(),
            }),
        }
    }
}

struct ClientInner {
    #[cfg(feature = "gateway")]
    gateway_requests: crate::gateway::requests::RequestRouter,
    request_telemetry: crate::diagnostics::RequestTelemetry,
    events: crate::collectors::EventHub,
    rate_limits: crate::rate_limits::RateLimits,
    requests: tokio::sync::watch::Sender<Option<crate::diagnostics::RequestObservation>>,
    interactions: crate::interactions::Registry,
    auto_ack_slots: Arc<tokio::sync::Semaphore>,
    auto_ack_tasks: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
    http: reqwest::Client,
    token: String,
    base_url: String,
    max_retries: u32,
    max_retry_wait: Duration,
    cache: Arc<Cache>,
    lookup_backend: Option<Arc<dyn crate::cache::LookupCache>>,
    failover: crate::failover::Failover,
}

/// Asynchronous REST client for the Yapin' Bot API.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::Client;
/// fn configure(token: String) -> Client {
///     Client::builder(token).max_retries(0).build()
/// }
/// ```
pub struct Client {
    inner: Arc<ClientInner>,
}

impl Client {
    /// Opt into JSON requests on a negotiated gateway connection. Configure
    /// before connecting; `None` restores HTTPS selection for subsequent calls.
    #[cfg(feature = "gateway")]
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, gateway::GatewayRequestConfig};
    /// fn configure(client: &Client) -> rappin::Result<()> {
    ///     client.configure_gateway_requests(Some(GatewayRequestConfig::default()))?;
    ///     // The gateway runner still needs to connect and advertise request support.
    ///     client.configure_gateway_requests(None)
    /// }
    /// ```
    pub fn configure_gateway_requests(
        &self,
        config: Option<crate::gateway::GatewayRequestConfig>,
    ) -> Result<()> {
        self.inner.gateway_requests.configure(config)
    }
    #[cfg(feature = "gateway")]
    /// Snapshot the configured gateway request capability, transport and bounded counters. This does not start a connection.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn inspect(client: &rappin::Client) {
    ///     let status = client.gateway_request_status();
    ///     println!("Gateway request capability: {}", status.capability);
    /// }
    /// ```
    pub fn gateway_request_status(&self) -> crate::gateway::GatewayRequestStatus {
        self.inner.gateway_requests.status()
    }
    #[cfg(feature = "gateway")]
    pub(crate) fn inner_gateway_router(&self) -> &crate::gateway::requests::RequestRouter {
        &self.inner.gateway_requests
    }
    pub(crate) fn read_wait_budget(&self) -> ReadWaitBudget {
        ReadWaitBudget {
            limit: self.inner.max_retry_wait,
            remaining: Arc::new(std::sync::Mutex::new(self.inner.max_retry_wait)),
        }
    }
    pub(crate) async fn request_json_once(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let mutation = !matches!(method, Method::GET | Method::HEAD);
        SINGLE_ATTEMPT_STATUS
            .scope(std::cell::Cell::new(None), async {
                let result = single_attempt(self.request_json(method, path, body.as_ref())).await;
                let status = SINGLE_ATTEMPT_STATUS.with(std::cell::Cell::get);
                result.map_err(|error| {
                    if mutation
                        && status.is_some()
                        && matches!(error, YappinError::Json(_) | YappinError::Http(_))
                    {
                        let source = self.redact_credentials(error.to_json());
                        let mut cause: crate::RemoteError =
                            serde_json::from_value(source.clone()).expect("structured cause");
                        cause.status = status;
                        cause.details.insert("source".into(), source);
                        cause.details.insert("outcome".into(), "unknown".into());
                        cause
                            .details
                            .insert("retry_safety".into(), "do_not_replay_workflow".into());
                        cause.into()
                    } else {
                        error
                    }
                })
            })
            .await
    }
    /// Creates a new `Client` with default settings and the given bot token.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::Client;
    ///
    /// let token = std::env::var("YAPPER_BOT_TOKEN").expect("set YAPPER_BOT_TOKEN");
    /// let client = Client::new(token);
    /// assert_eq!(client.base_url(), rappin::DEFAULT_API_BASE);
    /// ```
    pub fn new(token: impl Into<String>) -> Self {
        ClientBuilder::new(token).build()
    }

    /// Creates a builder to customize the client.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::Client;
    ///
    /// let token = std::env::var("YAPPER_BOT_TOKEN").expect("set YAPPER_BOT_TOKEN");
    /// let client = Client::builder(token)
    ///     .max_retries(5)
    ///     .build();
    /// assert_eq!(client.base_url(), rappin::DEFAULT_API_BASE);
    /// ```
    pub fn builder(token: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(token)
    }

    /// Private to authenticated transports; handlers are not given a token getter.
    #[cfg(any(feature = "gateway", feature = "voice"))]
    pub(crate) fn token(&self) -> &str {
        &self.inner.token
    }

    /// Returns the base URL of the API.
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     assert_eq!(client.base_url(), rappin::DEFAULT_API_BASE);
    /// }
    /// ```
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Current request origin, including any temporary fallback. `base_url()`
    /// continues to report the configured primary URL.
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     let active = client.active_base_url();
    ///     assert!(!active.is_empty());
    /// }
    /// ```
    pub fn active_base_url(&self) -> String {
        self.inner.failover.base(&self.inner.base_url)
    }

    /// Returns a reference to the client's internal in-memory cache.
    /// # Example
    ///
    /// ```rust
    /// async fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     let cache = client.cache();
    ///     let profile = cache.get_me().await;
    /// }
    /// ```
    pub fn cache(&self) -> &Cache {
        &self.inner.cache
    }

    /// Borrow the bounded ephemeral event hub. Publishing/observing hub events alone does not dispatch bot handlers or commit durable cursors.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) -> rappin::Result<()> {
    ///     let client = rappin::Client::new(token);
    ///     let subscription = client.events().collect::<rappin::MessageEvent>().timeout(std::time::Duration::from_secs(5)).subscribe()?;
    ///     drop(subscription);
    ///     Ok(())
    /// }
    /// ```
    pub fn events(&self) -> &crate::collectors::EventHub {
        &self.inner.events
    }
    /// Return the most recent sanitized attempt observation, or None before an observation exists. Body decoding and end-to-end UI latency are not included.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     assert!(client.last_request().is_none());
    /// }
    /// ```
    pub fn last_request(&self) -> Option<crate::diagnostics::RequestObservation> {
        self.inner.requests.borrow().clone()
    }
    /// Subscribe to latest-value request observations. Intermediate updates can coalesce; use cumulative metrics for reliable totals.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     let mut observations = client.subscribe_requests();
    ///     let latest = observations.borrow_and_update().clone();
    /// }
    /// ```
    pub fn subscribe_requests(
        &self,
    ) -> tokio::sync::watch::Receiver<Option<crate::diagnostics::RequestObservation>> {
        self.inner.requests.subscribe()
    }
    /// Exact totals across clones. The legacy watch subscription is latest-only.
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     let totals = client.request_metrics();
    ///     assert_eq!(totals.started, 0);
    /// }
    /// ```
    pub fn request_metrics(&self) -> crate::diagnostics::RequestMetrics {
        self.inner.request_telemetry.snapshot()
    }
    /// Bounded attempt stream (256 entries); slow readers receive explicit Lagged errors.
    /// # Example
    ///
    /// ```rust
    /// fn inspect(token: String) {
    ///     let client = rappin::Client::new(token);
    ///     let mut observations = client.subscribe_request_events();
    ///     assert!(matches!(observations.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
    /// }
    /// ```
    pub fn subscribe_request_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::diagnostics::RequestEvent> {
        self.inner.request_telemetry.subscribe()
    }
    pub(crate) fn interaction_registry(&self) -> &crate::interactions::Registry {
        &self.inner.interactions
    }
    pub(crate) fn auto_ack_permit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.inner.auto_ack_slots.clone().try_acquire_owned().ok()
    }
    pub(crate) fn track_auto_ack(&self, task: tokio::task::AbortHandle) {
        let mut tasks = self
            .inner
            .auto_ack_tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
    pub(crate) fn cancel_auto_ack(&self) {
        let tasks = std::mem::take(
            &mut *self
                .inner
                .auto_ack_tasks
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for task in tasks {
            task.abort();
        }
    }
    pub(crate) async fn receive_event(&self, event: crate::Event) {
        if matches!(event.kind.as_str(), "member_join" | "member_leave") {
            if let (Some(server), Some(user)) = (
                event.data["server_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok()),
                event.data["user_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok()),
            ) {
                self.invalidate_member(server, user).await;
            }
        }
        self.events().publish(event);
    }

    async fn invalidate_member(&self, server: Uuid, user: Uuid) {
        self.cache().invalidate_member(server, user).await;
        if let Some(backend) = &self.inner.lookup_backend {
            backend.invalidate_member(server, user).await;
        }
    }

    async fn invalidate_server_lookups(&self, server: Uuid) {
        self.cache().invalidate_server_lookups(server).await;
        if let Some(backend) = &self.inner.lookup_backend {
            backend.invalidate_server(server).await;
        }
    }

    /// Explicitly choose whether a fresh, bounded member snapshot may satisfy this lookup.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid, user: Uuid) -> Result<()> {
    ///     let response = client.fetch_member(server, user, Freshness::Refresh).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn fetch_member(
        &self,
        server: Uuid,
        user: Uuid,
        freshness: crate::cache::Freshness,
    ) -> Result<MemberDetail> {
        if freshness == crate::cache::Freshness::PreferCache {
            if let Some(backend) = &self.inner.lookup_backend {
                if let Some(value) = backend.get_member(server, user).await {
                    return Ok(value);
                }
            } else if let Some(cached) = self.cache().get_member(server, user).await {
                return Ok(cached.value);
            }
        }
        self.get_member(server, user).await
    }
    /// Fetch roles using the requested freshness policy. Refresh reaches the API; PreferCache can retain grants until the configured cache expires.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server: Uuid) -> Result<()> {
    ///     let response = client.fetch_roles(server, Freshness::Refresh).await?;
    ///     // A bot-visible resource snapshot does not independently prove requester access.
    ///     Ok(())
    /// }
    /// ```
    pub async fn fetch_roles(
        &self,
        server: Uuid,
        freshness: crate::cache::Freshness,
    ) -> Result<Vec<Role>> {
        if freshness == crate::cache::Freshness::PreferCache {
            if let Some(backend) = &self.inner.lookup_backend {
                if let Some(value) = backend.get_roles(server).await {
                    return Ok(value);
                }
            } else if let Some(cached) = self.cache().get_roles(server).await {
                return Ok(cached.value);
            }
        }
        self.get_roles(server).await
    }

    // -----------------------------------------------------------------------
    // Core HTTP request execution with automatic rate limit / retry handling
    // -----------------------------------------------------------------------

    async fn execute_with_retry<F, Fut, E>(
        &self,
        method: &Method,
        path: &str,
        operation: F,
    ) -> Result<Response>
    where
        F: FnMut(url::Url) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Response, E>>,
        E: Into<YappinError>,
    {
        let telemetry = self.inner.request_telemetry.start();
        let result = self
            .execute_attempts_with_retry(method, path, operation, &telemetry)
            .await;
        telemetry.finish(result.is_ok());
        result
    }

    async fn execute_attempts_with_retry<F, Fut, E>(
        &self,
        method: &Method,
        path: &str,
        mut operation: F,
        telemetry: &crate::diagnostics::RequestGuard<'_>,
    ) -> Result<Response>
    where
        F: FnMut(url::Url) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Response, E>>,
        E: Into<YappinError>,
    {
        let mut retries = if single_attempt_active() {
            0
        } else {
            self.inner.max_retries
        };
        let started = tokio::time::Instant::now();
        let key = crate::rate_limits::key(method, path);
        let is_message = crate::rate_limits::message_send(method, path);
        let route = crate::rate_limits::route_template(path);
        let mut wait = Duration::ZERO;
        let mut attempts = 0;
        let mut last_advice: Option<YappinError> = None;
        // Pin each call to its initial origin. Only an explicit safe-read
        // failover below may move it; concurrent failures must not move writes.
        let mut base = self.active_base_url();

        loop {
            let waiting_metric = telemetry.waiting();
            let remaining = self.inner.max_retry_wait.saturating_sub(wait);
            let remaining = READ_WAIT_BUDGET
                .try_with(|budget| budget.remaining().min(remaining))
                .unwrap_or(remaining);
            let waited =
                match tokio::time::timeout(remaining, self.inner.rate_limits.wait(key, is_message))
                    .await
                {
                    Ok(waited) => waited,
                    Err(_) => {
                        wait += remaining;
                        let _ = READ_WAIT_BUDGET.try_with(|budget| budget.charge(remaining));
                        let source = match last_advice.take() {
                            Some(error) => error,
                            None => YappinError::RateLimited {
                                retry_after: self
                                    .inner
                                    .rate_limits
                                    .remaining(key, is_message)
                                    .await
                                    .as_secs_f64(),
                            },
                        };
                        let error = READ_WAIT_BUDGET
                            .try_with(|budget| budget.exceeded(source.to_remote().into()));
                        return Err(error.unwrap_or_else(|_| YappinError::RetryWaitExceeded {
                            waited: wait,
                            budget: self.inner.max_retry_wait,
                            source: Box::new(source),
                        }));
                    }
                };
            waiting_metric.finish(waited);
            wait += waited;
            let _ = READ_WAIT_BUDGET.try_with(|budget| budget.charge(waited));
            let url = crate::security::api_url(&base, path)?;
            telemetry.attempt(attempts > 0);
            attempts += 1;
            let attempt_started = tokio::time::Instant::now();
            let res = operation(url).await.map_err(Into::into);
            let status = res.as_ref().ok().map(|res| res.status().as_u16());
            let _ = SINGLE_ATTEMPT_STATUS.try_with(|observed| observed.set(status));
            let label = if matches!(
                method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
            ) {
                method.as_str()
            } else {
                "OTHER"
            };
            let observation = crate::diagnostics::RequestObservation {
                method: label.into(),
                route: route.clone(),
                status,
                duration: started.elapsed(),
                rate_limit_wait: wait,
                attempts,
                api_version: res
                    .as_ref()
                    .ok()
                    .and_then(|r| r.headers().get("X-Yapin-API-Version"))
                    .and_then(|h| h.to_str().ok())
                    .filter(|s| s.len() <= 128)
                    .map(|s| crate::security::scrub(s, &[self.inner.token.as_str()])),
                api_build: res
                    .as_ref()
                    .ok()
                    .and_then(|r| r.headers().get("X-Yapin-Build"))
                    .and_then(|h| h.to_str().ok())
                    .filter(|s| s.len() <= 128)
                    .map(|s| crate::security::scrub(s, &[self.inner.token.as_str()])),
            };
            telemetry.observe(&observation, attempt_started.elapsed());
            self.inner.requests.send_replace(Some(observation));
            tracing::debug!(method=label,route=%route,status,elapsed_ms=started.elapsed().as_millis() as u64,attempts,"REST response headers");
            let unavailable = match &res {
                Ok(response) => matches!(
                    response.status(),
                    StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT
                ),
                Err(YappinError::Http(error)) => error.is_connect(),
                Err(_) => false,
            };
            if unavailable && self.inner.failover.activate(&base) {
                tracing::warn!("dedicated bot API unavailable; using website API for ten minutes");
                if matches!(*method, Method::GET | Method::HEAD) && retries > 0 {
                    retries -= 1;
                    base = self.active_base_url();
                    continue;
                }
            }
            // Transport/unknown-outcome errors are terminal, including submitted
            // gateway writes. Only explicit known-unapplied HTTP advice below
            // can spend the client retry budget.
            let res = res?;
            let status = res.status();

            if status.is_success() {
                return Ok(res);
            }

            if status == StatusCode::UNAUTHORIZED {
                return Err(YappinError::Unauthorized);
            }

            let retry_header = res
                .headers()
                .get("Retry-After")
                .and_then(|h| h.to_str().ok())
                .and_then(crate::rate_limits::retry_after);
            let cloudflare = res.headers().contains_key("cf-ray");
            let body_bytes = res.bytes().await.unwrap_or_default();
            let err_json: serde_json::Value =
                serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null);

            if cloudflare && status == StatusCode::TOO_MANY_REQUESTS && !err_json.is_object() {
                warn!("Cloudflare returned a rate limit before the Yapin API");
            }
            let msg = err_json
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or_else(|| status.canonical_reason().unwrap_or("Unknown error"))
                .to_string();

            // A headerless 429 is a permanent limit (e.g. exhausted interaction).
            // 503 guarantees the request did not execute; don't retry other writes.
            if matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
            ) {
                if let Some(delay) = retry_header {
                    if delay <= Duration::from_secs(60) {
                        let global = is_message
                            && (err_json["global"] == true
                                || msg.to_ascii_lowercase().contains("overall"));
                        self.inner.rate_limits.delay(key, delay, global).await;
                    }
                    if retries > 0 && delay <= Duration::from_secs(60) {
                        let source = YappinError::Api {
                            status: status.as_u16(),
                            message: crate::security::scrub(&msg, &[self.inner.token.as_str()]),
                            retry_after: Some(delay.as_secs_f64()),
                        };
                        if delay > self.inner.max_retry_wait.saturating_sub(wait) {
                            return Err(YappinError::RetryWaitExceeded {
                                waited: wait,
                                budget: self.inner.max_retry_wait,
                                source: Box::new(source),
                            });
                        }
                        warn!(
                            status = status.as_u16(),
                            delay_ms = delay.as_millis() as u64,
                            "API asked us to retry"
                        );
                        last_advice = Some(source);
                        retries -= 1;
                        continue;
                    }
                }
            }
            return Err(YappinError::Api {
                status: status.as_u16(),
                message: crate::security::scrub(&msg, &[&self.inner.token]),
                retry_after: retry_header.map(|d| d.as_secs_f64()),
            });
        }
    }

    /// Performs a JSON request confined to the configured origin and API prefix.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// use reqwest::Method;
    /// async fn read(client: &Client) -> Result<serde_json::Value> {
    ///     client.request_json::<serde_json::Value, ()>(Method::GET, "/me", None).await
    /// }
    /// ```
    pub async fn request_json<T: DeserializeOwned, B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        crate::security::api_url(&self.inner.base_url, path)?;
        let authorization = crate::security::authorization("Bot", &self.inner.token)?;
        debug!(%method, "Yapin API request");

        let body_data = match body {
            Some(b) => Some(bytes::Bytes::from(serde_json::to_vec(b)?)),
            None => None,
        };

        let callback = method == Method::POST
            && path.starts_with("/interactions/")
            && path.ends_with("/callback");
        let kind = if callback {
            body_data
                .as_ref()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
                .and_then(|body| body["type"].as_str().map(str::to_owned))
        } else {
            None
        };
        let tracked = if callback {
            Some(self.inner.interactions.entry(path)?)
        } else {
            None
        };
        let mut tracked = match &tracked {
            Some(entry) => Some(entry.lock().await),
            None => None,
        };
        let previous = tracked.as_ref().map(|entry| entry.state);
        if let Some(entry) = &mut tracked {
            use crate::interactions::ResponsePhase;
            if tokio::time::Instant::now() >= entry.expires {
                return Err(YappinError::validation(
                    "interaction",
                    "interaction expired",
                ));
            }
            if entry.state.phase == ResponsePhase::Uncertain {
                return Err(YappinError::validation(
                    "interaction",
                    "previous response outcome is uncertain; do not blindly retry",
                ));
            }
            if kind.as_deref() == Some("ack") && entry.state.phase != ResponsePhase::Unanswered {
                return serde_json::from_value(serde_json::json!({})).map_err(Into::into);
            }
            if kind.as_deref() == Some("modal") && entry.state.phase != ResponsePhase::Unanswered {
                return Err(YappinError::validation("modal", "modal must be the first response; acknowledgement or an earlier response prevents opening it"));
            }
            if kind.as_deref() == Some("modal") && entry.kind.as_deref() == Some("modal_submit") {
                return Err(YappinError::validation(
                    "modal",
                    "cannot open a modal from its submission",
                ));
            }
            if kind.as_deref() == Some("update")
                && (entry.kind.as_deref() == Some("command")
                    || entry.kind.as_deref() == Some("modal_submit") && !entry.has_message)
            {
                return Err(YappinError::validation(
                    "interaction",
                    "interaction has no source message to update",
                ));
            }
            // A cancelled request leaves Uncertain rather than permitting a duplicate initial response.
            entry.state.phase = ResponsePhase::Uncertain;
        }
        let response = self
            .execute_with_retry(&method, path, |url| {
                let mut req = self
                    .inner
                    .http
                    .request(method.clone(), url)
                    .header(header::AUTHORIZATION, authorization.clone())
                    .header(
                        header::USER_AGENT,
                        concat!("rappin-rs/", env!("CARGO_PKG_VERSION")),
                    );

                if let Some(ref data) = body_data {
                    req = req
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(data.clone());
                }

                #[cfg(feature = "gateway")]
                let gateway_method = method.clone();
                #[cfg(feature = "gateway")]
                let gateway_body = body_data.clone();
                async move {
                    #[cfg(feature = "gateway")]
                    if let Some(result) = self
                        .inner
                        .gateway_requests
                        .request(&gateway_method, path, gateway_body.as_deref())
                        .await
                    {
                        return result;
                    }
                    req.send()
                        .await
                        .map_err(|e| YappinError::Http(e.without_url()))
                }
            })
            .await;
        if let Some(entry) = &mut tracked {
            use crate::interactions::ResponsePhase;
            match &response {
                Ok(_) => {
                    entry.state.phase = if kind.as_deref() == Some("ack") {
                        ResponsePhase::Acknowledged
                    } else {
                        ResponsePhase::Responded
                    };
                    entry.state.responses = entry.state.responses.saturating_add(1);
                }
                Err(error) if error.to_json()["outcome"] == "unknown" => (),
                Err(YappinError::Api { .. } | YappinError::Unauthorized) => {
                    if let Some(previous) = previous {
                        entry.state = previous;
                    }
                }
                _ => (),
            }
        }
        drop(tracked);

        // Even a failed/ambiguous write may have changed permissions. Invalidate
        // once here so typed methods, raw requests and worker proxies agree.
        if !matches!(method, Method::GET | Method::HEAD) {
            let parts: Vec<_> = path
                .split('?')
                .next()
                .unwrap_or(path)
                .trim_matches('/')
                .split('/')
                .collect();
            if parts.first() == Some(&"servers")
                && (parts.get(2).is_some_and(|part| {
                    matches!(*part, "members" | "roles" | "kick" | "bans" | "timeouts")
                }) || (parts.get(2) == Some(&"categories")
                    && parts.get(4) == Some(&"permissions")))
            {
                if let Some(server) = parts.get(1).and_then(|id| Uuid::parse_str(id).ok()) {
                    self.invalidate_server_lookups(server).await;
                }
            }
        }

        let response = response?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        if bytes.is_empty() {
            // Support endpoints returning 200/204 with no body
            let val = serde_json::from_str("{}")?;
            return Ok(val);
        }

        let parsed = serde_json::from_slice(&bytes)
            .map_err(|e| crate::security::json_error(e, &[&self.inner.token]))?;
        Ok(parsed)
    }

    /// Performs a raw binary request, e.g. for avatar and banner updates.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Image, Result};
    /// use reqwest::Method;
    /// async fn avatar(client: &Client, png_bytes: Vec<u8>) -> Result<Image> {
    ///     client.request_binary(Method::PUT, "/me/avatar", png_bytes, "image/png").await
    /// }
    /// ```
    pub async fn request_binary<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<T> {
        crate::security::api_url(&self.inner.base_url, path)?;
        let bytes = bytes::Bytes::from(bytes);
        let authorization = crate::security::authorization("Bot", &self.inner.token)?;
        debug!(%method, %content_type, "Yapin binary request");

        let response = self
            .execute_with_retry(&method, path, |url| {
                self.inner
                    .http
                    .request(method.clone(), url)
                    .header(header::AUTHORIZATION, authorization.clone())
                    .header(
                        header::USER_AGENT,
                        concat!("rappin-rs/", env!("CARGO_PKG_VERSION")),
                    )
                    .header(header::CONTENT_TYPE, content_type)
                    .body(bytes.clone())
                    .send()
            })
            .await?;

        let resp_bytes = response
            .bytes()
            .await
            .map_err(|e| YappinError::Http(e.without_url()))?;
        let parsed = serde_json::from_slice(&resp_bytes)
            .map_err(|e| crate::security::json_error(e, &[&self.inner.token]))?;
        Ok(parsed)
    }

    // -----------------------------------------------------------------------
    // Bot / Me endpoints
    // -----------------------------------------------------------------------

    /// Fetches the profile of the authenticated bot (`GET /me`).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let me = client.get_me().await?;
    ///     println!("Bot username: {}", me.username);
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_me(&self) -> Result<BotProfile> {
        let profile = self
            .request_json::<BotProfile, ()>(Method::GET, "/me", None)
            .await?;
        self.inner.cache.set_me(profile.clone()).await;
        Ok(profile)
    }

    /// Updates the display name or biography of the bot (`PATCH /me`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.update_me(&UpdateBotProfile::new().display_name("Helper")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn update_me(&self, req: &UpdateBotProfile) -> Result<BotProfile> {
        if req
            .display_name
            .as_ref()
            .is_some_and(|s| s.chars().count() > 32)
            || req.bio.as_ref().is_some_and(|s| s.chars().count() > 300)
        {
            return Err(YappinError::validation(
                "profile",
                "display_name maximum 32 and bio maximum 300 characters (developer guide; server enforces its active limit)",
            ));
        }
        let profile: BotProfile = self.request_json(Method::PATCH, "/me", Some(req)).await?;
        self.inner.cache.set_me(profile.clone()).await;
        Ok(profile)
    }

    /// Sets the bot's avatar (`PUT /me/avatar`). Accepts image data and MIME type (e.g. `image/png`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, image_data: Vec<u8>) -> Result<()> {
    ///     let response = client.set_avatar(image_data, "image/png").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_avatar(&self, image_data: Vec<u8>, mime_type: &str) -> Result<Image> {
        self.request_binary(Method::PUT, "/me/avatar", image_data, mime_type)
            .await
    }

    /// Removes the bot's avatar (`DELETE /me/avatar`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.delete_avatar().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete_avatar(&self) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(Method::DELETE, "/me/avatar", None)
            .await?;
        Ok(())
    }

    /// Sets the bot's profile banner (`PUT /me/banner`). Accepts image data and MIME type.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, image_data: Vec<u8>) -> Result<()> {
    ///     let response = client.set_banner(image_data, "image/png").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_banner(&self, image_data: Vec<u8>, mime_type: &str) -> Result<Image> {
        self.request_binary(Method::PUT, "/me/banner", image_data, mime_type)
            .await
    }

    /// Removes the bot's banner (`DELETE /me/banner`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.delete_banner().await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn delete_banner(&self) -> Result<()> {
        let _: OkResponse = self
            .request_json::<OkResponse, ()>(Method::DELETE, "/me/banner", None)
            .await?;
        Ok(())
    }

    /// Updates the bot's status and activity text (`PUT /me/status`).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     client.set_status(BotStatus::Online, Some("Working in Rust 🦀")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_status(&self, status: BotStatus, activity: Option<&str>) -> Result<()> {
        if activity.is_some_and(|s| s.chars().count() > 60) {
            return Err(YappinError::validation("activity", "maximum 60 characters"));
        }
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.get_commands().await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.set_commands(&[Command::new("ping", "Check the bot")]).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_commands(&self, commands: &[Command]) -> Result<Vec<Command>> {
        crate::validation::validate_commands(commands)?;
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, handled_cursor: u64) -> Result<()> {
    ///     let response = client.get_events(Some(handled_cursor), Some(25), None).await?;
    ///     // This is received delivery, not a committed handled checkpoint.
    ///     let received_watermark = response.last;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_events(
        &self,
        after: Option<u64>,
        wait: Option<u32>,
        max_age: Option<u32>,
    ) -> Result<EventsResponse> {
        if wait.is_some_and(|w| w > 25) || max_age.is_some_and(|age| age > 86400) {
            return Err(YappinError::validation(
                "events",
                "wait maximum 25 and max_age maximum 86400 seconds",
            ));
        }
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
    ///
    /// Also populates the in-memory cache.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let servers = client.get_servers().await?;
    ///     for server in servers {
    ///         println!("Joined server: {}", server.name);
    ///     }
    ///     Ok(())
    /// }
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    /// use uuid::Uuid;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let server = client.get_server(Uuid::new_v4()).await?;
    ///     println!("Server name: {}", server.name);
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.get_roles(server_id).await?;
    ///     // A bot-visible resource snapshot does not independently prove requester access.
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_roles(&self, server_id: Uuid) -> Result<Vec<Role>> {
        #[derive(Deserialize)]
        struct Resp {
            roles: Vec<Role>,
        }
        let resp: Resp = self
            .request_json::<Resp, ()>(Method::GET, &format!("/servers/{}/roles", server_id), None)
            .await?;
        if let Some(backend) = &self.inner.lookup_backend {
            backend.store_roles(server_id, &resp.roles).await;
        } else {
            self.cache().update_roles(server_id, &resp.roles).await;
        }
        Ok(resp.roles)
    }

    /// Fetches members of a server (`GET /servers/{id}/members`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.get_members(server_id, Some(25), None).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_members(
        &self,
        server_id: Uuid,
        limit: Option<u32>,
        after: Option<Uuid>,
    ) -> Result<Vec<Member>> {
        if limit.is_some_and(|l| !(1..=500).contains(&l)) {
            return Err(YappinError::validation(
                "limit",
                "must be between 1 and 500",
            ));
        }
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.get_member(server_id, user_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
        if let Some(backend) = &self.inner.lookup_backend {
            backend.store_member(server_id, user_id, &resp.member).await;
        } else {
            self.cache()
                .update_member(server_id, user_id, &resp.member)
                .await;
        }
        Ok(resp.member)
    }

    /// Sets or clears a member's nickname (`PATCH /servers/{id}/members/{uid}`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.set_nickname(server_id, user_id, "River").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn set_nickname(&self, server_id: Uuid, user_id: Uuid, nickname: &str) -> Result<()> {
        if nickname.chars().count() > 32 {
            return Err(YappinError::validation("nickname", "maximum 32 characters"));
        }
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid, role_id: Uuid) -> Result<()> {
    ///     let response = client.add_role(server_id, user_id, role_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid, role_id: Uuid) -> Result<()> {
    ///     let response = client.remove_role(server_id, user_id, role_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.kick_member(server_id, user_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.get_bans(server_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.ban_member(server_id, user_id, Some("Repeated spam")).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.unban_member(server_id, user_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.timeout_member(server_id, user_id, 10, Some("Repeated spam")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn timeout_member(
        &self,
        server_id: Uuid,
        user_id: Uuid,
        minutes: u32,
        reason: Option<&str>,
    ) -> Result<()> {
        if minutes > 40320 || reason.is_some_and(|r| r.chars().count() > 200) {
            return Err(YappinError::validation(
                "timeout",
                "maximum 40320 minutes and 200 reason characters",
            ));
        }
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.create_category(server_id, "Information", Some(0)).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, category_id: Uuid) -> Result<()> {
    ///     let response = client.edit_category(server_id, category_id, "Information").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn edit_category(
        &self,
        server_id: Uuid,
        category_id: Uuid,
        name: &str,
    ) -> Result<()> {
        let req = EditCategoryRequest {
            name: name.to_owned(),
        };
        let _: OkResponse = self
            .request_json(
                Method::PATCH,
                &format!("/servers/{}/categories/{}", server_id, category_id),
                Some(&req),
            )
            .await?;
        Ok(())
    }

    /// Deletes a category (`DELETE /servers/{id}/categories/{cid}`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, category_id: Uuid) -> Result<()> {
    ///     let response = client.delete_category(server_id, category_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.get_server_channels(server_id, Some(ChannelKind::Text)).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.create_channel(server_id, &CreateChannelRequest::text("announcements")).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.get_voice_states(server_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid, channel_id: Uuid) -> Result<()> {
    ///     let response = client.move_voice_member(server_id, user_id, channel_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid, user_id: Uuid) -> Result<()> {
    ///     let response = client.disconnect_voice_member(server_id, user_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    /// use uuid::Uuid;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let ticket = client.join_voice(Uuid::new_v4(), Uuid::new_v4()).await?;
    ///     println!("LiveKit endpoint: {}", ticket.url);
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, server_id: Uuid) -> Result<()> {
    ///     let response = client.leave_voice(server_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client) -> Result<()> {
    ///     let response = client.get_channels(Some(ChannelKind::Text)).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid) -> Result<()> {
    ///     let response = client.get_channel(channel_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid) -> Result<()> {
    ///     let response = client.edit_channel(channel_id, &EditChannelRequest::new().name("announcements")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn edit_channel(
        &self,
        channel_id: Uuid,
        req: &EditChannelRequest,
    ) -> Result<Channel> {
        req.validate()?;
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid) -> Result<()> {
    ///     let response = client.delete_channel(channel_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid) -> Result<()> {
    ///     let response = client.get_messages(channel_id, Some(25), None, None).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_messages(
        &self,
        channel_id: Uuid,
        limit: Option<u32>,
        before: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<MessagesResponse> {
        if limit.is_some_and(|l| !(1..=100).contains(&l)) {
            return Err(YappinError::validation(
                "limit",
                "must be between 1 and 100",
            ));
        }
        if before.is_some() && after.is_some() {
            return Err(YappinError::validation("cursor", "choose before or after"));
        }
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            if let Some(l) = limit {
                query.append_pair("limit", &l.to_string());
            }
            if let Some(b) = before {
                query.append_pair("before", &b.to_rfc3339());
            }
            if let Some(a) = after {
                query.append_pair("after", &a.to_rfc3339());
            }
            format!("/channels/{channel_id}/messages?{}", query.finish())
        };
        self.request_json::<MessagesResponse, ()>(Method::GET, &path, None)
            .await
    }

    /// Fetches a single message by ID (`GET /channels/{id}/messages/{mid}`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.get_message(channel_id, message_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    /// use uuid::Uuid;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let channel_id = Uuid::new_v4();
    ///     let msg = CreateMessage::new("Hello from Rappin!");
    ///     let sent = client.send_message(channel_id, &msg).await?;
    ///     println!("Sent message ID: {}", sent.id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn send_message(&self, channel_id: Uuid, message: &CreateMessage) -> Result<Message> {
        let clean = message.for_request();
        let message = clean.as_ref();
        message.validate()?;
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    /// use uuid::Uuid;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let channel_id = Uuid::new_v4();
    ///     let container = Container::new()
    ///         .add(TextDisplay::new("# Announcement\nWelcome to our channel!"));
    ///     client.send_layout(channel_id, vec![container], None).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.edit_message(channel_id, message_id, &EditMessage::new().content("Updated")).await?;
    ///     Ok(())
    /// }
    /// ```
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
        let clean = edit.for_request();
        let edit = clean.as_ref();
        edit.validate()?;
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid, components: impl IntoIterator<Item = impl Into<Component>>) -> Result<()> {
    ///     let response = client.edit_layout(channel_id, message_id, components).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.delete_message(channel_id, message_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
    ///     let response = client.bulk_delete_messages(channel_id, message_ids).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn bulk_delete_messages(
        &self,
        channel_id: Uuid,
        message_ids: &[Uuid],
    ) -> Result<BulkDeleteResponse> {
        if message_ids.is_empty() || message_ids.len() > 100 {
            return Err(YappinError::validation(
                "ids",
                "requires 1 to 100 message IDs",
            ));
        }
        let req = BulkDeleteRequest::new(message_ids.iter().copied());
        self.request_json(
            Method::POST,
            &format!("/channels/{}/messages/bulk-delete", channel_id),
            Some(&req),
        )
        .await
    }

    /// Pins a message in a channel (`PUT /channels/{id}/messages/{mid}/pin`).
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.pin_message(channel_id, message_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.unpin_message(channel_id, message_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.get_reactions(channel_id, message_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.add_reaction(channel_id, message_id, "👍").await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, channel_id: Uuid, message_id: Uuid) -> Result<()> {
    ///     let response = client.remove_reaction(channel_id, message_id, "👍").await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, interaction_id: &str, token: &str) -> Result<()> {
    ///     let response = client.respond_interaction(interaction_id, token, &InteractionCallback::ephemeral("Recorded")).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn respond_interaction(
        &self,
        interaction_id: &str,
        token: &str,
        callback: &InteractionCallback,
    ) -> Result<()> {
        callback.validate()?;
        let interaction_id: String =
            url::form_urlencoded::byte_serialize(interaction_id.as_bytes())
                .collect::<String>()
                .replace('+', "%20");
        let token: String = url::form_urlencoded::byte_serialize(token.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::*;
    /// use uuid::Uuid;
    /// async fn example(client: &Client, user_id: Uuid) -> Result<()> {
    ///     let response = client.get_user(user_id).await?;
    ///     Ok(())
    /// }
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use rappin::prelude::*;
    /// use uuid::Uuid;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = Client::new("yb_token");
    ///     let dm_channel_id = client.create_dm(Uuid::new_v4()).await?;
    ///     client.send_message(dm_channel_id, &CreateMessage::new("Hello in DMs!")).await?;
    ///     Ok(())
    /// }
    /// ```
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

impl Client {
    pub(crate) fn redact_credentials(&self, value: serde_json::Value) -> serde_json::Value {
        crate::security::redact_json(value, &[&self.inner.token])
    }
    #[cfg(feature = "hot-reload")]
    pub(crate) fn worker_output(&self, value: serde_json::Value) -> serde_json::Value {
        self.redact_credentials(value)
    }
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;

#[cfg(test)]
mod failover_tests {
    use super::test_support::{mock, response};
    use super::*;
    use serde_json::json;

    fn client(primary: String, fallback: String, retries: u32) -> Client {
        let mut client = Client::builder("test-secret")
            .base_url(primary)
            .max_retries(retries)
            .build();
        Arc::get_mut(&mut client.inner).unwrap().failover =
            crate::failover::Failover::for_test(fallback);
        client
    }

    #[tokio::test]
    async fn read_failover_is_shared_and_does_not_loop_back() {
        let (primary, mut a, ta) = mock(vec![response(502, json!({"error":"gateway"}))]).await;
        let (fallback, mut b, tb) = mock(vec![
            response(200, json!({"ok":true})),
            response(504, json!({"error":"down"})),
        ])
        .await;
        let client = client(primary.clone(), fallback.clone(), 3);
        let value: serde_json::Value = client
            .request_json::<_, ()>(Method::GET, "/me", None)
            .await
            .unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(client.base_url(), primary);
        assert_eq!(client.clone().active_base_url(), fallback);
        let err = client
            .request_json::<serde_json::Value, ()>(Method::GET, "/me", None)
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), Some(504));
        assert_eq!(client.last_request().unwrap().attempts, 1);
        assert!(a.recv().await.unwrap().starts_with("GET /me "));
        assert!(b
            .recv()
            .await
            .unwrap()
            .contains("authorization: Bot test-secret"));
        ta.await.unwrap();
        tb.await.unwrap();
    }

    #[tokio::test]
    async fn ambiguous_writes_are_not_replayed_but_next_request_uses_fallback() {
        let (primary, mut a, ta) =
            mock(vec![response(502, json!({"error":"unknown outcome"}))]).await;
        let (fallback, mut b, tb) = mock(vec![response(200, json!({}))]).await;
        let client = client(primary, fallback, 3);
        let err = client
            .request_json::<serde_json::Value, _>(
                Method::POST,
                "/commands",
                Some(&json!({"commands":[]})),
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), Some(502));
        assert!(b.try_recv().is_err());
        client
            .request_json::<serde_json::Value, ()>(Method::GET, "/me", None)
            .await
            .unwrap();
        assert!(a.recv().await.unwrap().starts_with("POST /commands "));
        assert!(b.recv().await.unwrap().starts_with("GET /me "));
        ta.await.unwrap();
        tb.await.unwrap();
    }

    #[tokio::test]
    async fn shared_failover_during_a_server_advised_retry_does_not_move_a_write() {
        let (primary, mut requests, task) = mock(vec![
            (
                503,
                vec![("Retry-After", "0")],
                "{\"error\":\"busy\"}".into(),
            ),
            response(200, json!({})),
        ])
        .await;
        let fallback = "http://127.0.0.1:1".to_owned();
        let client = client(primary.clone(), fallback.clone(), 3);
        client
            .execute_with_retry(&Method::PUT, "/commands", |url| {
                let client = client.clone();
                let primary = primary.clone();
                async move {
                    let response = client.inner.http.put(url).send().await;
                    // Simulate another request activating the shared fallback.
                    client.inner.failover.activate(&primary);
                    response
                }
            })
            .await
            .unwrap();
        for _ in 0..2 {
            assert!(requests.recv().await.unwrap().starts_with("PUT /commands "));
        }
        assert_eq!(client.active_base_url(), fallback);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn connection_failure_can_fail_over_and_zero_retries_stays_zero() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let primary = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let (fallback, _rx, task) = mock(vec![response(200, json!({}))]).await;
        let client = client(primary, fallback, 0);
        assert!(client
            .request_json::<serde_json::Value, ()>(Method::GET, "/me", None)
            .await
            .is_err());
        assert_eq!(client.last_request().unwrap().attempts, 1);
        client
            .request_json::<serde_json::Value, ()>(Method::GET, "/me", None)
            .await
            .unwrap();
        task.await.unwrap();
    }
}
