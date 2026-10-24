# Rappin 🦀

[![GitHub](https://img.shields.io/badge/github-Lost1419/Rappin-blue?logo=github)](https://github.com/Lost1419/Rappin)
[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io/crates/rappin)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

An idiomatic, type-safe, asynchronous Rust client and bot framework for the [Yapin’](https://www.yapin.gg) Bot API.

Yapin’ is a chat platform providing an HTTPS-based bot API with long-polling events, slash commands, interactive buttons, embeds, Components V2 cards, and LiveKit-powered voice channels.

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

- **Complete API Coverage (All 35 OpenAPI Endpoints)**:
  - Bot profile & presence (`/me`, `/me/status`, `/me/avatar`, `/me/banner`)
  - Channels & messages (send, edit, delete, bulk delete, pin, reactions)
  - Server management (members, roles, kicks, bans, unbans, timeouts, categories)
  - Button interactions (`reply`, `ephemeral`, `update`, `ack`)
  - Voice channels (obtain LiveKit tickets with E2EE metadata)
  - Direct messages (`/users/{id}/dm`)
- **Dual Mode**: Use either the declarative `Bot` long-polling engine or the standalone `Client` for REST scripting.
- **Dynamic Cogs & Hot-Reloading**: Group commands and interactions into modular cogs that can be added, removed, or reloaded while the bot is running without dropping connections.
- **Components V2 Support**: Full support for Yapin's modern layout system (`Container`, `Section`, `TextDisplay`, `Thumbnail`, `MediaGallery`, `Separator`, `FileDisplay`).
- **CDN Helpers**: Helper methods to resolve avatar, banner, icon, and attachment paths to their complete CDN URLs (`https://files.yapper.website/...`).

---

## 📦 Installation

Add `rappin` and `tokio` to your `Cargo.toml`:

```toml
[dependencies]
rappin = "0.1.0"
tokio = { version = "1", features = ["full"] }
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

## 🧩 Cogs & Live Hot-Reloading

Cogs allow splitting commands, button interactions, and event listeners into isolated modules that can be added, removed, or reloaded in real time **while the bot is still running**, with zero downtime.

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

### 2. Updating Cogs at Runtime

Because `Bot` provides a cloneable `BotHandle` (accessible through `bot.handle()` or `ctx.bot()`), you can reload or add cogs on the fly without restarting the process:

```rust
let bot = Bot::builder(token)
    .cog(ModerationCog)
    .command("reload", "Reload a cog live", |ctx| async move {
        if let Some(bot) = ctx.bot() {
            bot.reload_cog(ModerationCog).await?;
            ctx.reply("✅ Moderation cog reloaded live!").await?;
        }
        Ok(())
    })
    .build();

bot.run().await?;
```

When a cog is added, removed, or reloaded, Rappin automatically synchronizes the new command list with Yapin' (`PUT /commands`), immediately updating the server's slash command menu!

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
- `cargo run --example cogs_bot`: Modular bot demonstrating dynamic Cog loading and real-time reloading.
- `cargo run --example embeds_and_buttons`: Building rich Embed cards and Components V2 containers.
- `cargo run --example api_client`: Interacting directly with the REST API.

---

## 📄 License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
