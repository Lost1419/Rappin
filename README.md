# Rappin 🦀

[![GitHub](https://img.shields.io/badge/github-Lost1419/Rappin-blue?logo=github)](https://github.com/Lost1419/Rappin)
[![Crates.io](https://img.shields.io/crates/v/rappin-rs.svg?v=0.1.5)](https://crates.io/crates/rappin-rs)
[![Docs.rs](https://docs.rs/rappin-rs/badge.svg)](https://docs.rs/rappin-rs)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

An idiomatic, type-safe, asynchronous Rust client and bot framework for the [Yapin’](https://www.yapin.gg) Bot API.

Yapin’ is a chat platform providing an HTTPS-based bot API with long-polling events, slash commands, interactive buttons, embeds, Components V2 cards, and LiveKit-powered voice channels.

See [credential security](docs/agents.md#credential-hardening-014) for the security boundaries and migration notes.

## Agent CLI, MCP, and resumable bots

The optional `rappin` binary lets agents use Yapin without writing a Rust program:

```sh
cargo install --path . --features cli,gateway,voice
export YAPPER_BOT_TOKEN="yb_your_token"
rappin --json doctor
rappin servers list
rappin channels resolve general --server "$SERVER_ID"
rappin messages list "$CHANNEL_ID" --limit 25
rappin --dry-run messages send "$CHANNEL_ID" --content "Hello from an agent"
rappin messages send "$CHANNEL_ID" --content "Hello from an agent"
```

All output is JSON, writes support `--dry-run`, and JSON bodies can come from stdin.
`rappin mcp` exposes the same discovery, history, message, and event workflows as
stdio MCP tools. `rappin schema` prints the bundled official OpenAPI contract.
See the [agent guide](docs/agents.md) for commands, auth/config, permissions,
pagination, explicit event acknowledgement, voice setup, and compatibility changes.
A reusable [agent skill](skills/rappin/SKILL.md) is included.

Library users can opt into durable checkpoints and bounded handlers:

```rust,no_run
use rappin::prelude::*;
let bot = Bot::builder("yb_token")
    .cursor_file("./state/events.json")
    .handler_concurrency(8)
    .build();
```

Enable Cargo feature `gateway` and add
`.gateway(rappin::gateway::DEFAULT_GATEWAY)` for WebSocket events with automatic
resume, heartbeats, and polling fallback. Feature `voice` adds `Client::play_audio`
using the official SDK's LiveKit encryption via an optional Node/ffmpeg bridge.
The default library build requires neither CLI dependencies nor Node.

---

## 📖 How It Works

Rappin is designed around modern asynchronous Rust with `tokio` and `reqwest`:

```
┌────────────────────────────────────────────────────────────────┐
│                       Yapin’ Platform                          │
│                   https://www.yapin.gg                         │
└───────────────▲───────────────────────────────┬────────────────┘
                │ HTTPS POST/PATCH/DELETE        │ GET /events (long poll)
                │ (messages, commands, servers) │ (up to 100 events/call)
┌───────────────┴───────────────────────────────▼────────────────┐
│                        Rappin Engine                           │
│                                                                │
│  ┌────────────────────┐          ┌───────────────────────────┐ │
│  │    REST Client     │          │      Event Dispatcher     │ │
│  │ (35 endpoints with │          │    (Deduplication &       │ │
│  │ rate-limit retry)  │          │    Auto Reconnection)     │ │
│  └────────────────────┘          └─────────────┬─────────────┘ │
│                                                │               │
│                        ┌───────────────────────┴─────────────┐ │
│                        ▼                                     │ │
│           ┌────────────────────────┐  ┌──────────────────┐   │ │
│           │ Slash & Prefix Router  │  │  Button Callback │   │ │
│           │   (/ping, !roll, ...)  │  │   (vote:yes, ...)│   │ │
│           └───────────▲────────────┘  └────────▲─────────┘   │ │
│                       │                        │             │ │
│                 ┌─────┴────────────────────────┴─────┐       │ │
│                 │   Dynamic Cogs & Hot-Reloading     │       │ │
│                 │ (Add/remove/update while running)  │       │ │
│                 └────────────────────────────────────┘       │ │
└────────────────────────────────────────────────────────────────┘
```

1. **Authentication**: Bots authenticate using `Authorization: Bot <token>` generated in Yapin’ Settings → Developer.
2. **Command Synchronization**: When you register commands (e.g. `bot.command("ping", ...)` or via a `Cog`), Rappin automatically registers them with Yapin’ (`PUT /commands`), making them show up in the `/` slash menu for all server members.
3. **Long Polling Loop**: Instead of requiring open firewall ports or webhooks, bots fetch their own events with `GET /events?after=<last>&wait=25`. Rappin transparently handles connection drops, exponential backoff, event deduplication, and HTTP 429 (`Retry-After`) / HTTP 503 (`busy`) rate limits.
4. **Context-Driven Handlers**: Every command handler receives a `MessageContext` that makes replying trivial (`ctx.reply("pong")`, `ctx.author_mention()`, `ctx.argv`). Every button click receives an `InteractionContext` (`ctx.reply()`, `ctx.ephemeral()`, `ctx.update()`).

---

## ✨ Features

- **Complete REST Coverage (35 paths, 51 HTTP operations)**:
  - Bot profile & presence (`/me`, `/me/status`, `/me/avatar`, `/me/banner`)
  - Channels & messages (send, edit, delete, bulk delete, pin, reactions)
  - Server management (members, roles, kicks, bans, unbans, timeouts, categories)
  - Button interactions (`reply`, `ephemeral`, `update`, `ack`)
  - Voice channels (obtain LiveKit tickets with E2EE metadata; optional encrypted playback)
  - Direct messages (`/users/{id}/dm`)
- **Dual Mode**: Use either the declarative `Bot` long-polling engine or the standalone `Client` for REST scripting.
- **Cogs & Rust Code Reloading**: Replace native cog instances at runtime, or enable `hot-reload` to compile and swap edited Rust worker cogs while the main bot keeps running. See the [hot-reloading guide](docs/hot-reloading.md).
- **In-Memory Cache**: Built-in thread-safe cache for bot profile, servers, and channels with zero network latency for repeated reads.
- **Command Cooldowns**: Declarative rate limiting per user, channel, server, or globally.
- **Interactive Paginator & Confirm Dialog**: Pre-built UI navigation components for browsing through multi-page embeds and binary yes/no confirmation prompts.
- **Standalone Webhook & OAuth2 Clients**: Lightweight `WebhookClient` for CI/CD alerts and `OAuth2Client` for "Login with Yapin'".
- **Components V2 Support**: Full support for Yapin's modern layout system (`Container`, `Section`, `TextDisplay`, `Thumbnail`, `MediaGallery`, `Separator`, `FileDisplay`).
- **CDN Helpers**: Helper methods to resolve avatar, banner, icon, and attachment paths to their complete CDN URLs (`https://files.yapper.website/...`).

---

## 📦 Installation

Add `rappin-rs` and `tokio` to your `Cargo.toml`:

```toml
[dependencies]
rappin-rs = "0.1.5"
tokio = { version = "1", features = ["full"] }
```

Or via Cargo:

```bash
cargo add rappin-rs
```

---

## 🚀 Quick Start: Creating a Bot

```rust
use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .expect("Please set YAPPER_BOT_TOKEN environment variable");

    let bot = Bot::builder(token)
        .prefix("!")
        .prefix("/")
        // 1. Simple slash or prefix command
        .command("ping", "Check if bot is alive", |ctx| async move {
            ctx.reply("pong 🏓").await?;
            Ok(())
        })
        // 2. Command with parsed arguments: /roll 20
        .command("roll", "Roll a die, like /roll 20", |ctx| async move {
            let sides = ctx
                .argv
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(6);
            ctx.reply(format!("🎲 Rolled a d{sides}!")).await?;
            Ok(())
        })
        // 3. Command sending interactive buttons
        .command("menu", "Display an interactive menu", |ctx| async move {
            let buttons = ActionRow::new(vec![
                Button::success("Apples", "vote:apples").with_emoji("🍎"),
                Button::primary("Oranges", "vote:oranges").with_emoji("🍊"),
                Button::link("Website", "https://www.yapin.gg"),
            ]);
            let msg = CreateMessage::reply("Pick your favorite fruit:", ctx.message_id())
                .action_row(buttons);
            ctx.reply_with(msg).await?;
            Ok(())
        })
        // 4. Handle button clicks matching the "vote:" prefix
        .button("vote:", |ctx| async move {
            let choice = ctx.custom_id().strip_prefix("vote:").unwrap_or("unknown");
            ctx.ephemeral(format!("You voted for **{choice}**!")).await?;
            Ok(())
        })
        .build();

    println!("Starting bot... Press Ctrl+C to exit.");
    bot.run().await?;
    Ok(())
}
```

---

## 🧩 Cogs & Rust Hot-Reloading

Cogs group commands, button interactions, and listeners into modules. Native `Cog` instances use code compiled into the main bot. The optional `hot-reload` feature loads separate Rust worker binaries, so edited source can be rebuilt and activated **without restarting the main bot**.

### Reload edited Rust source

Available since `0.1.5` with Cargo feature `hot-reload`. Try the example from this checkout:

```sh
export YAPPER_BOT_TOKEN="yb_your_token"
cargo run --features hot-reload --example hot_reload_bot
```

Edit the reply in [`examples/hot_cog_worker.rs`](examples/hot_cog_worker.rs) and save. The watcher builds and starts a candidate worker while the old one serves requests, then swaps its routes. Existing requests finish on the old worker. A failed build or registration keeps the working version installed.

Worker handlers stay in Rust:

```rust,ignore
WorkerCog::new("fun")
    .command("ping", "Check the bot", |ctx| async move {
        ctx.reply("Pong!").await?;
        Ok(())
    })
    .run().await?;
```

The parent retains the bot token and performs authenticated API calls for the worker. Worker memory resets on reload; keep durable state in a database or file. See the [complete setup and operating limits](docs/hot-reloading.md).

### 1. Creating a Cog

Implement the `Cog` trait:

```rust
use async_trait::async_trait;
use rappin::prelude::*;

pub struct ModerationCog;

#[async_trait]
impl Cog for ModerationCog {
    fn name(&self) -> &str {
        "moderation"
    }

    // Optional guard: return false to abort execution (e.g. check permissions)
    async fn check(&self, _ctx: &MessageContext) -> bool {
        true
    }

    fn commands(&self) -> Vec<CogCommand> {
        vec![
            CogCommand::new("kick", "Kick a user", |ctx| async move {
                ctx.reply("User kicked.").await?;
                Ok(())
            }),
            CogCommand::new("timeout", "Timeout a user", |ctx| async move {
                ctx.reply("User timed out.").await?;
                Ok(())
            }),
        ]
    }
}
```

Or build one dynamically with `CogBuilder`:

```rust
let fun_cog = CogBuilder::new("fun")
    .command("joke", "Tell a joke", |ctx| async move {
        ctx.reply("Why do programmers prefer dark mode? Because light attracts bugs!").await?;
        Ok(())
    })
    .build();
```

### 2. Replacing Native Cog Instances

`BotHandle` (accessible through `bot.handle()` or `ctx.bot()`) can replace native cog instances at runtime. `reload_cog` uses already linked code; use `HotReload` above to activate source edits.

```rust
let bot = Bot::builder(token)
    .cog(ModerationCog)
    .command("reload", "Reload a cog live", |ctx| async move {
        let owner = ctx.cache().get_me().await.and_then(|me| me.bot).map(|bot| bot.owner_id);
        if owner != Some(ctx.author_id()) {
            ctx.reply("Only the bot owner can reload cogs.").await?;
            return Ok(());
        }
        if let Some(bot) = ctx.bot() {
            bot.reload_cog(ModerationCog).await?;
            ctx.reply("✅ Moderation cog reloaded live!").await?;
        }
        Ok(())
    })
    .build();

bot.run().await?;
```

While the bot is running, cog changes synchronize the command list with Yapin' (`PUT /commands`) before publishing local routes, unless command synchronization is disabled. A registration error preserves the previous local routes.

---

## ⏱️ Command Cooldowns & Rate Limiting

Easily protect your commands from spam with built-in cooldowns:

```rust
use std::time::Duration;
use rappin::prelude::*;

let bot = Bot::builder(token)
    // Cooldown per-user (e.g. daily reward, generative AI prompt)
    .command_with_cooldown(
        "daily",
        "Claim your daily reward",
        Cooldown::user(Duration::from_secs(86400)),
        |ctx| async move {
            ctx.reply("🎁 Claimed daily coins!").await?;
            Ok(())
        },
    )
    // Cooldown per-channel or per-server
    .command_with_cooldown(
        "trivia",
        "Start a trivia game in this channel",
        Cooldown::channel(Duration::from_secs(30)),
        |ctx| async move {
            ctx.reply("🧠 Trivia started!").await?;
            Ok(())
        },
    )
    .build();
```

---

## 🛡️ Global Error Handling

Easily catch and handle command and button failures across your entire bot:

```rust
let bot = Bot::builder(token)
    // Custom command error handler
    .on_command_error(|ctx, err| async move {
        if err.is_not_found() {
            let _ = ctx.reply("❌ The requested item was not found.").await;
        } else if let Some(wait) = err.retry_after() {
            let _ = ctx.reply(format!("⏳ Rate limited! Please wait {:.1}s.", wait.as_secs_f32())).await;
        } else {
            let _ = ctx.reply(format!("⚠️ An error occurred: {err}")).await;
        }
    })
    // Custom button click error handler
    .on_button_error(|ctx, err| async move {
        let _ = ctx.ephemeral(format!("⚠️ Failed to process button: {err}")).await;
    })
    .build();
```

---

## ⚡ In-Memory Cache

Rappin provides a thread-safe in-memory cache of your bot's profile, joined servers, and channels:

```rust
// Access the cache from any command or interaction context
let servers = ctx.cache().get_servers().await;
let channels = ctx.cache().get_channels_for_server(&server_id).await;

if let Some(bot_user) = ctx.cache().get_me().await {
    println!("Bot display name: {:?}", bot_user.display_name);
}
```

Calls to `get_me()`, `get_servers()`, and `get_server_channels()` populate the cache from their responses. Read the cache explicitly for local lookups; REST methods still fetch fresh data. The cache is a snapshot, not a live subscription.

---

## 📖 Interactive UI: Paginator & Confirm Dialog

### 1. Multi-Page Embed Paginator

Easily create browseable menus with previous/next buttons and page count indicators:

```rust
let pages = vec![
    Embed::new().title("Page 1").description("First page content"),
    Embed::new().title("Page 2").description("Second page content"),
    Embed::new().title("Page 3").description("Third page content"),
];

// Automatically builds message with buttons and sends it
let (_msg, paginator) = ctx.send_paginated(pages).await?;
```

Handle pagination updates in your button router:

```rust
bot.button("pg_", |ctx| async move {
    // Extract target page from button custom_id (e.g. "pg_1234:2")
    if let Some(page) = paginator.parse_page_from_custom_id(ctx.custom_id()) {
        ctx.update_paginator(&paginator, page).await?;
    }
    Ok(())
});
```

### 2. Confirm / Cancel Dialogs

Quickly prompt users for confirmation before destructive actions:

```rust
let (_msg, dialog) = ctx.send_confirm("Are you sure you want to reset the settings?").await?;
```

---

## 🪝 Standalone Webhooks & OAuth2

### 1. Webhook Client

Send messages or embed cards from external systems (GitHub Actions, GitLab CI, Grafana) without running a bot:

```rust
use rappin::webhook::WebhookClient;
use rappin::models::embed::Embed;

let webhook = WebhookClient::new("https://www.yapin.gg/api/webhooks/your_id/your_token");

// Send simple text
webhook.send_content("🚀 Build #142 passed!").await?;

// Send rich embed card
webhook.send_embed(
    Embed::new()
        .title("Deployment Complete")
        .description("Production has been updated to v1.4.0.")
        .color(0x57F287)
).await?;
```

### 2. "Login with Yapin’" OAuth2 Client

Implement web authentication:

```rust
use rappin::oauth2::OAuth2Client;

let oauth = OAuth2Client::new("client_id", "client_secret", "https://app.com/callback");

// 1. Redirect user to Yapin' authorization page
let login_url = oauth.authorization_url(&["identify"], Some("random_state_csrf"));
// Validate callback state against the value saved in the login session.
// For public apps, use OAuth2Client::public and the PKCE methods (see agent guide).

// 2. Exchange redirect authorization code for access token
let token = oauth.exchange_code(&code).await?;

// 3. Fetch user profile
let user = oauth.get_user_profile(&token.access_token).await?;
println!("Logged in as: {}#{}", user.username, user.id);
```

---

## 🌐 Quick Start: Standalone REST Client

If you want to use the API directly without running a background bot loop:

```rust
use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::new("yb_your_token_here");

    // Fetch bot profile
    let me = client.get_me().await?;
    println!("Bot user: {} (ID: {})", me.username, me.id);

    // Set online status & custom status text
    client.set_status(BotStatus::Online, Some("Working in Rust 🦀")).await?;

    // List all servers the bot has joined
    let servers = client.get_servers().await?;
    for server in servers {
        println!("Server: {} ({})", server.name, server.id);
    }

    Ok(())
}
```

---

## 🎨 Rich Cards & Components V2

### 1. Rich Embed Cards

```rust
use rappin::prelude::*;

let embed = Embed::new()
    .title("Server Report")
    .description("Here is the latest snapshot.")
    .color(EmbedColor::BLUE)
    .field("Members", "150", true)
    .field("Online", "42", true)
    .footer(EmbedFooter::new("Generated with Rappin"))
    .timestamp(chrono::Utc::now());

let msg = CreateMessage::new("Latest report:").embed(embed);
client.send_message(channel_id, &msg).await?;
```

### 2. Components V2 Layouts

Yapin’ supports Components V2 (`flags: 32768`), where components form the full message layout:

```rust
use rappin::prelude::*;

let layout = Container::new()
    .accent(EmbedColor::PURPLE)
    .add(TextDisplay::new("# Welcome to the Server!\nPlease review the rules below."))
    .add(Separator::divider())
    .add(
        Section::new("Ready to join the discussion?")
            .accessory(Button::primary("Agree & Enter", "agree:rules"))
    );

client.send_layout(channel_id, vec![layout], None).await?;
```

---

## 📡 LiveKit Voice Channels

Bots can join voice channels and receive an authorized LiveKit ticket:

```rust
let ticket = client.join_voice(server_id, channel_id).await?;
println!("LiveKit URL: {}", ticket.url);
println!("Token: {}", ticket.token);
println!("End-to-end encrypted: {}", ticket.e2ee);
```

Connect with any LiveKit client to publish an audio stream.

---

## 🗂️ API Endpoint Reference

| Category | Endpoint | Method | Client Method |
|---|---|---|---|
| **Bot / Me** | `/me` | `GET` | `client.get_me()` |
| | `/me` | `PATCH` | `client.update_me(req)` |
| | `/me/avatar` | `PUT` / `DELETE` | `client.set_avatar(...)` / `delete_avatar()` |
| | `/me/banner` | `PUT` / `DELETE` | `client.set_banner(...)` / `delete_banner()` |
| | `/me/status` | `PUT` | `client.set_status(...)` |
| **Commands** | `/commands` | `GET` / `PUT` | `client.get_commands()` / `set_commands(...)` |
| **Events** | `/events` | `GET` | `client.get_events(...)` |
| **Servers** | `/servers` | `GET` | `client.get_servers()` |
| | `/servers/{id}` | `GET` | `client.get_server(id)` |
| | `/servers/{id}/roles` | `GET` | `client.get_roles(id)` |
| | `/servers/{id}/members` | `GET` | `client.get_members(id, limit, after)` |
| | `/servers/{id}/members/{uid}` | `GET` / `PATCH` | `client.get_member(...)` / `set_nickname(...)` |
| | `/servers/{id}/members/{uid}/roles/{rid}` | `PUT` / `DELETE` | `client.add_role(...)` / `remove_role(...)` |
| | `/servers/{id}/kick` | `POST` | `client.kick_member(id, uid)` |
| | `/servers/{id}/bans` | `GET` / `POST` / `DELETE` | `get_bans` / `ban_member` / `unban_member` |
| | `/servers/{id}/timeouts` | `POST` | `client.timeout_member(id, uid, min, reason)` |
| | `/servers/{id}/categories` | `POST` / `PATCH` / `DELETE` | `create_category` / `edit_category` / `delete_category` |
| | `/servers/{id}/channels` | `GET` / `POST` | `get_server_channels` / `create_channel` |
| | `/servers/{id}/voice` | `GET` | `client.get_voice_states(id)` |
| | `/servers/{id}/voice/move` | `POST` | `client.move_voice_member(...)` |
| | `/servers/{id}/voice/disconnect` | `POST` | `client.disconnect_voice_member(...)` |
| | `/servers/{id}/voice/join` | `POST` | `client.join_voice(id, channel_id)` |
| | `/servers/{id}/voice/leave` | `POST` | `client.leave_voice(id)` |
| **Channels** | `/channels` | `GET` | `client.get_channels(type)` |
| | `/channels/{id}` | `GET` / `PATCH` / `DELETE` | `get_channel` / `edit_channel` / `delete_channel` |
| | `/channels/{id}/messages` | `GET` / `POST` | `get_messages` / `send_message` |
| | `/channels/{id}/messages/bulk-delete` | `POST` | `client.bulk_delete_messages(...)` |
| | `/channels/{id}/messages/{mid}` | `GET` / `PATCH` / `DELETE` | `get_message` / `edit_message` / `delete_message` |
| | `/channels/{id}/messages/{mid}/pin` | `PUT` / `DELETE` | `pin_message` / `unpin_message` |
| | `/channels/{id}/messages/{mid}/reactions` | `GET` / `PUT` / `DELETE` | `get_reactions` / `add_reaction` / `remove_reaction` |
| **Interactions** | `/interactions/{id}/{token}/callback` | `POST` | `client.respond_interaction(...)` |
| **Users** | `/users/{id}` | `GET` | `client.get_user(id)` |
| | `/users/{id}/dm` | `POST` | `client.create_dm(user_id)` |

---

## 🛠️ Examples

Check the `examples/` directory for ready-to-run examples:

- `cargo run --example ping_bot`: Complete interactive bot responding to `/ping`, `/roll`, and button selections.
- `cargo run --example cogs_bot`: Modular bot demonstrating replacement of native cog instances.
- `cargo run --features hot-reload --example hot_reload_bot`: Edit Rust worker source while the main bot stays running.
- `cargo run --example advanced_bot`: Showcase of command cooldowns, in-memory cache, multi-page paginator, and confirmation dialogs.
- `cargo run --example embeds_and_buttons`: Building rich Embed cards and Components V2 containers.
- `cargo run --example api_client`: Interacting directly with the REST API.

---

## 📄 License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
