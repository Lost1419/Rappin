# Rappin 🦀

[![GitHub](https://img.shields.io/badge/github-Lost1419/Rappin-blue?logo=github)](https://github.com/Lost1419/Rappin)
[![Crates.io](https://img.shields.io/crates/v/rappin-rs.svg)](https://crates.io/crates/rappin-rs)
[![Docs.rs](https://docs.rs/rappin-rs/badge.svg)](https://docs.rs/rappin-rs)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An async Rust SDK and bot framework for [Yapin’](https://www.yapin.gg).
Build a bot with commands and interactive messages, use the REST client on its
own, or automate Yapin through the JSON CLI and MCP tools.

## What you can build

- **Command bots:** slash and prefix commands, typed arguments, aliases,
  subcommands, permission checks, cooldowns, and generated help.
- **Interactive messages:** embeds, buttons, Components V2 layouts, paginators,
  confirmation dialogs, and typed collectors for replies and clicks.
- **Modular Rust bots:** group handlers into cogs, or rebuild and swap Rust worker
  cogs while the parent bot keeps running.
- **API integrations:** manage messages, channels, servers, members, roles,
  moderation, reactions, and voice through a standalone REST client.
- **Agent workflows:** discover channels, inspect history, send messages, and
  consume events using JSON commands or MCP over stdio.

The library also includes channel webhooks, OAuth2 with PKCE, asset URL helpers,
and optional encrypted voice playback.

## Get started

Requires Rust 1.88 or newer. Create a bot in **Yapin → Settings → Developer**,
copy its token, and add it to your server.

Add these dependencies to your Rust project:

```toml
[dependencies]
rappin-rs = "0.1.10"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Save this as `src/main.rs`:

```rust
use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .expect("Set YAPPER_BOT_TOKEN before starting the bot");

    let bot = Bot::builder(token)
        .command("ping", "Check if the bot is alive", |ctx| async move {
            ctx.reply("pong 🦀").await?;
            Ok(())
        })
        .command("menu", "Choose a fruit", |ctx| async move {
            let row = ActionRow::new(vec![
                Button::success("Apple", "fruit:apple"),
                Button::primary("Orange", "fruit:orange"),
            ]);
            ctx.reply_with(
                CreateMessage::reply("Pick a fruit:", ctx.message_id()).action_row(row),
            ).await?;
            Ok(())
        })
        .button("fruit:", |ctx| async move {
            let choice = ctx.custom_id().strip_prefix("fruit:").unwrap_or("unknown");
            ctx.ephemeral(format!("You picked {choice}!")).await?;
            Ok(())
        })
        .build();

    bot.run().await
}
```

Run it with your token in the environment:

```sh
export YAPPER_BOT_TOKEN="yb_your_token"
cargo run
```

Send `/ping` or `!ping` in your server, or try `/menu` and click a button.
Rappin registers the commands with Yapin automatically. The default command
prefixes are `/` and `!`; add your own with `.prefix(...)`, or use
`.prefixes_with(...)` for per-server or database-backed prefixes.

## Events and state

Bots use long polling by default, so they can run on a laptop without a public
HTTP endpoint. Enable the `gateway` feature and add
`.gateway(rappin::gateway::DEFAULT_GATEWAY)` to receive events over WebSocket,
with heartbeats, resume, and polling fallback when the gateway is unavailable.

Configure `.cursor_file("./state/events.json")` for restart checkpoints and
`.handler_concurrency(8)` to bound concurrent handlers. Checkpoints advance after
ordered handler completion; a failed batch remains available for replay.

The built-in cache holds profile, server, and channel snapshots. Member/role
lookup caching is opt-in, and `LookupCache` lets you use your own database or
cache backend. Applications control cache freshness, storage, and dynamic prefix
resolution. See [transport and cache configuration](docs/transport-and-cache.md).

## Cogs and hot reloading

Cogs group commands, button handlers, and listeners. Native cogs can be installed,
removed, or replaced at runtime using code linked into the bot.

For source edits, the `hot-reload` feature builds separate Rust worker cogs and
swaps their routes while the parent bot stays connected. Existing requests finish
on the old worker; failed builds or registrations leave it serving. The parent
owns the bot token, and workers request authenticated operations through it.
Keep durable worker state in your own database or files.

Try the demo from this repository:

```sh
cargo run --features hot-reload --example hot_reload_bot
```

Edit [hot_cog_worker.rs](examples/hot_cog_worker.rs) and save to trigger a rebuild.
See the [hot-reloading guide](docs/hot-reloading.md) for setup and operating limits.

## CLI and MCP for agents

Install the optional CLI from crates.io:

```sh
cargo install rappin-rs --features cli,gateway
export YAPPER_BOT_TOKEN="yb_your_token"
rappin doctor
rappin servers list
rappin channels resolve general --server "$SERVER_ID"
rappin messages list "$CHANNEL_ID" --limit 25
rappin --dry-run messages send "$CHANNEL_ID" --content "Hello from an agent"
```

Use the IDs returned by discovery commands for `SERVER_ID` and `CHANNEL_ID`.
Output is JSON, writes support `--dry-run`, and JSON request bodies can come from
stdin. `rappin mcp` exposes discovery, history, messages, and explicit event
acknowledgement as stdio MCP tools. `rappin schema` prints the bundled OpenAPI spec.

See the [agent guide](docs/agents.md) for configuration and tool usage, or use the
included [operator skill](skills/rappin/SKILL.md). Skills for
[building Rust bots](skills/rappin-bot-dev/SKILL.md) and
[diagnosing failures](skills/rappin-diagnostics/SKILL.md) are also included.

## Optional features

The default library build needs neither Node nor the CLI dependencies.

| Cargo feature | Enables |
|---|---|
| `cli` | JSON CLI and MCP stdio server |
| `gateway` | WebSocket events with resume and polling fallback |
| `hot-reload` | Compilation and replacement of Rust worker cogs |
| `voice` | Encrypted voice playback through the Node/ffmpeg bridge |

## Guides and examples

- [API reference](https://docs.rs/rappin-rs): Rust types and client methods.
- [Rust bot guide](docs/rust-bots.md): typed commands, checks, collectors, and delivery semantics.
- [Transport and caching](docs/transport-and-cache.md): API hosts, retries, custom caches, and prefixes.
- [Server management](docs/server-management.md): roles, permission overrides, invites, counts, and scheduled events.
- [Shared SDK workflows](docs/sdk-workflows.md): permissions, native/worker APIs, runtime schemas, request metrics, and bounded lookups.
- [Hot reloading](docs/hot-reloading.md): native cogs and reloadable Rust workers.
- [Agent guide](docs/agents.md): CLI, MCP, credentials, OAuth, webhooks, and voice setup.
- [CLI/MCP workflows](docs/cli-mcp.md): shared operations, bounded results, delivery receipts, and recovery.
- [Changelog](CHANGELOG.md): release history and migration notes.

Run the examples from a checkout with `YAPPER_BOT_TOKEN` set:

| Command | Demonstrates |
|---|---|
| `cargo run --example ping_bot` | Commands and interactive buttons |
| `cargo run --example typed_bot` | Typed arguments, aliases, checks, and help |
| `cargo run --example cogs_bot` | Modular native cogs |
| `cargo run --features hot-reload --example hot_reload_bot` | Rust source hot reloading |
| `cargo run --example advanced_bot` | Cooldowns, cache, pagination, and confirmations |
| `cargo run --example embeds_and_buttons` | Embeds and Components V2 layouts |
| `cargo run --example api_client` | Standalone REST client |

Keep bot tokens out of source control and logs. Handlers, custom transports, and
loaded cogs are trusted application code; see the
[credential security notes](docs/agents.md#credential-hardening-014).

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
