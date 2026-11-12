//! Shared entity snapshots and optional application-owned member/role lookup storage.
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::{BotProfile, Channel, Server};
use crate::models::{MemberDetail, Role};
use std::{hash::Hash, time::Duration};
use tokio::time::Instant;

/// Application-owned member/role lookup storage. Implement this with your own
/// database, Redis client, or cache. The SDK supplies no database dependencies.
///
/// Implementations own TTL, capacity and failure policy: return `None` on a miss
/// (or an unavailable backend) to fetch from the API. Store/invalidation failures
/// should be handled by the implementation. Never retain credentials here.
/// `Freshness::Refresh` always bypasses reads, but updates this backend afterward.
#[async_trait::async_trait]
/// # Example
///
/// ```rust
/// use rappin::LookupCache;
/// use uuid::Uuid;
/// async fn invalidate_after_role_change(storage: &dyn LookupCache, server: Uuid) {
///     storage.invalidate_server(server).await;
/// }
/// ```
pub trait LookupCache: Send + Sync {
    /// Read a member lookup snapshot; a miss or expired entry returns None.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::LookupCache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &dyn LookupCache, server: Uuid, user: Uuid) {
    ///     let result = cache.get_member(server, user).await;
    /// }
    /// ```
    async fn get_member(&self, server: Uuid, user: Uuid) -> Option<MemberDetail>;
    /// Read a role lookup snapshot; a miss or expired entry returns None.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::LookupCache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &dyn LookupCache, server: Uuid) {
    ///     let result = cache.get_roles(server).await;
    /// }
    /// ```
    async fn get_roles(&self, server: Uuid) -> Option<Vec<Role>>;
    /// Store fresh evidence using the backend's TTL, capacity and failure policy.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{LookupCache, MemberDetail};
    /// use uuid::Uuid;
    /// async fn update(cache: &dyn LookupCache, server: Uuid, user: Uuid, member: &MemberDetail) {
    ///     cache.store_member(server, user, member).await;
    /// }
    /// ```
    async fn store_member(&self, server: Uuid, user: Uuid, member: &MemberDetail);
    /// Store fresh evidence using the backend's TTL, capacity and failure policy.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{LookupCache, Role};
    /// use uuid::Uuid;
    /// async fn update(cache: &dyn LookupCache, server: Uuid, roles: &[Role]) {
    ///     cache.store_roles(server, roles).await;
    /// }
    /// ```
    async fn store_roles(&self, server: Uuid, roles: &[Role]);
    /// Remove the selected member lookup.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::LookupCache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &dyn LookupCache, server: Uuid, user: Uuid) {
    ///     let result = cache.invalidate_member(server, user).await;
    /// }
    /// ```
    async fn invalidate_member(&self, server: Uuid, user: Uuid);
    /// Remove member and role lookups for a server.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::LookupCache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &dyn LookupCache, server: Uuid) {
    ///     let result = cache.invalidate_server(server).await;
    /// }
    /// ```
    async fn invalidate_server(&self, server: Uuid);
}

/// Bounds for each member/role lookup cache. The default disables lookup caching.
#[derive(Debug, Clone, Copy)]
/// # Example
///
/// ```rust
/// use rappin::CachePolicy;
/// use std::time::Duration;
/// let policy = CachePolicy::new(256, Duration::from_secs(30));
/// assert_eq!(policy.capacity, 256);
/// ```
pub struct CachePolicy {
    /// Maximum entries per lookup cache; zero disables storage.
    pub capacity: usize,
    /// Maximum lookup age; zero disables storage.
    pub ttl: Duration,
}
impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            capacity: 0,
            ttl: Duration::ZERO,
        }
    }
}
impl CachePolicy {
    /// Create explicit lookup bounds; either zero bound disables storage.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::CachePolicy;
    /// use std::time::Duration;
    /// let policy = CachePolicy::new(256, Duration::from_secs(30));
    /// assert_eq!(policy.capacity, 256);
    /// ```
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self { capacity, ttl }
    }
}
/// Explicitly choose a fresh request or an unexpired cached snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// # Example
///
/// ```rust
/// use rappin::Freshness;
/// assert_eq!(Freshness::default(), Freshness::Refresh);
/// let display_only = Freshness::PreferCache;
/// ```
pub enum Freshness {
    #[default]
    /// Bypass cached reads and fetch from Yapin; successful results may update the backend.
    Refresh,
    /// Use an unexpired cached result when available; otherwise fetch from Yapin.
    PreferCache,
}
#[derive(Debug, Clone)]
/// A cloned lookup value and its monotonic fetch age.
///
/// # Example
///
/// ```rust
/// use rappin::{Cached, MemberDetail};
/// fn display_age(entry: &Cached<MemberDetail>) {
///     println!("Member {} fetched {:?} ago", entry.value.member.user.id, entry.age());
/// }
/// ```
pub struct Cached<T> {
    /// Snapshot value at the recorded fetch time.
    pub value: T,
    fetched_at: Instant,
}
impl<T> Cached<T> {
    /// Elapsed monotonic time since this snapshot was inserted.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cached, MemberDetail};
    /// fn display_age(entry: &Cached<MemberDetail>) {
    ///     println!("Member {} fetched {:?} ago", entry.value.member.user.id, entry.age());
    /// }
    /// ```
    pub fn age(&self) -> Duration {
        self.fetched_at.elapsed()
    }
}
#[derive(Clone)]
struct Lookup<K, V> {
    policy: CachePolicy,
    values: Arc<tokio::sync::Mutex<HashMap<K, Cached<V>>>>,
}
impl<K, V> Default for Lookup<K, V> {
    fn default() -> Self {
        Self {
            policy: CachePolicy::default(),
            values: Arc::default(),
        }
    }
}
impl<K: Eq + Hash + Clone, V: Clone> Lookup<K, V> {
    fn new(policy: CachePolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }
    async fn get(&self, key: &K) -> Option<Cached<V>> {
        if self.policy.capacity == 0 || self.policy.ttl.is_zero() {
            return None;
        }
        let mut values = self.values.lock().await;
        if values.get(key).is_some_and(|v| v.age() >= self.policy.ttl) {
            values.remove(key);
            return None;
        }
        values.get(key).cloned()
    }
    async fn insert(&self, key: K, value: &V) {
        if self.policy.capacity == 0 || self.policy.ttl.is_zero() {
            return;
        }
        let mut values = self.values.lock().await;
        if !values.contains_key(&key) && values.len() >= self.policy.capacity {
            values.retain(|_, v| v.age() < self.policy.ttl);
        }
        if !values.contains_key(&key) && values.len() >= self.policy.capacity {
            if let Some(oldest) = values
                .iter()
                .min_by_key(|(_, v)| v.fetched_at)
                .map(|(k, _)| k.clone())
            {
                values.remove(&oldest);
            }
        }
        values.insert(
            key,
            Cached {
                value: value.clone(),
                fetched_at: Instant::now(),
            },
        );
    }
}

/// Thread-safe in-memory cache for Yapin' entities (servers, channels, bot profile).
#[derive(Clone, Default)]
/// # Example
///
/// ```rust
/// use rappin::Cache;
/// let cache = Cache::new();
/// let shared = cache.clone(); // Clones share entries.
/// ```
pub struct Cache {
    servers: Arc<RwLock<HashMap<Uuid, Server>>>,
    channels: Arc<RwLock<HashMap<Uuid, Channel>>>,
    me: Arc<RwLock<Option<BotProfile>>>,
    members: Lookup<(Uuid, Uuid), MemberDetail>,
    roles: Lookup<Uuid, Vec<Role>>,
}

impl Cache {
    /// Create shared entity storage; member and role caching requires a nonzero explicit policy.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// let cache = Cache::new();
    /// let shared = cache.clone(); // Clones share entries.
    /// ```
    pub fn new() -> Self {
        Self::default()
    }
    /// Lookup caching is disabled by default. Eviction is oldest insertion, with a hard capacity per cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, CachePolicy};
    /// use std::time::Duration;
    /// let cache = Cache::with_lookup_policy(CachePolicy::new(128, Duration::from_secs(10)));
    /// ```
    pub fn with_lookup_policy(policy: CachePolicy) -> Self {
        Self {
            members: Lookup::new(policy),
            roles: Lookup::new(policy),
            ..Self::default()
        }
    }
    /// Read a member lookup snapshot; a miss or expired entry returns None.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &Cache, server: Uuid, user: Uuid) {
    ///     let result = cache.get_member(server, user).await;
    /// }
    /// ```
    pub async fn get_member(&self, server: Uuid, user: Uuid) -> Option<Cached<MemberDetail>> {
        self.members.get(&(server, user)).await
    }
    /// Read a role lookup snapshot; a miss or expired entry returns None.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &Cache, server: Uuid) {
    ///     let result = cache.get_roles(server).await;
    /// }
    /// ```
    pub async fn get_roles(&self, server: Uuid) -> Option<Cached<Vec<Role>>> {
        self.roles.get(&server).await
    }
    pub(crate) async fn update_member(&self, server: Uuid, user: Uuid, value: &MemberDetail) {
        self.members.insert((server, user), value).await;
    }
    pub(crate) async fn update_roles(&self, server: Uuid, value: &Vec<Role>) {
        self.roles.insert(server, value).await;
    }
    /// Remove member and role lookups without removing entity entries.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &Cache, server: Uuid) {
    ///     let result = cache.invalidate_server_lookups(server).await;
    /// }
    /// ```
    pub async fn invalidate_server_lookups(&self, server: Uuid) {
        self.members
            .values
            .lock()
            .await
            .retain(|(s, _), _| *s != server);
        self.roles.values.lock().await.remove(&server);
    }
    /// Remove the selected member lookup.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// use uuid::Uuid;
    /// async fn use_cache(cache: &Cache, server: Uuid, user: Uuid) {
    ///     let result = cache.invalidate_member(server, user).await;
    /// }
    /// ```
    pub async fn invalidate_member(&self, server: Uuid, user: Uuid) {
        self.members.values.lock().await.remove(&(server, user));
    }

    /// Stores or updates a server in the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Server};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, server: Server) {
    ///     let result = cache.update_server(server).await;
    /// }
    /// ```
    pub async fn update_server(&self, server: Server) {
        self.servers.write().await.insert(server.id, server);
    }

    /// Stores or updates multiple servers in the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Server};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, servers: Vec<Server>) {
    ///     let result = cache.update_servers(servers).await;
    /// }
    /// ```
    pub async fn update_servers(&self, servers: impl IntoIterator<Item = Server>) {
        let mut guard = self.servers.write().await;
        for s in servers {
            guard.insert(s.id, s);
        }
    }

    /// Retrieves a server by ID from the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Server};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, id: Uuid) {
    ///     let result = cache.get_server(&id).await;
    /// }
    /// ```
    pub async fn get_server(&self, id: &Uuid) -> Option<Server> {
        self.servers.read().await.get(id).cloned()
    }

    /// Returns a list of all servers currently cached.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Server};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache) {
    ///     let result = cache.get_servers().await;
    /// }
    /// ```
    pub async fn get_servers(&self) -> Vec<Server> {
        self.servers.read().await.values().cloned().collect()
    }

    /// Removes a server from the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Server};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, id: Uuid) {
    ///     let result = cache.remove_server(&id).await;
    /// }
    /// ```
    pub async fn remove_server(&self, id: &Uuid) -> Option<Server> {
        self.servers.write().await.remove(id)
    }

    /// Stores or updates a channel in the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Channel};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, channel: Channel) {
    ///     let result = cache.update_channel(channel).await;
    /// }
    /// ```
    pub async fn update_channel(&self, channel: Channel) {
        self.channels.write().await.insert(channel.id, channel);
    }

    /// Stores or updates multiple channels in the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Channel};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, channels: Vec<Channel>) {
    ///     let result = cache.update_channels(channels).await;
    /// }
    /// ```
    pub async fn update_channels(&self, channels: impl IntoIterator<Item = Channel>) {
        let mut guard = self.channels.write().await;
        for ch in channels {
            guard.insert(ch.id, ch);
        }
    }

    /// Retrieves a channel by ID from the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Channel};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, id: Uuid) {
    ///     let result = cache.get_channel(&id).await;
    /// }
    /// ```
    pub async fn get_channel(&self, id: &Uuid) -> Option<Channel> {
        self.channels.read().await.get(id).cloned()
    }

    pub(crate) async fn invalidate_resource(&self, event: &crate::ResourceEvent) {
        let mut channels = self.channels.write().await;
        if event.kind.object() == "category" {
            channels.retain(|_, channel| channel.server_id != Some(event.server_id));
        } else {
            channels.remove(&event.id);
        }
    }

    /// Returns a list of all channels currently cached.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Channel};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache) {
    ///     let result = cache.get_channels().await;
    /// }
    /// ```
    pub async fn get_channels(&self) -> Vec<Channel> {
        self.channels.read().await.values().cloned().collect()
    }

    /// Returns all channels that belong to a specific server.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// use uuid::Uuid;
    /// async fn server_channels(cache: &Cache, server: Uuid) {
    ///     for channel in cache.get_channels_for_server(&server).await {
    ///         assert_eq!(channel.server_id, Some(server));
    ///     }
    /// }
    /// ```
    pub async fn get_channels_for_server(&self, server_id: &Uuid) -> Vec<Channel> {
        self.channels
            .read()
            .await
            .values()
            .filter(|c| c.server_id.as_ref() == Some(server_id))
            .cloned()
            .collect()
    }

    /// Removes a channel from the cache.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Cache, Channel};
    /// use uuid::Uuid;
    /// async fn local_entities(cache: &Cache, id: Uuid) {
    ///     let result = cache.remove_channel(&id).await;
    /// }
    /// ```
    pub async fn remove_channel(&self, id: &Uuid) -> Option<Channel> {
        self.channels.write().await.remove(id)
    }

    /// Stores the bot's own profile.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{BotProfile, Cache};
    /// async fn store_identity(cache: &Cache, profile: BotProfile) { cache.set_me(profile).await; }
    /// ```
    pub async fn set_me(&self, me: BotProfile) {
        *self.me.write().await = Some(me);
    }

    /// Retrieves the cached bot profile.
    /// # Example
    ///
    /// ```rust
    /// use rappin::Cache;
    /// async fn display_identity(cache: &Cache) {
    ///     if let Some(bot) = cache.get_me().await { println!("{}", bot.id); }
    /// }
    /// ```
    pub async fn get_me(&self) -> Option<BotProfile> {
        self.me.read().await.clone()
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    #[tokio::test]
    async fn ttl_and_capacity_preserve_unexpired_entries_without_read_scans() {
        let lookup = Lookup::new(CachePolicy::new(2, Duration::from_secs(60)));
        lookup.insert(1, &"old".to_owned()).await;
        lookup.insert(2, &"also old".to_owned()).await;
        for value in lookup.values.lock().await.values_mut() {
            value.fetched_at = Instant::now() - Duration::from_secs(61);
        }
        assert!(lookup.get(&1).await.is_none());
        assert_eq!(lookup.values.lock().await.len(), 1); // Only the queried key was removed.
        lookup.insert(3, &"new".to_owned()).await;
        lookup.insert(4, &"newer".to_owned()).await;
        assert_eq!(lookup.values.lock().await.len(), 2);
        assert!(lookup.get(&2).await.is_none());
        assert!(lookup.get(&3).await.is_some());
        assert!(lookup.get(&4).await.is_some());
    }
    #[tokio::test]
    async fn disabled_lookup_does_not_clone_values() {
        struct Counted(Arc<std::sync::atomic::AtomicUsize>);
        impl Clone for Counted {
            fn clone(&self) -> Self {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Self(self.0.clone())
            }
        }
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let lookup: Lookup<u8, Counted> = Lookup::default();
        lookup.insert(1, &Counted(count.clone())).await;
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(lookup.values.lock().await.is_empty());
    }
}
