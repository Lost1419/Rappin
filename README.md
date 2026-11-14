# Rappin 🦀

An async Rust SDK and bot framework for [Yapin](https://www.yapin.gg).
Build commands, buttons and forms, organize them into cogs, and use the API from
native Rust handlers or reloadable Rust workers.

**This branch contains the 2.0 development preview.** Follow the
[migration guide](docs/migration-2.0.md) when moving an existing bot to this branch.
The [published release](https://crates.io/crates/rappin-rs) has its own versioned reference.

## Start a bot

Requires **Rust 1.89+** and a bot token created in Yapin's developer settings.
Add the bot to your server, then install the development CLI and create a project.

```sh
cargo install --git https://github.com/Lost1419/Rappin --branch dev --features cli --locked
rappin init my-bot --human
```

Set `YAPPER_BOT_TOKEN` in the new bot's process environment and run `cargo run`
inside the generated directory. Send `!ping` for `pong`, or `!help` to see commands.
The starter includes logging, a local restart checkpoint, and gateway events with
polling fallback. Edit `src/main.rs` to add commands.

A minimal bot looks like this.

```rust,no_run
use rappin::prelude::*;
use rappin::bot::TransportConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::from_env()?
        .transport(TransportConfig::gateway())
        .command("ping", "Check the bot", |ctx| async move {
            ctx.reply("pong").await?;
            Ok(())
        })
        .build()?;
    bot.run().await
}
```

The example requires the `gateway` feature. The generated project enables it.
Keep the token in the process environment and out of source control and logs.

## Learn in order

The [tutorial](docs/tutorial.md) assumes basic Rust, async functions and `Result`.
It walks through a first command, typed arguments, buttons, shared state,
permissions, cogs, hot reloading and deployment. Each section links to a runnable
example and the relevant task guide.

For typed commands, enable `derive` and describe the arguments once.

```rust
use rappin::prelude::*;

#[derive(FromArgs)]
struct Roll {
    /// Number of sides on the die.
    #[arg(default = 6, min = 2, max = 100)]
    sides: u32,
}

let command = CommandSpec::<Roll>::new("roll", "Roll a die");
assert!(command.info().help().contains("Number of sides"));
```

The derive generates parsing, bounds, usage and argument help together. Ordinary
Rust handlers and manual `FromArgs` implementations support custom behavior.

## Find a task

- [Commands, checks and collectors](docs/rust-bots.md)
- [Messages, buttons, selects and forms](docs/interactions.md)
- [Cogs and Rust hot reloading](docs/hot-reloading.md)
- [Resource lookup and computed permissions](docs/resources.md)
- [Uploads, emojis and webhooks](docs/uploads-and-webhooks.md)
- [Transport, caching and custom prefixes](docs/transport-and-cache.md)
- [Failure handling and checkpoints](docs/handler-failures.md)
- [CLI and MCP](docs/agents.md)

The [guide index](docs/README.md) includes advanced invocation, shared ingress,
recovery, authorization and SDK development. Applications select their own state,
databases, cache backends, prefix resolvers and approval policies.

## Optional features

The library's default feature set is empty. Generated gateway projects enable
`gateway` and `derive` explicitly.

| Feature | Adds |
|---|---|
| `derive` | Argument parsing and help derives |
| `gateway` | WebSocket event delivery and optional gateway API requests |
| `hot-reload` | Rebuildable Rust worker cogs |
| `cli` | JSON CLI, project creation and MCP over stdio |
| `webhook-middleware` | Verified Axum webhook intake |
| `metrics-http` | Prometheus HTTP export |
| `metrics` | Prometheus HTTP and push-gateway export |
| `voice` | Encrypted playback through the Node/ffmpeg bridge |

Use `rappin init my-bot --template basic` for a polling starter, or
`--template worker` for hot reloading. Generation is offline. Building can access
the registry and bot execution contacts Yapin.

## Contributing

See the [development guide](docs/development.md) for tests, documentation checks,
minimum Rust verification and smaller local builds. Technical release notes live
in the [changelog](CHANGELOG.md).

Licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
