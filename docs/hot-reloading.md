# Reload Rust code while the bot runs

Enable `hot-reload` to put a cog in a separate Rust executable. Rappin compiles the edited source, starts and validates a candidate, then switches its commands, button prefixes, and listeners to the new worker. The parent bot retains its event loop, client, cache, cursor, and token throughout. Existing handler invocations retain their original worker until they finish.

The worker uses stable, versioned JSON over local stdin/stdout pipes. Both sides are ordinary Rust programs; no additional runtime or `cargo-watch` installation is needed. Native `Cog` and `BotHandle::reload_cog` remain available for replacing instances of code already linked into the parent.

Available since `rappin-rs` version `0.1.5` with Cargo feature `hot-reload`.

## Try the included example

From the Rappin checkout:

```sh
export YAPPER_BOT_TOKEN="yb_your_token"
cargo run --features hot-reload --example hot_reload_bot
```

Send `/ping`, edit the reply in `examples/hot_cog_worker.rs`, and save. The watcher rebuilds that example and replaces its worker. Send `/ping` again to see the changed code. Compilation or startup errors appear on stderr and leave the previous version available. `make hot-demo` runs the same example.

## Use two Rust binaries in your bot

A simple layout keeps the parent and worker in the same Cargo package:

```text
my-bot/
  Cargo.toml
  src/main.rs
  src/bin/fun.rs
```

In `Cargo.toml`, select the parent as the default binary and enable the feature:

```toml
[package]
name = "my-bot"
version = "0.1.0"
edition = "2021"
default-run = "my-bot"

[dependencies]
rappin = { package = "rappin-rs", version = "0.1.6", features = ["hot-reload"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

`src/main.rs` owns the token and the running bot:

```rust,no_run
use rappin::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
    let bot = Bot::builder(token)
        .cursor_file("./state/events.json")
        .build();

    let fun = HotReload::load(
        bot.handle(),
        WorkerProject::new("Cargo.toml", "fun"),
    ).await?;
    let _watch = fun.watch(Duration::from_millis(500)).await?;
    bot.run().await
}
```

`src/bin/fun.rs` contains the reloadable commands:

```rust,no_run
use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    WorkerCog::new("fun")
        .command("ping", "Check the bot", |ctx| async move {
            ctx.reply("Pong from Rust!").await?;
            Ok(())
        })
        .button("fun:hello", |ctx| async move {
            ctx.ephemeral("Hello!").await
        })
        .run().await
}
```

Run `cargo run` from your bot package, then edit `src/bin/fun.rs`. Keep the watch guard alive: dropping it stops watching but leaves the installed cog running. The parent source can also change on disk, but only code compiled into the worker is activated by a worker reload. Parent changes still require rebuilding and restarting the parent.

## Handlers and API access

`WorkerContext` provides `args()`, `argv()`, `author_id()`, `channel_id()`, and the public `event: CogEvent`. Reply helpers include `reply`, `reply_with`, `say`, `send`, `ephemeral`, `update`, `ack`, and `respond`. `reply_with` accepts the regular rich `CreateMessage` builders for message events.

For other endpoints, `ctx.api("GET", "/servers", None).await?` sends a JSON request through the parent's authenticated client, including its normal URL validation and rate-limit handling. This interface supports JSON bodies and responses. Streaming or binary uploads are not exposed by the worker proxy.

Use `.listen(Listener::Message, |ctx| async move { /* ... */ Ok(()) })` for message listeners; `Interaction`, `MemberJoin`, and `MemberLeave` are also supported. Match `ctx.event` to read the corresponding payload. As with native cogs, message listeners run after command dispatch and interaction listeners run after button dispatch.

Reserve stdout for the protocol. Use `eprintln!` or configure your logger for stderr. Configure a `tracing` subscriber in the parent to see reload diagnostics (as the included example does), or inspect the watch guard's `last_error()`. Await API requests inside handlers; do not detach requests into background tasks or retain a context for use after its handler returns. Worker contexts reject requests after their invocation has finished, and a cancelled partial API exchange invalidates the worker connection.

## Reload controls

- `fun.reload().await?` builds and activates a candidate explicitly. Controllers are cloneable, so an owner-only native command can capture one. Keep project paths and target names operator-controlled.
- `fun.unload().await?` removes its routes and lets active invocations finish. Its watcher stops, and that controller cannot reload the unloaded cog. Call `HotReload::load` to load it again.
- `fun.generation()` starts at 1 and advances after successful replacements. `process_id().await` and `commands().await` expose the current worker and metadata.
- `watch.last_error()` returns the latest watcher failure; successful watched reloads clear it. A failed build is retried after another source change, rather than continuously compiling the same broken code.
- `HotReload::from_executable(handle, path)` loads an existing worker executable. Its watcher fingerprints that file; your deployment tool supplies rebuilt binaries.

The cog name must remain stable. Commands cannot collide with another cog or a native bot command. Slash commands synchronize before the local route switch while the parent is running, unless `.sync_commands(false)` was selected. API registration and local routing cannot form one distributed transaction: a network timeout may leave the server's slash menu uncertain, while local handlers remain on the previous version. Retry or call `BotHandle::sync_commands()` to reconcile it.

## Builds, state, and limits

Cargo builds use a separate `target/rappin-hot` directory to avoid sharing the parent's build lock. The worker executable is copied to a private temporary directory so subsequent builds cannot overwrite a live generation. Its working directory is the Cargo manifest directory; prebuilt workers use their source executable's directory. Workers exit after their last invocation and controller reference is released; Rappin sends shutdown, allows two seconds to exit, then kills them if necessary.

The watcher scans Rust source, TOML files, and `Cargo.lock` beneath the manifest directory. It skips hidden directories, symlinks, `target`, and `node_modules`. Add shared source directories outside that tree with `.watch_path(path)`. `.features(["feature"])`, `.package("workspace-member")`, and `.offline(true)` configure worker builds. For examples use `WorkerProject::example(manifest, name)`.

Worker-local memory starts fresh each generation. Keep durable state in a database or file, and design shared writes for overlapping generations. State is not automatically migrated. Each worker generation handles one invocation at a time; use separate cogs for independent workloads. The parent remains asynchronous, and new routes can use the new generation while an old invocation drains.

Default limits are a five-minute build, ten-second startup, and 120-second handler invocation, including time queued behind another invocation in that worker. Customize the public `WorkerProject::{build_timeout, startup_timeout, handler_timeout}` fields before loading. IPC frames are limited to 1 MiB, with at most 128 API requests per invocation.

Runtime crashes, protocol errors, and handler timeouts follow the bot's normal handler-error policy; they are not silently retried or automatically restarted. Reload the worker to recover an invalid connection. A timed-out write may already have reached the API, so check its outcome before retrying. Failed event batches retain the normal cursor/replay behavior. Cog listener errors are logged, as with the existing listener API.

## Credential boundary

The parent keeps the bot token. Workers receive event data, including interaction callback tokens when applicable, and the results of their requested API operations. The parent redacts its bot token from API values and errors before returning them. Worker environments are cleared and rebuilt from a small runtime allowlist; Cargo receives additional toolchain variables but does not inherit bot or registry-token environment variables.

Worker code, dependencies, Cargo configuration, and build scripts must be trusted. These are ordinary same-user processes with API capabilities and filesystem access, not an OS sandbox. Environment isolation does not prevent malicious source from reading same-user credential files or performing destructive API calls. Restrict source/build paths and any reload command to the operator; never load executable paths from chat input.
