# Start a Rust bot project

Install the CLI with its `cli` feature, then create a new project:

```sh
cargo install rappin-rs --features cli
rappin init my-bot
rappin init gateway-bot --template gateway --prefix '?'
rappin init worker-bot --template worker
rappin --dry-run init preview-bot --template worker
```

`init` is an offline, local operator command. It does not load your Rappin config,
read a token, run Cargo, or contact Yapin. It is not an MCP tool. All commands return
the standard JSON envelope; `data.files` contains generated relative paths and
contents. Dry runs return the same plan without creating files.

The destination must not exist, and its parent must already exist. Existing
directories and destination symlinks are refused, including during dry runs.
There is no overwrite flag. File creation is exclusive; if writing fails, the
command attempts to remove only files/directories it created. A filesystem failure
may leave a partial new project, which should be inspected before a retry.
Generation is not an atomic directory publication or an isolation boundary against
another process modifying the same destination.

Choose a Cargo package name with `--name` when the directory name is unsuitable.
The generator accepts ASCII names starting with a letter, up to 64 characters,
using letters, digits, hyphens and underscores, excluding reserved names. Prefixes
accept 1..64 UTF-8 bytes without control characters and are escaped as Rust literals.
Generated `prefixes_with` selects that initial prefix; replace it with your own
server-aware resolver when needed. Explicit slash-command dispatch remains available.

| Template | Required SDK features | Behavior |
|---|---|---|
| `basic` | None | Ping command using default polling |
| `gateway` | `gateway` | Ping command using the default gateway and supported fallback |
| `worker` | `hot-reload` | Parent with a separately compiled, watched `examples/worker.rs` |

Generated projects depend on the generating CLI's Rappin version and declare Rust
1.89. Tests compile each template against the checkout under test; when using a
published CLI, Cargo resolves its published dependency normally. Building can
require network access even though generation itself does not.

Set `YAPPER_BOT_TOKEN` in the bot's environment and run `cargo run` inside the new
directory. Do not put credentials in generated source or commit them. The parent
uses `state/events.json` for a local restart checkpoint; production persistence,
application state, database/cache backend, approvals and replay policy remain yours.
No database is installed or selected by the template.

Worker builds run trusted Rust code, including build scripts and procedural macros.
Workers are trusted executables rather than sandboxed extensions. Keep stdout
reserved for the protocol, write diagnostics to stderr, and retain the controller
and watch guard for the parent's lifetime. See [hot reloading](hot-reloading.md)
for replacement failures, draining and protocol compatibility.
