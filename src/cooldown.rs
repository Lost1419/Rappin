//! Execution-time in-memory command cooldown buckets; read-only preparation must not consume them.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::bot::context::MessageContext;

/// The scope of a rate-limit cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// # Example
///
/// ```rust
/// use rappin::cooldown::{Cooldown, CooldownType};
/// use std::time::Duration;
/// let cooldown = Cooldown::new(CooldownType::User, Duration::from_secs(10));
/// assert_eq!(cooldown.duration(), Duration::from_secs(10));
/// assert_eq!(cooldown.kind(), CooldownType::User);
/// ```
pub enum CooldownType {
    /// Cooldown applies per-user.
    User,
    /// Cooldown applies per-channel.
    Channel,
    /// Cooldown applies per-server.
    Server,
    /// Cooldown applies globally.
    Global,
}

/// A rate limiter that tracks cooldown buckets.
#[derive(Clone)]
/// # Example
///
/// ```rust
/// use rappin::cooldown::{Cooldown, CooldownType};
/// use std::time::Duration;
/// let cooldown = Cooldown::new(CooldownType::User, Duration::from_secs(10));
/// assert_eq!(cooldown.duration(), Duration::from_secs(10));
/// assert_eq!(cooldown.kind(), CooldownType::User);
/// ```
pub struct Cooldown {
    kind: CooldownType,
    duration: Duration,
    buckets: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Cooldown {
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::{Cooldown, CooldownType};
    /// use std::time::Duration;
    /// let cooldown = Cooldown::new(CooldownType::User, Duration::from_secs(10));
    /// assert_eq!(cooldown.duration(), Duration::from_secs(10));
    /// assert_eq!(cooldown.kind(), CooldownType::User);
    /// ```
    pub fn new(kind: CooldownType, duration: Duration) -> Self {
        Self {
            kind,
            duration,
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Creates a per-user cooldown.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use rappin::cooldown::{Cooldown, CooldownType};
    ///
    /// let cd = Cooldown::user(Duration::from_secs(30));
    /// assert_eq!(cd.kind(), CooldownType::User);
    /// assert_eq!(cd.duration(), Duration::from_secs(30));
    /// ```
    pub fn user(duration: Duration) -> Self {
        Self::new(CooldownType::User, duration)
    }

    /// Creates a per-channel cooldown.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use rappin::cooldown::{Cooldown, CooldownType};
    ///
    /// let cd = Cooldown::channel(Duration::from_secs(10));
    /// assert_eq!(cd.kind(), CooldownType::Channel);
    /// ```
    pub fn channel(duration: Duration) -> Self {
        Self::new(CooldownType::Channel, duration)
    }

    /// Creates a per-server cooldown.
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::{Cooldown, CooldownType};
    /// use std::time::Duration;
    /// let cooldown = Cooldown::server(Duration::from_secs(10));
    /// assert_eq!(cooldown.kind(), CooldownType::Server);
    /// ```
    pub fn server(duration: Duration) -> Self {
        Self::new(CooldownType::Server, duration)
    }

    /// Creates a global cooldown across all servers and users.
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::{Cooldown, CooldownType};
    /// use std::time::Duration;
    /// let cooldown = Cooldown::global(Duration::from_secs(10));
    /// assert_eq!(cooldown.kind(), CooldownType::Global);
    /// ```
    pub fn global(duration: Duration) -> Self {
        Self::new(CooldownType::Global, duration)
    }

    /// Returns the cooldown duration.
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::{Cooldown, CooldownType};
    /// use std::time::Duration;
    /// let cooldown = Cooldown::new(CooldownType::User, Duration::from_secs(10));
    /// assert_eq!(cooldown.duration(), Duration::from_secs(10));
    /// assert_eq!(cooldown.kind(), CooldownType::User);
    /// ```
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the cooldown type.
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::{Cooldown, CooldownType};
    /// use std::time::Duration;
    /// let cooldown = Cooldown::new(CooldownType::User, Duration::from_secs(10));
    /// assert_eq!(cooldown.duration(), Duration::from_secs(10));
    /// assert_eq!(cooldown.kind(), CooldownType::User);
    /// ```
    pub fn kind(&self) -> CooldownType {
        self.kind
    }

    /// Checks if a command invocation is on cooldown.
    ///
    /// If on cooldown, returns `Some(remaining_duration)`.
    /// If not on cooldown, records the invocation and returns `None`.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{MessageContext, cooldown::Cooldown};
    /// async fn admit(cooldown: &Cooldown, ctx: &MessageContext) -> bool {
    ///     // A successful check consumes a bucket; do not call during preparation.
    ///     cooldown.check(ctx).await.is_none()
    /// }
    /// ```
    pub async fn check(&self, ctx: &MessageContext) -> Option<Duration> {
        let key = match self.kind {
            CooldownType::User => ctx.author_id().to_string(),
            CooldownType::Channel => ctx.channel_id().to_string(),
            CooldownType::Server => ctx
                .server_id()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "dm".to_string()),
            CooldownType::Global => "global".to_string(),
        };

        self.check_key(&key).await
    }

    /// Checks if a specific key is on cooldown.
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let cooldown = Cooldown::user(Duration::from_secs(10));
    /// assert!(cooldown.check_key("member:123").await.is_none());
    /// assert!(cooldown.check_key("member:123").await.is_some());
    /// cooldown.reset_key("member:123").await;
    /// assert!(cooldown.check_key("member:123").await.is_none());
    /// # }
    /// ```
    pub async fn check_key(&self, key: &str) -> Option<Duration> {
        let mut buckets = self.buckets.lock().await;
        let now = Instant::now();

        if let Some(&last_time) = buckets.get(key) {
            let elapsed = now.duration_since(last_time);
            if elapsed < self.duration {
                return Some(self.duration - elapsed);
            }
        }

        buckets.insert(key.to_string(), now);
        None
    }

    /// Manually resets the cooldown for a specific key.
    /// # Example
    ///
    /// ```rust
    /// use rappin::cooldown::Cooldown;
    /// use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let cooldown = Cooldown::user(Duration::from_secs(10));
    /// assert!(cooldown.check_key("member:123").await.is_none());
    /// assert!(cooldown.check_key("member:123").await.is_some());
    /// cooldown.reset_key("member:123").await;
    /// assert!(cooldown.check_key("member:123").await.is_none());
    /// # }
    /// ```
    pub async fn reset_key(&self, key: &str) {
        self.buckets.lock().await.remove(key);
    }
}
