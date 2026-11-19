# Rappin 🦀

[![Crates.io](https://img.shields.io/crates/v/rappin-rs.svg)](https://crates.io/crates/rappin-rs)
[![Docs.rs](https://docs.rs/rappin-rs/badge.svg)](https://docs.rs/rappin-rs)
[![CI](https://github.com/Lost1419/Rappin/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Lost1419/Rappin/actions/workflows/ci.yml?query=branch%3Amain)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An async Rust SDK and bot framework for [Yapin](https://www.yapin.gg).
Build commands, buttons and forms, organize them into cogs, and use the API from
native Rust handlers or reloadable Rust workers.

Rappin 0.2.0 contains breaking changes from 0.1. Follow the
[migration guide](docs/migration-0.2.md) when moving an existing bot to 0.2.

## Start a bot

Requires **Rust 1.89+** and a bot token created in Yapin's developer settings.
Add the bot to your server, then install the CLI and create a project.

```sh
cargo install rappin-rs --features cli --locked
rappin init my-bot --human
```

Set `YAPPER_BOT_TOKEN` in the new bot's process environment and run `cargo run`
inside the generated directory. Send `!ping` for `pong`, or `!help` to see commands.
The starter includes logging, a local restart checkpoint, and gateway events with
polling fallback. Edit `src/main.rs` to add commands. Commands use the same context
for prefix messages and slash interactions, with bounded read recovery enabled
by default.

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

## Cargo features

The default feature set is empty. Basic HTTP and polling, prefix commands, and
slash interactions are available in the base SDK.

| To use | Enable |
| --- | --- |
| Gateway event delivery with polling fallback | `gateway` |
| Derive argument parsing and help with `#[derive(FromArgs)]` | `derive` |
| Build the `rappin` CLI and MCP server | `cli` |
| Run commands in reloadable Rust workers | `hot-reload` |
| Use the voice playback bridge | `voice` |
| Use the webhook middleware for Axum | `webhook-middleware` |
| Export metrics over HTTP | `metrics-http` |
| Export metrics over HTTP and use Pushgateway | `metrics` |

Use `rappin init my-bot --template basic` for a polling starter, or
`--template worker` for hot reloading. Generation is offline. Building can access
the registry and bot execution contacts Yapin.

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

## Contributing

See the [development guide](docs/development.md) for tests, documentation checks,
minimum Rust verification and smaller local builds. Technical release notes live
in the [changelog](CHANGELOG.md).

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
