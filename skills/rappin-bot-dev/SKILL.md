---
name: rappin-bot-dev
description: "Create or extend Rust bots using Rappin, including commands, interactive messages, custom state, cogs, and hot reloading. Use for Rust bot implementation rather than operating Yapin through chat tools."
---

Preserve the existing bot's project structure, database and prefix choices. For a
new bot, the bundled [starter](assets/starter) is a compilable Cargo application:
copy it to the requested project directory, rename `Cargo.toml.template` to
`Cargo.toml`, set `YAPPER_BOT_TOKEN` in the
environment and run `cargo run`. Check the installed/published Rappin version
before choosing dependency constraints. Rust 1.88+ is required.

The crate package is `rappin-rs`; Rust imports use `rappin`. Ordinary commands are
async closures returning `rappin::Result<()>`. Use CommandSpec/FromArgs for typed
arguments, bounds, aliases, checks and generated help. Inspect the crate's Rust
API docs rather than inventing derive macros or copying a JavaScript SDK shape:
https://docs.rs/rappin-rs/latest/rappin/.

Choose event transport explicitly when needed: polling works by default; Cargo
feature `gateway` plus `.gateway(rappin::gateway::DEFAULT_GATEWAY)` opts into
WebSocket with fallback. Request observations measure REST through response
headers; heartbeat RTT is a separate metric. A fast heartbeat does not prove a
fast message API.

Static prefixes use `.prefix(...)`. Database-dependent prefixes belong in
`.prefixes_with(...)`, whose async resolver runs per message requiring prefix
resolution. Member/role caching is off by default. Applications may implement
`LookupCache` for their own database or manage state outside the SDK; never force
an in-memory database into an existing application. Fresh authorization checks
should bypass stale cached permissions.

Keep authorization checks on moderation and reload commands. Use context helpers
for replies and interactions; they coordinate response state and expiry. Register
collectors before sending prompts. Bounded handler concurrency and cursor files
are configurable, and failed batches can replay after restart; design side
effects accordingly.

Native cogs replace code already linked into the process. Source changes require
the optional `hot-reload` worker system: the Rust parent retains credentials and
workers use its API proxy. Failed candidate builds leave the old worker running.
Store durable worker state in application storage because worker memory resets.

Run `cargo fmt --check`, `cargo check`, and tests relevant to changed behavior.
Prefer local fixtures for routing, permission checks, retries and event delivery.
Use live channel writes only within the user's authorized test scope; do not ship
temporary tokens or acceptance bots with the project. See the published
`guide`/`transport_and_cache_guide` rustdoc modules and linked examples for
advanced setup, or the source repository's docs when a checkout is available.
