---
name: rappin-bot-dev
description: "Build and extend Rust bots with Rappin: typed or runtime commands, scoped API bindings, message policies, state, and reloadable workers. Use for bot implementation, not operating Yapin through CLI/MCP or investigating a running incident."
---

Inspect the existing Cargo manifest/lock and application structure first. Match
examples to the selected Rappin version; do not upgrade dependencies merely to
fit an example. Preserve application-owned storage, caches, state and prefixes.
For a new bot, copy the bundled [starter](assets/starter), rename
`Cargo.toml.template` to `Cargo.toml`, set `YAPPER_BOT_TOKEN` in the environment,
and run `cargo run`. Do not overwrite an existing project with the starter.

The package is `rappin-rs`; imports use `rappin`. Rust 1.88+ is required. Ordinary
commands are async closures returning `rappin::Result<()>`. Use `CommandSpec` /
`FromArgs` for typed parsing, bounds, aliases, checks and help. Do not invent
derive macros. The default library needs neither CLI/Clap nor Node.

Choose the reference only when relevant:

- For model-selected operations, permanent commands or moderation authorization,
  read [bindings and permissions](references/bindings-and-permissions.md).
- For message gates/listeners, completion/cursors, availability recovery or Rust
  source reloads, read [execution and reload](references/execution-and-reload.md).

Static prefixes use `.prefix(...)`; database-dependent prefixes use
`.prefixes_with(...)`. Member/role caching is off by default. `LookupCache` lets
an application retain its database; use `Freshness::Refresh` for fresh checks.
`LookupResult::into_unique` rejects incomplete/ambiguous searches; one candidate
from a bounded search is not proof of uniqueness. Prefer scoped stable IDs.

Use context reply/interaction helpers to coordinate acknowledgement and expiry.
Register collectors before sending prompts; distinguish timeout, overflow and
shutdown. Await required I/O before completing a handler. Preserve structured
causes and partial receipts; never retry an entire handler or uncertain mutation
because an individual request supplied Retry-After.

Validate the changed behavior with local fixtures and compile against the chosen
features. Keep live writes inside the user's requested test scope. Report what
was tested and any remaining platform limitations. Read API docs matching the
selected dependency at https://docs.rs/rappin-rs/ rather than assuming `latest`
matches an older lockfile. Public guide modules include `guide`,
`capability_bindings_guide`, `message_policy_guide`, `handler_failures_guide`, and
`availability_and_hierarchy_guide` (the last requires Rappin 0.1.20+).
