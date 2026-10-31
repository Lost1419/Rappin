//! # rappin-rs
//!
//! An idiomatic, asynchronous Rust client and bot framework for the [Yapin'](https://www.yapin.gg) Bot API.
//!
//! ## Overview
//!
//! The `rappin` crate provides:
//! - **Complete API Coverage**: High-level and low-level methods for all 35 OpenAPI endpoints (messages, channels, servers, members, roles, categories, voice, bans, reactions, interactions).
//! - **Rich Message & Layout Builders**: Type-safe builders for `Embed` cards, `Button` rows, and Components V2 layouts (`Container`, `Section`, `TextDisplay`, etc.).
//! - **Bot Framework & Event Dispatcher**: Ergonomic command routing, prefix support (`!`, `/`), button click handlers, event listeners, and resilient long-polling with exponential backoff and rate limit handling.
//! - **Modular Cogs System**: Add and replace cog instances at runtime. Optional `hot-reload` compiles and swaps edited Rust worker cogs while the main bot keeps running.
//! - **Custom Lookup Caches**: Optional asynchronous member/role storage through `LookupCache`, including application-owned databases.
//! - **In-Memory Cache Layer**: Thread-safe caching of bot profile, servers, and channels with zero network calls for repetitive lookups.
//! - **Command Cooldowns**: Declarative rate limiting per user, channel, server, or globally.
//! - **Interactive Paginator & Confirm Dialog**: Pre-built UI components for paging through embeds and binary confirmation prompts.
//! - **Standalone Webhooks & OAuth2**: Lightweight `WebhookClient` for alerting/CI notifications and `OAuth2Client` for "Login with Yapin'".
//! - **Asset CDN Helpers**: URL resolvers for avatars, banners, icons, and attachments (`https://files.yapper.website/...`).
//!
//! ## Quickstart: Building a Bot
//!
//! ```no_run
//! use rappin::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let token = std::env::var("YAPPER_BOT_TOKEN")
//!         .unwrap_or_else(|_| "yb_sample_token".to_string());
//!
//!     let bot = Bot::builder(token)
//!         .prefix("!")
//!         .prefix("/")
//!         .command("ping", "Check if the bot is alive", |ctx| async move {
//!             ctx.reply("pong 🏓").await?;
//!             Ok(())
//!         })
//!         .command("roll", "Roll a die, like /roll 20", |ctx| async move {
//!             let sides = ctx
//!                 .argv
//!                 .get(0)
//!                 .and_then(|s| s.parse::<u32>().ok())
//!                 .unwrap_or(6);
//!             ctx.reply(format!("🎲 Rolled a d{sides}!")).await?;
//!             Ok(())
//!         })
//!         .button("vote:", |ctx| async move {
//!             let choice = ctx.custom_id().strip_prefix("vote:").unwrap_or("unknown");
//!             ctx.ephemeral(format!("You voted for {choice}!")).await?;
//!             Ok(())
//!         })
//!         .build();
//!
//!     bot.run().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Quickstart: Direct REST Client Usage
//!
//! ```no_run
//! use rappin::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let client = Client::new("yb_your_token_here");
//!
//!     let me = client.get_me().await?;
//!     println!("Logged in as: {} ({})", me.username, me.id);
//!
//!     let servers = client.get_servers().await?;
//!     for server in servers {
//!         println!("Server: {} (ID: {})", server.name, server.id);
//!     }
//!
//!     Ok(())
//! }
//! ```

/// Rust-native command, collector, transport, and caching workflows.
#[doc = include_str!("../docs/rust-bots.md")]
pub mod guide {}

pub mod bot;
pub mod cache;
pub mod client;
pub mod collectors;
pub mod commands;
pub mod cooldown;
pub mod diagnostics;
pub mod error;
pub mod events;
mod failover;
#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "hot-reload")]
pub mod hot_reload;
pub mod interactions;
pub mod models;
pub mod oauth2;
pub mod paginator;
mod rate_limits;
pub mod security;
pub mod utils;
pub mod validation;
#[cfg(feature = "voice")]
pub mod voice;
pub mod webhook;

pub mod api;
pub mod lookup;
pub use lookup::{LookupResult, LookupStatus, SearchStop};
pub mod permissions;
pub use api::{Api, ApiExt};
pub use permissions::{PermissionEvaluation, PermissionSnapshot};

pub use bot::{
    Bot, BotBuilder, BotHandle, Cog, CogBuilder, CogButton, CogCommand, EventHandler,
    InteractionContext, MessageContext, RegistrationPlan,
};
pub use cache::{Cache, CachePolicy, Cached, Freshness, LookupCache};
pub use client::{Client, ClientBuilder};
pub use collectors::{Collectible, Collector, CollectorBuilder, EventFilter, EventHub, WaitError};
pub use commands::{
    check_fn, ArgumentInfo, ArgumentSchema, ArgumentType, Arguments, BoundedU32, ChannelId, Check,
    CommandCheck, CommandError, CommandInfo, CommandSchema, CommandSpec, CommandValues,
    DynamicCommandSpec, FromArgs, HumanDuration, Permission, RegisteredCommand, UserId,
};
pub use cooldown::{Cooldown, CooldownType};
pub use diagnostics::{
    EventTransport, FallbackReason, RequestEvent, RequestMetrics, RequestObservation,
    TransportMode, TransportStatus,
};
pub use error::{RemoteError, Result, YappinError};
pub use interactions::{InteractionState, ResponsePhase};
pub use models::*;
pub use oauth2::{OAuth2Client, OAuth2Token, OAuth2User, Pkce};
pub use paginator::{ConfirmDialog, Paginator};
pub use utils::{
    attachment_url, avatar_url, banner_url, icon_url, DEFAULT_API_BASE, DEFAULT_FILES_BASE,
    DEFAULT_SITE_BASE, IS_COMPONENTS_V2, LEGACY_API_BASE,
};
pub use webhook::{WebhookClient, WebhookInfo, WebhookMessage};

/// Commonly used types for building bots and interacting with Yapin'.
pub mod prelude {
    pub use crate::bot::{
        Bot, BotBuilder, BotHandle, Cog, CogBuilder, CogButton, CogCommand, EventHandler,
        InteractionContext, MessageContext, RegistrationPlan,
    };
    pub use crate::client::{Client, ClientBuilder};
    pub use crate::cooldown::{Cooldown, CooldownType};
    pub use crate::error::{Result, YappinError};
    pub use crate::events::{EventConsumer, EventCursor};
    #[cfg(feature = "hot-reload")]
    pub use crate::hot_reload::{HotReload, Listener, WorkerCog, WorkerContext, WorkerProject};
    pub use crate::models::component::{
        ActionRow, Button, ButtonStyle, Component, Container, FileDisplay, LayoutComponent,
        MediaGallery, MediaItem, Section, Separator, TextDisplay, ThumbnailComponent,
    };
    pub use crate::models::embed::{Embed, EmbedAuthor, EmbedColor, EmbedField, EmbedFooter};
    pub use crate::models::message::{CreateMessage, EditMessage, Message};
    pub use crate::models::server::{Category, Member, Role, Server};
    pub use crate::models::user::{BotProfile, BotStatus, User};
    pub use crate::oauth2::{OAuth2Client, OAuth2Token, OAuth2User, Pkce};
    pub use crate::paginator::{ConfirmDialog, Paginator};
    pub use crate::utils::IS_COMPONENTS_V2;
    pub use crate::validation::split_message;
    pub use crate::webhook::{WebhookClient, WebhookInfo, WebhookMessage};
    pub use crate::{
        check_fn, Api, ApiExt, ArgumentInfo, ArgumentSchema, ArgumentType, Arguments, BoundedU32,
        Cache, CachePolicy, Cached, ChannelId, Check, Collector, CommandCheck, CommandError,
        CommandSchema, CommandSpec, CommandValues, DynamicCommandSpec, EventFilter, EventHub,
        EventTransport, Freshness, FromArgs, HumanDuration, InteractionState, LookupCache,
        Permission, PermissionEvaluation, PermissionSnapshot, ResponsePhase, TransportMode,
        TransportStatus, UserId, WaitError,
    };
}

pub use events::{EventConsumer, EventCursor};
pub use validation::split_message;

#[cfg(feature = "hot-reload")]
pub use hot_reload::{CogEvent, HotReload, Listener, WorkerCog, WorkerContext, WorkerProject};

#[doc = include_str!("../docs/transport-and-cache.md")]
pub mod transport_and_cache_guide {}

#[doc = include_str!("../docs/server-management.md")]
pub mod server_management_guide {}

#[doc = include_str!("../docs/sdk-workflows.md")]
pub mod sdk_workflows_guide {}
