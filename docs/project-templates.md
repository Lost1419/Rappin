# Start a Rust bot project

Install the development CLI, then create a project.

```sh
cargo install --git https://github.com/Lost1419/Rappin --branch dev --features cli --locked
rappin init my-bot --human
rappin init polling-bot --template basic --prefix '?'
rappin init worker-bot --template worker --human
rappin --dry-run init preview-bot --template worker
```

`init` creates files offline. It reads neither authentication nor the CLI's API
configuration. `--human` prints next steps. Default output and explicit `--json`
use the stable JSON envelope, with generated relative paths and contents in
`data.files`. A dry run returns the planned files.

| Template | SDK features | Behavior |
|---|---|---|
| `gateway` — default | `gateway`, `derive` | Ping, help, logging, WebSocket events and polling fallback |
| `basic` | `derive` | Ping, help and logging with long polling |
| `worker` | `gateway`, `hot-reload`, `derive` | Gateway parent and a watched `examples/worker.rs` |

Set `YAPPER_BOT_TOKEN` in the bot process environment and run `cargo run` inside
the new directory. Send `!ping` for `pong` and `!help` for command help. Change
static prefixes with `prefixes`, or provide a database-backed resolver through
`prefixes_with`. The [tutorial](tutorial.md) shows how to add commands and state.

Generated projects require the generating CLI's Rust minimum and SDK version.
Development versions use the GitHub `dev` dependency. Published stable versions
use crates.io. Cargo builds can access the network. Tests compile every starter
against the exact checkout under test with its declared features.

The project includes an ignored `state/events.json` checkpoint. Give each cursor
one runner. Applications select their own database, cache backend, durable state,
approvals and replay policy. Failed replies and uncertain effects retain the
checkpoint barrier. See [failure handling](handler-failures.md).

## Destination rules

The destination must be new and its parent must exist. Existing directories and
destination symlinks are rejected. Creation reserves the directory and creates
files exclusively. If writing fails, cleanup covers files created by that
invocation. Inspect any remaining partial directory before retrying.

Use `--name` for an explicit Cargo package name. Names use ASCII letters, digits,
hyphens and underscores, begin with a letter, and have at most 64 characters.
Reserved Cargo/system names are rejected. Prefixes accept 1–64 UTF-8 bytes with
control characters excluded and are escaped as Rust literals.

## Worker lifecycle

Keep the reload controller and watcher guard alive for the parent's lifetime.
Worker command specs use protocol 12. Each generation initializes its own
process-local state. Workers, build scripts and procedural macros execute trusted
code. Reserve worker stdout for IPC and send diagnostics to stderr.
See [hot reloading](hot-reloading.md) for candidate failures, draining and replacement.
