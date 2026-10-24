use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::bot::context::MessageContext;

/// The scope of a rate-limit cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub struct Cooldown {
    kind: CooldownType,
    duration: Duration,
    buckets: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Cooldown {
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
    pub fn server(duration: Duration) -> Self {
        Self::new(CooldownType::Server, duration)
    }

    /// Creates a global cooldown across all servers and users.
    pub fn global(duration: Duration) -> Self {
        Self::new(CooldownType::Global, duration)
    }

    /// Returns the cooldown duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the cooldown type.
    pub fn kind(&self) -> CooldownType {
        self.kind
    }

    /// Checks if a command invocation is on cooldown.
    ///
    /// If on cooldown, returns `Some(remaining_duration)`.
    /// If not on cooldown, records the invocation and returns `None`.
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
    pub async fn reset_key(&self, key: &str) {
        self.buckets.lock().await.remove(key);
    }
}
