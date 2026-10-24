//! # rappin
//!
//! An idiomatic, asynchronous Rust client and bot framework for the [Yapin'](https://www.yapin.gg) Bot API.
//!
//! ## Overview
//!
//! The `rappin` crate provides:
//! - **Complete API Coverage**: High-level and low-level methods for all 35 OpenAPI endpoints (messages, channels, servers, members, roles, categories, voice, bans, reactions, interactions).
//! - **Rich Message & Layout Builders**: Type-safe builders for `Embed` cards, `Button` rows, and Components V2 layouts (`Container`, `Section`, `TextDisplay`, etc.).
//! - **Bot Framework & Event Dispatcher**: Ergonomic command routing, prefix support (`!`, `/`), button click handlers, event listeners, and resilient long-polling with exponential backoff and rate limit handling.
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

pub mod bot;
pub mod client;
pub mod error;
pub mod models;
pub mod utils;

pub use bot::{
    Bot, BotBuilder, BotHandle, Cog, CogBuilder, CogButton, CogCommand, EventHandler,
    InteractionContext, MessageContext,
};
pub use client::{Client, ClientBuilder};
pub use error::{Result, YappinError};
pub use models::*;
pub use utils::{
    attachment_url, avatar_url, banner_url, icon_url, DEFAULT_API_BASE, DEFAULT_FILES_BASE,
    DEFAULT_SITE_BASE, IS_COMPONENTS_V2,
};

/// Commonly used types for building bots and interacting with Yapin'.
pub mod prelude {
    pub use crate::bot::{
        Bot, BotBuilder, BotHandle, Cog, CogBuilder, CogButton, CogCommand, EventHandler,
        InteractionContext, MessageContext,
    };
    pub use crate::client::{Client, ClientBuilder};
    pub use crate::error::{Result, YappinError};
    pub use crate::models::component::{
        ActionRow, Button, ButtonStyle, Component, Container, FileDisplay, LayoutComponent,
        MediaGallery, MediaItem, Section, Separator, TextDisplay, ThumbnailComponent,
    };
    pub use crate::models::embed::{Embed, EmbedAuthor, EmbedColor, EmbedField, EmbedFooter};
    pub use crate::models::message::{CreateMessage, EditMessage, Message};
    pub use crate::models::server::{Category, Member, Role, Server};
    pub use crate::models::user::{BotProfile, BotStatus, User};
    pub use crate::utils::IS_COMPONENTS_V2;
}
