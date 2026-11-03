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

Send `/ping`, edit the reply in `examples/hot_cog_worker.rs`, and save. The watcher rebuilds that example and replaces its worker. Send `/ping` again to see the changed code. Failed candidates leave the previous version available; inspect their structured reports as described below. `make hot-demo` runs the same example.

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
rappin = { package = "rappin-rs", version = "0.1.17", features = ["hot-reload"] }
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
- `watch.status()` returns separate `scan_error` and `reload_error` fields. A successful scan clears its scan error even if the source digest has not changed. A failed build, startup or publication remains visible until a changed candidate installs successfully. `last_error()` reports the scan error first, then any outstanding reload error. Failed candidates are retried after another source change.
- `watch.status().reload_failure` retains the typed candidate report, independently of scan recovery. It clears when a changed candidate installs successfully.

### Candidate diagnostics

`HotReload::load`, `from_executable` and `reload` return
`YappinError::HotReload { failure }` for candidate failures. Use
`YappinError::hot_reload_failure()` or `RemoteError::hot_reload_failure()` to
recover the same `HotReloadFailure` after a worker boundary or in a failure
callback. These report types are available without Cargo features; actually
building/loading workers requires `hot-reload`.

| `ReloadPhase` | Meaning |
|---|---|
| `Compilation` | Manifest/Cargo startup, compiler failure, build timeout, missing executable, or unreadable/oversized compiler output |
| `Startup` | Preparing/spawning the candidate, exit before ready, or startup timeout |
| `Protocol` | Invalid/incomplete ready frame or unsupported protocol version |
| `Registration` | Invalid manifest/definitions, name/ownership conflict, unloaded controller, or unavailable parent |
| `Publication` | Remote slash-command synchronization failed before local routes changed |

`diagnostics` contains compiler codes (for example `E0425`), levels, messages,
child notes and primary/secondary line/column spans. `CompilerSpan.file` is
project-relative or absent for outside locations. The report also carries
`exit` (observed code/signal), `timed_out`, `candidate_started`,
`publication_attempted`, `routes_published` and an optional structured `source`.
An absent exit means it was not observed, not success; cleanup-induced exits
are not substituted for the original status. Failed local candidates do not
publish routes. A remote publication attempt can nevertheless have an unknown
outcome: inspect `source` for its HTTP status, retry hint and outcome. Do not
infer remote rollback from `routes_published == false`.

Reports retain at most 32 diagnostic nodes including child notes, depth 2 of
children, 8 spans per node, 2,048 UTF-8 bytes per text field and 32 KiB of
diagnostic text. Structured causes have separate bounded text, 32-entry collection,
512-node and depth-8 limits; their top-level kind/status/retry and outcome fields remain intact.
`truncated` reports omitted/shortened fields. Compiler frames above 1 MiB stop
the build rather than allocate without limit. Raw Cargo/build-script stderr,
rendered output, source excerpts and suggestions are not returned or printed by
the builder. Credential-bearing environment values, the configured bot credential
and path-like message tokens are redacted; external source paths are omitted. Compiler text
can still contain application data deliberately embedded by trusted code, so
review reports before sharing them publicly.

Try the runnable offline example (dependencies must already be cached):

```sh
cargo run --features hot-reload --example hot_reload_diagnostics -- path/to/Cargo.toml worker_binary
```

It prints the structured report and exits with status 1 on failure. It performs
no authentication or API requests, but **does compile and execute trusted local
code**. Cargo build scripts and procedural macros can execute even when
compilation fails. Neither a timeout nor an unpublished candidate proves there
were no filesystem/network effects, rolls back build effects, or guarantees
termination of every descendant process. This is not a side-effect-free sandbox.

### Reporting a rejected reload without stopping intake

A failed candidate remains an unexpected handler error by default. Applications
can complete a known reload/proposal workflow using the explicit failure policy:

```rust,no_run
use rappin::{prelude::*, ReloadPhase};
let bot = Bot::builder("yb_token").on_command_failure(|ctx, failure| async move {
    if failure.handler == "reload" {
        if let Some(report) = failure.error.hot_reload_failure() {
            // Application contract: this operator-only route only attempted the
            // reload, and rejecting the proposal completes all required work.
            // Persist any required proposal/diagnostic receipt before Handled.
            if report.phase == ReloadPhase::Compilation
                && !report.candidate_started && !report.publication_attempted
                && !report.routes_published
            {
                let mut text = String::from("Candidate did not compile; active routes were kept.");
                for diagnostic in report.diagnostics.iter().take(3) {
                    let summary: String = diagnostic.message.chars().take(300).collect();
                    text.push_str(&format!("\n{}: {}", diagnostic.code.as_deref().unwrap_or("compiler"), summary));
                }
                ctx.reply(text).await?;
                return Ok(FailureDisposition::Handled);
            }
        }
    }
    Ok(FailureDisposition::Default)
});
```

Authorize the reload route separately; the callback does not grant permission.
This example handles only a known terminal compilation rejection. It does not
retry compilation or infer that arbitrary build effects were undone. Keep any
remaining reconciliation or durable recording explicit. A failed reply returns
an error and retains the original candidate report in `FailureHandling`; startup,
protocol, registration and publication errors keep the default stop/barrier
behavior here. The same typed report can be recorded and reconciled for those
phases without collapsing their uncertain outcomes into a generic rejection.
- `HotReload::from_executable(handle, path)` loads an existing worker executable. Its watcher fingerprints that file; your deployment tool supplies rebuilt binaries.

## Source scanning and recovery

Project watchers fingerprint Rust files, TOML files and `Cargo.lock`. They skip
hidden entries, `target` and `node_modules`. Database journals and session files
are not source inputs and can change without triggering a build. Rust/TOML files
under `data` or `state` still participate in source discovery. Use
`WorkerProject::watch_path(...)` to include additional shared source/config roots
or sources inside normally skipped directories. Explicit roots are required to
exist; keep their paths operator-controlled.

Disappearing non-source entries are tolerated. A source read interrupted by
replacement makes up to three whole-scan attempts, with 5 ms between attempts.
Persistent manifest, explicit-root, executable or access errors include the path
and phase, such as `resolve manifest`, `inspect entry`, `enumerate directory` or
`read source`. Source deletions detected by a changed inventory still trigger a
Cargo build: removing a required module is a failed candidate, not a successful
scan of an empty project. The active worker and routes remain installed on failure.

```rust,no_run
use rappin::hot_reload::WatchGuard;

fn report_watch(watch: &WatchGuard) {
    let status = watch.status();
    if let Some(error) = status.scan_error {
        eprintln!("Source scan: {error}");
    }
    if let Some(error) = status.reload_error {
        eprintln!("Candidate reload: {error}");
    }
}
```

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

## Typed commands and event waits (0.1.7)

The 0.1.7 worker SDK adds `command_typed` / `WorkerContext::parse`, live help
metadata, `prompt_for_reply`, and `wait_for_event`. The parent registers prompt
waiters before posting and owns the authenticated transport. See the
[Rust workflow guide](rust-bots.md#hot-workers) for bounds, cancellation, and
protocol compatibility. The worker example includes `roll` and `question`.

### API 1.2 worker transport

Protocol v4 adds public metadata reads, fixed 64 KiB upload chunks, DM listeners
and reaction listeners. Total upload data stays bounded to 12 MiB; ordinary frames
remain 1 MiB. New parents accept v1–v3 workers; rebuild both binaries when upgrading
workers. DM and native cog reaction hooks return Result and leave failed processing
uncommitted. See [API 1.2 workflows](https://github.com/Lost1419/Rappin/blob/main/docs/api-1.2.md).
