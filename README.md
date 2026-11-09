# Rappin 🦀

[![GitHub](https://img.shields.io/badge/github-Lost1419/Rappin-blue?logo=github)](https://github.com/Lost1419/Rappin)
[![Crates.io](https://img.shields.io/crates/v/rappin-rs.svg)](https://crates.io/crates/rappin-rs)
[![Docs.rs](https://docs.rs/rappin-rs/badge.svg)](https://docs.rs/rappin-rs)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An async Rust SDK and bot framework for [Yapin’](https://www.yapin.gg).
Build a bot with commands and interactive messages, use the REST client on its
own, or automate Yapin through the JSON CLI and MCP tools.

## Compatibility

0.1.x updates preserve documented public APIs and existing defaults; new behavior
is opt-in. Application-owned storage, cache backends and custom prefixes remain
supported. Breaking changes require a new Cargo compatibility line and an explicit
migration guide.

If you use 0.1.16 or earlier, follow the [0.1.17 migration notes](CHANGELOG.md#0117--2026-11-02)
when upgrading. Worker capabilities are opt-in and versioned; upgrade the parent
before enabling a newer worker protocol. See the [hot-reloading guide](docs/hot-reloading.md).

## What you can build

- **Command bots:** slash and prefix commands, typed arguments, aliases,
  subcommands, permission checks, cooldowns, and generated help.
- **Interactive messages:** embeds, buttons, Components V2 layouts, paginators,
  select menus, modal forms, confirmation dialogs, and typed collectors for replies
  and interactions.
- **Modular Rust bots:** group handlers into cogs, or rebuild and swap Rust worker
  cogs while the parent bot keeps running.
- **API integrations:** manage messages, channels, servers, members, roles,
  moderation, reactions, and voice through a standalone REST client.
- **Agent workflows:** discover channels, inspect history, send messages, and
  consume events using JSON commands or MCP over stdio. Rust consumers can discover
  capabilities offline, bind validated operations to commands, and prepare and invoke
  installed commands with consumer-owned operation IDs.

The library also includes channel webhooks, OAuth2 with PKCE, asset URL helpers,
and optional encrypted voice playback.

## Get started

Requires Rust 1.88 or newer. Create a bot in **Yapin → Settings → Developer**,
copy its token, and add it to your server.

Add these dependencies to your Rust project:

```toml
[dependencies]
rappin-rs = "0.1.31"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Save this as `src/main.rs`:

```rust,no_run
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
HTTP endpoint. To prefer WebSocket events, enable the `gateway` feature and add
`.gateway(rappin::gateway::DEFAULT_GATEWAY)`; Rappin handles heartbeats, resume,
and polling fallback. Follow the [gateway quickstart](docs/rust-bots.md#gateway-events)
for a complete runnable example. JSON API calls still use HTTPS by default;
[gateway API requests](docs/gateway-requests.md) are a separate opt-in.

Configure `.cursor_file("./state/events.json")` for restart checkpoints and
`.handler_concurrency(8)` to bound concurrent handlers. Checkpoints advance after
ordered completion; incomplete work holds the checkpoint. Use the
[failure policy](docs/handler-failures.md) to complete expected rejections or
explicitly handle a failed invocation while intake continues. Reconcile partial
or uncertain effects before replaying work. An opt-in
[availability policy](docs/availability-and-hierarchy.md) provides bounded,
cancellable recovery for transient startup and intake read failures.

Use [shared ingress](docs/shared-ingress.md) to combine verified webhooks with
polling recovery and optional gateway delivery. It deduplicates before event
effects and exposes admission receipts; durable restart recovery uses a
consumer-owned `IngressStore`. Bounded conflict reports identify competing sources
and schema differences; optional recovery retains completed receipts while waiting
for matching polling evidence.

[Message gates and fallible listeners](docs/message-policy.md) support shared
access policy before command dispatch and natural-language handlers. For composed
workflows, [checked invocation](docs/checked-invocation.md) reuses installed command
checks and handlers while retaining the original message and a separate consumer
operation identity.

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
cargo install rappin-rs --features cli
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

Create a starter with `rappin init my-bot`, or select `--template gateway` or
`--template worker`. Generation is offline and refuses existing directories; see
[project templates](docs/project-templates.md).

See the [agent guide](docs/agents.md) for configuration and tool usage, or use the
repository's [operator skill](https://github.com/Lost1419/Rappin/blob/main/skills/rappin/SKILL.md). Skills for
[building Rust bots](https://github.com/Lost1419/Rappin/blob/main/skills/rappin-bot-dev/SKILL.md) and
[diagnosing failures](https://github.com/Lost1419/Rappin/blob/main/skills/rappin-diagnostics/SKILL.md) are also available in the repository.

## Optional features

The default library build needs neither Node nor the CLI dependencies.

| Cargo feature | Enables |
|---|---|
| `cli` | JSON CLI and MCP stdio server |
| `gateway` | WebSocket events and opt-in JSON API requests |
| `hot-reload` | Compilation and replacement of Rust worker cogs |
| `webhook-middleware` | Axum webhook signature verification and verified event extraction |
| `metrics-http` | Prometheus HTTP export for request, availability and transport metrics |
| `metrics` | Prometheus export including push-gateway support |
| `voice` | Encrypted voice playback through the Node/ffmpeg bridge |

## Smaller builds

Install the CLI with `cargo install rappin-rs --features cli --profile small`,
or build a checkout with `cargo build --profile small --features cli`. The profile
uses size optimization, thin LTO, one codegen unit and symbol stripping, with panic
unwinding enabled. Symbol stripping reduces backtrace detail. Consumer profile
settings and compact local development commands are in the build-size guide.
[Build-size guide](docs/development.md#smaller-executables).

## Guides and examples

- [API reference](https://docs.rs/rappin-rs): public types, client methods, and compiling usage examples. The reference includes all optional features; enable the relevant Cargo feature in your application.
- [SDK development](docs/development.md): feature/MSRV checks, documentation coverage, randomized testing and offline performance measurement.
- [Project templates](docs/project-templates.md): offline basic, gateway and worker scaffolding.
- [API support by release](docs/api-support.md): reviewed feature coverage, required features, known gaps, and the public API-audit skill.
- [Rust bot guide](docs/rust-bots.md): typed commands, checks, collectors, and delivery semantics.
- [Checked invocation](docs/checked-invocation.md): invoke installed commands from the original message context, with read-only preparation, consumer operation IDs, and explicit completion.
- [Handler failures](docs/handler-failures.md): completion decisions, structured errors, draining, and checkpoints.
- [Shared ingress](docs/shared-ingress.md): verified webhooks, polling recovery, deduplication, durable receipts, and checkpoint ownership.
- [Timestamp reconciliation](docs/timestamp-reconciliation.md): explicit audit for completed legacy envelope conflicts.
- [Draining and retirement](docs/draining-and-retirement.md): API lifetimes during shutdown, polling authority, and explicit assessment of uncertain work.
- [Gateway API requests](docs/gateway-requests.md): opt-in JSON request transport, bounded concurrency, and uncertain-write handling.
- [Transport and caching](docs/transport-and-cache.md): API hosts, retries, custom caches, and prefixes.
- [API 1.3 interactions](docs/api-1.3.md): select menus, modal forms, slash interaction mode, and message emoji discovery.
- [Server management](docs/server-management.md): roles, permission overrides, invites, counts, and scheduled events.
- [Availability and role hierarchy](docs/availability-and-hierarchy.md): bounded read recovery and fresh, independent requester/bot role checks.
- [Shared SDK workflows](docs/sdk-workflows.md): permissions, native/worker APIs, runtime schemas, request metrics, and bounded lookups.
- [Capability bindings](docs/capability-bindings.md): offline operation discovery, scoped execution, and permanent commands.
- [Access policy example](docs/authorization-policy.md): fresh permission evidence, native/worker use, and platform limits on effective access.
- [Hot reloading](docs/hot-reloading.md): native cogs and reloadable Rust workers.
- [Agent guide](docs/agents.md): CLI, MCP, credentials, OAuth, webhooks, and voice setup.
- [CLI/MCP workflows](docs/cli-mcp.md): shared operations, bounded results, delivery receipts, and recovery.
- [Changelog](CHANGELOG.md): release history and migration notes.

Run these examples from a checkout. Set `YAPPER_BOT_TOKEN` for the bot, client,
and form examples below. The shared webhook example also requires
`YAPIN_BOT_ID` and `YAPIN_WEBHOOK_SECRET`.

| Command | Demonstrates |
|---|---|
| `cargo run --example ping_bot` | Commands and interactive buttons |
| `cargo run --example typed_bot` | Typed arguments, aliases, checks, and help |
| `cargo run --example cogs_bot` | Modular native cogs |
| `cargo run --features hot-reload --example hot_reload_bot` | Rust source hot reloading |
| `cargo run --features gateway --example gateway_bot` | WebSocket events with polling fallback |
| `cargo run --example advanced_bot` | Cooldowns, cache, pagination, and confirmations |
| `cargo run --example embeds_and_buttons` | Embeds and Components V2 layouts |
| `cargo run --example api_client` | Standalone REST client |
| `cargo run --example interactive_forms` | Select menus and modal forms |
| `cargo run --example checked_invocation` | Prepare and invoke installed commands |
| `cargo run --example operation_identity` | Two-step preparation and consumer-owned operation keys |
| `cargo run --features gateway --example gateway_api_requests` | Gateway events and JSON API requests |
| `cargo run --features webhook-middleware --example shared_webhook_polling` | Verified webhooks with polling recovery |

Keep bot tokens out of source control and logs. Handlers, custom transports, and
loaded cogs are trusted application code; see the
[credential security notes](docs/agents.md#credential-hardening-014).

## Local build storage

Use `cargo build --profile compact` or `cargo test --profile compact --all-features`
for smaller artifacts without debug symbols or incremental caches. This profile
is opt-in; normal dev/test builds keep their existing settings. Cargo profiles
are controlled by the root workspace, so applications depending on Rappin keep
their own build configuration.

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
