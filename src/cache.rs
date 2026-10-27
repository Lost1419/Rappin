use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::{BotProfile, Channel, Server};
use crate::models::{MemberDetail, Role};
use std::{hash::Hash, time::Duration};
use tokio::time::Instant;

/// Bounds for each member/role lookup cache. The default disables lookup caching.
#[derive(Debug, Clone, Copy)]
pub struct CachePolicy {
    pub capacity: usize,
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
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self { capacity, ttl }
    }
}
/// Explicitly choose a fresh request or an unexpired cached snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Freshness {
    #[default]
    Refresh,
    PreferCache,
}
#[derive(Debug, Clone)]
pub struct Cached<T> {
    pub value: T,
    fetched_at: Instant,
}
impl<T> Cached<T> {
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
        let mut values = self.values.lock().await;
        values.retain(|_, v| v.age() < self.policy.ttl);
        values.get(key).cloned()
    }
    async fn insert(&self, key: K, value: V) {
        if self.policy.capacity == 0 || self.policy.ttl.is_zero() {
            return;
        }
        let mut values = self.values.lock().await;
        values.retain(|_, v| v.age() < self.policy.ttl);
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
                value,
                fetched_at: Instant::now(),
            },
        );
    }
}

/// Thread-safe in-memory cache for Yapin' entities (servers, channels, bot profile).
#[derive(Clone, Default)]
pub struct Cache {
    servers: Arc<RwLock<HashMap<Uuid, Server>>>,
    channels: Arc<RwLock<HashMap<Uuid, Channel>>>,
    me: Arc<RwLock<Option<BotProfile>>>,
    members: Lookup<(Uuid, Uuid), MemberDetail>,
    roles: Lookup<Uuid, Vec<Role>>,
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }
    /// Lookup caching is disabled by default. Eviction is oldest insertion, with a hard capacity per cache.
    pub fn with_lookup_policy(policy: CachePolicy) -> Self {
        Self {
            members: Lookup::new(policy),
            roles: Lookup::new(policy),
            ..Self::default()
        }
    }
    pub async fn get_member(&self, server: Uuid, user: Uuid) -> Option<Cached<MemberDetail>> {
        self.members.get(&(server, user)).await
    }
    pub async fn get_roles(&self, server: Uuid) -> Option<Cached<Vec<Role>>> {
        self.roles.get(&server).await
    }
    pub(crate) async fn update_member(&self, server: Uuid, user: Uuid, value: MemberDetail) {
        self.members.insert((server, user), value).await;
    }
    pub(crate) async fn update_roles(&self, server: Uuid, value: Vec<Role>) {
        self.roles.insert(server, value).await;
    }
    pub async fn invalidate_server_lookups(&self, server: Uuid) {
        self.members
            .values
            .lock()
            .await
            .retain(|(s, _), _| *s != server);
        self.roles.values.lock().await.remove(&server);
    }
    pub async fn invalidate_member(&self, server: Uuid, user: Uuid) {
        self.members.values.lock().await.remove(&(server, user));
    }

    /// Stores or updates a server in the cache.
    pub async fn update_server(&self, server: Server) {
        self.servers.write().await.insert(server.id, server);
    }

    /// Stores or updates multiple servers in the cache.
    pub async fn update_servers(&self, servers: impl IntoIterator<Item = Server>) {
        let mut guard = self.servers.write().await;
        for s in servers {
            guard.insert(s.id, s);
        }
    }

    /// Retrieves a server by ID from the cache.
    pub async fn get_server(&self, id: &Uuid) -> Option<Server> {
        self.servers.read().await.get(id).cloned()
    }

    /// Returns a list of all servers currently cached.
    pub async fn get_servers(&self) -> Vec<Server> {
        self.servers.read().await.values().cloned().collect()
    }

    /// Removes a server from the cache.
    pub async fn remove_server(&self, id: &Uuid) -> Option<Server> {
        self.servers.write().await.remove(id)
    }

    /// Stores or updates a channel in the cache.
    pub async fn update_channel(&self, channel: Channel) {
        self.channels.write().await.insert(channel.id, channel);
    }

    /// Stores or updates multiple channels in the cache.
    pub async fn update_channels(&self, channels: impl IntoIterator<Item = Channel>) {
        let mut guard = self.channels.write().await;
        for ch in channels {
            guard.insert(ch.id, ch);
        }
    }

    /// Retrieves a channel by ID from the cache.
    pub async fn get_channel(&self, id: &Uuid) -> Option<Channel> {
        self.channels.read().await.get(id).cloned()
    }

    /// Returns a list of all channels currently cached.
    pub async fn get_channels(&self) -> Vec<Channel> {
        self.channels.read().await.values().cloned().collect()
    }

    /// Returns all channels that belong to a specific server.
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
    pub async fn remove_channel(&self, id: &Uuid) -> Option<Channel> {
        self.channels.write().await.remove(id)
    }

    /// Stores the bot's own profile.
    pub async fn set_me(&self, me: BotProfile) {
        *self.me.write().await = Some(me);
    }

    /// Retrieves the cached bot profile.
    pub async fn get_me(&self) -> Option<BotProfile> {
        self.me.read().await.clone()
    }
}
