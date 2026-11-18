//! Execution-time in-memory command cooldown buckets; read-only preparation must not consume them.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;

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
    /// Cooldown applies per-server, or per-requester in direct messages.
    Server,
    /// Cooldown applies globally.
    Global,
}

/// A rate limiter that checks active cooldowns and prunes expired buckets on use.
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
    buckets: Arc<Mutex<Buckets>>,
}

#[derive(Default)]
struct Buckets {
    entries: HashMap<String, Instant>,
    last_sweep: Option<Instant>,
}

pub(crate) fn server_key(server: Option<Uuid>, author: Uuid) -> String {
    server.map_or_else(|| format!("dm:{author}"), |server| server.to_string())
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
            buckets: Arc::new(Mutex::new(Buckets::default())),
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

    /// Creates a per-server cooldown with per-requester buckets in direct messages.
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
            CooldownType::Server => server_key(ctx.server_id(), ctx.author_id()),
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
        if self.duration.is_zero() {
            return None;
        }
        let mut buckets = self.buckets.lock().await;
        let now = Instant::now();
        // Amortize global cleanup while checking the current bucket precisely.
        let sweep_interval = self.duration.min(Duration::from_secs(60));
        if buckets
            .last_sweep
            .is_none_or(|last| now - last >= sweep_interval)
        {
            buckets
                .entries
                .retain(|_, last| now - *last < self.duration);
            buckets.last_sweep = Some(now);
        }

        if let Some(&last_time) = buckets.entries.get(key) {
            let elapsed = now.duration_since(last_time);
            if elapsed < self.duration {
                return Some(self.duration - elapsed);
            }
        }

        buckets.entries.insert(key.to_string(), now);
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
        self.buckets.lock().await.entries.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, CommandContext, MessageEvent};

    #[tokio::test]
    async fn expired_entries_are_pruned_without_resetting_live_buckets() {
        let cooldown = Cooldown::user(Duration::from_secs(60));
        assert!(cooldown.check_key("live").await.is_none());
        {
            let mut buckets = cooldown.buckets.lock().await;
            let past = Instant::now() - Duration::from_secs(120);
            buckets.last_sweep = Some(past);
            buckets.entries.insert("expired".into(), past);
        }
        assert!(cooldown.check_key("live").await.is_some());
        assert!(!cooldown
            .buckets
            .lock()
            .await
            .entries
            .contains_key("expired"));
        assert!(cooldown.check_key("expired").await.is_none());
    }

    #[tokio::test]
    async fn direct_message_server_cooldowns_share_per_requester_buckets_across_contexts() {
        fn dm(author: u128, channel: u128) -> MessageContext {
            let event: MessageEvent = serde_json::from_value(serde_json::json!({
                "id": Uuid::from_u128(1),
                "author_id": Uuid::from_u128(author),
                "channel_id": Uuid::from_u128(channel),
                "is_dm": true,
                "content": "!example",
            }))
            .unwrap();
            MessageContext::new(
                Arc::new(Client::new("fixture")),
                event,
                None,
                String::new(),
                None,
            )
        }
        let cooldown = Cooldown::server(Duration::from_secs(60));
        assert!(cooldown.check(&dm(2, 3)).await.is_none());
        assert!(CommandContext::from(dm(2, 4))
            .check_cooldown(&cooldown)
            .await
            .unwrap()
            .is_some());
        assert!(CommandContext::from(dm(5, 3))
            .check_cooldown(&cooldown)
            .await
            .unwrap()
            .is_none());
    }
}
