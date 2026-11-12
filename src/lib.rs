//! # rappin-rs
//!
//! An idiomatic, asynchronous Rust client and bot framework for the [Yapin'](https://www.yapin.gg) Bot API.
//!
//! ## Overview
//!
//! The `rappin` crate provides:
//! - **Yapin API client**: Typed methods for common operations and a low-level request API for platform endpoints.
//! - **Rich Message & Layout Builders**: Type-safe builders for `Embed` cards, `Button` rows, and Components V2 layouts (`Container`, `Section`, `TextDisplay`, etc.).
//! - **Bot Framework & Event Dispatcher**: Command routing, configurable prefixes, interaction handlers, ordered checkpoints, and polling or optional gateway intake. Bounded availability retries are opt-in; incomplete handler work is never retried automatically.
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
//!         .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
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
//!     let token = std::env::var("YAPPER_BOT_TOKEN")
//!         .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
//!     let client = Client::new(token);
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
mod reload_failure;
pub use reload_failure::{
    CompilerDiagnostic, CompilerSpan, HotReloadFailure, ProcessExit, ReloadPhase,
};
pub mod security;
pub mod utils;
pub mod validation;
#[cfg(feature = "voice")]
pub mod voice;
pub mod webhook;
#[cfg(feature = "webhook-middleware")]
pub mod webhook_middleware;

pub mod availability;
pub use availability::{
    AvailabilityEnd, AvailabilityFailure, AvailabilityPhase, AvailabilityPolicy, AvailabilityState,
    AvailabilityStatus,
};
pub mod api;
pub mod resource_api;
pub use resource_api::{ComputedPermissionsPolicy, ResourceApiExt};
pub mod capabilities;
pub mod lookup;
pub use lookup::{LookupResult, LookupStatus, SearchStop};
#[cfg(any(feature = "metrics", feature = "metrics-http"))]
pub mod metrics;
pub mod permissions;
pub mod role_hierarchy;
pub use api::{Api, ApiExt};
pub use bot::{
    canonical_event_fingerprint, prepare_event_retirement, prepare_timestamp_reconciliation,
    Admission, AdmissionDurability, AdmissionStatus, ConflictDisposition, EventIngress,
    EventReceipt, EventRetirement, EventRetirementAssessment, EventRetirementEvidence,
    EventRetirementOptions, EventRetirementRequest, EventRetirementState, IngressAdmissionPolicy,
    IngressConfig, IngressConflict, IngressConflictPolicy, IngressRecord, IngressRecovery,
    IngressSource, IngressStatus, IngressStore, PayloadDifference, PayloadType,
    PreparedEventRetirement, PreparedTimestampReconciliation, ReceiptStage, ReceiptStatus,
    TimestampReconciliation, TimestampReconciliationOptions, TimestampReconciliationRequest,
    TimestampReconciliationState,
};
pub use bot::{
    EventFailure, EventProcessingFailure, FailureClass, FailureDisposition, HandlerFailure,
    HandlerKind, MessageDisposition, MessageFailure, MessageStage,
};
pub use bot::{
    InvocationOutcome, InvocationStatus, OperationId, PreparationReport, PreparedInvocation,
    MAX_INVOCATION_DEPTH,
};
#[cfg(any(feature = "metrics", feature = "metrics-http"))]
pub use metrics::{
    install_prometheus, install_prometheus_with_config, record_availability_status,
    record_request_metrics, record_request_observation, record_transport_status,
};
pub use permissions::{PermissionEvaluation, PermissionSnapshot};
pub use role_hierarchy::{
    execute_role_deletion, RoleHierarchyAssessment, RoleHierarchyPolicy, RoleHierarchyRules,
};

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
#[cfg(feature = "webhook-middleware")]
pub use webhook_middleware::{
    get_verified_event, route, verified_handler, verify_middleware, WebhookConfig,
};

/// Commonly used types for building bots and interacting with Yapin'.
pub mod prelude {
    pub use crate::bot::{
        Bot, BotBuilder, BotHandle, Cog, CogBuilder, CogButton, CogCommand, EventFailure,
        EventHandler, EventProcessingFailure, FailureClass, FailureDisposition, HandlerFailure,
        HandlerKind, InteractionContext, MessageContext, MessageDisposition, MessageFailure,
        MessageStage, RegistrationPlan,
    };
    pub use crate::client::{Client, ClientBuilder};
    pub use crate::cooldown::{Cooldown, CooldownType};
    pub use crate::error::{Result, YappinError};
    pub use crate::events::{EventConsumer, EventCursor};
    #[cfg(feature = "hot-reload")]
    pub use crate::hot_reload::{HotReload, Listener, WorkerCog, WorkerContext, WorkerProject};
    #[cfg(any(feature = "metrics", feature = "metrics-http"))]
    pub use crate::metrics::{
        install_prometheus, install_prometheus_with_config, record_availability_status,
        record_request_metrics, record_request_observation, record_transport_status,
    };
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
    #[cfg(feature = "webhook-middleware")]
    pub use crate::webhook_middleware::{
        get_verified_event, route, verified_handler, verify_middleware, WebhookConfig,
    };
    pub use crate::{
        check_fn, Api, ApiExt, ArgumentInfo, ArgumentSchema, ArgumentType, Arguments, BoundedU32,
        Cache, CachePolicy, Cached, ChannelId, Check, Collector, CommandCheck, CommandError,
        CommandSchema, CommandSpec, CommandValues, DynamicCommandSpec, EventFilter, EventHub,
        EventTransport, Freshness, FromArgs, HumanDuration, InteractionState, LookupCache,
        Permission, PermissionEvaluation, PermissionSnapshot, ResponsePhase, TransportMode,
        TransportStatus, UiApiExt, UiInteractionContext, UserId, WaitError,
    };
    pub use crate::{
        ApiObject, CategoryId, EmojiId, MemberPermissions, MessageId, ResolvedObject,
        ResourceEvent, ResourceEventKind, ResourceOverride, RoleId, RolePosition, ScheduledEventId,
        ServerId, StickerId, StickerMessage, ThreadId, WebhookId,
    };
    pub use crate::{ComputedPermissionsPolicy, ResourceApiExt};
}

pub use events::{EventConsumer, EventCursor};
pub use validation::split_message;

#[cfg(feature = "hot-reload")]
pub use hot_reload::{CogEvent, HotReload, Listener, WorkerCog, WorkerContext, WorkerProject};

#[doc = include_str!("../docs/transport-and-cache.md")]
pub mod transport_and_cache_guide {}

#[doc = include_str!("../docs/server-management.md")]
pub mod server_management_guide {}

#[doc = include_str!("../docs/availability-and-hierarchy.md")]
pub mod availability_and_hierarchy_guide {}

#[doc = include_str!("../docs/sdk-workflows.md")]
pub mod sdk_workflows_guide {}

#[doc = include_str!("../docs/capability-bindings.md")]
pub mod capability_bindings_guide {}

#[doc = include_str!("../docs/handler-failures.md")]
pub mod handler_failures_guide {}

#[doc = include_str!("../docs/api-1.2.md")]
pub mod api_1_2_guide {}

/// Shared server-message gates, fallible listeners and explicit completion.
#[doc = include_str!("../docs/message-policy.md")]
pub mod message_policy_guide {}

pub mod ui_api;
pub use ui_api::{UiApiExt, UiInteractionContext};

#[doc = include_str!("../docs/api-1.3.md")]
pub mod api_1_3_guide {}

#[doc = include_str!("../docs/api-1.4.md")]
pub mod api_1_4_guide {}
