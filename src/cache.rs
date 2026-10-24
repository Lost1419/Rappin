use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::{BotProfile, Channel, Server};

/// Thread-safe in-memory cache for Yapin' entities (servers, channels, bot profile).
#[derive(Clone, Default)]
pub struct Cache {
    servers: Arc<RwLock<HashMap<Uuid, Server>>>,
    channels: Arc<RwLock<HashMap<Uuid, Channel>>>,
    me: Arc<RwLock<Option<BotProfile>>>,
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
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
