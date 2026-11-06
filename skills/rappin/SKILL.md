---
name: rappin
description: "Operate Yapin with Rappin CLI/MCP: resolve servers/channels/members, read conversations, perform requested message or server actions, and consume durable events. Use for chat operations, not Rust bot implementation or incident diagnosis."
---

Use an available Rappin MCP connection or installed `rappin` from any directory.
For CLI setup, inspect `rappin --version`, `rappin capabilities` and
`rappin doctor --offline`. Install with
`cargo install rappin-rs --features cli` if needed. Preserve existing
configuration and compiled feature choices. Authentication uses `YAPPER_BOT_TOKEN`
or `~/.config/rappin/config.toml`; never put credentials in command arguments or
print configuration contents. Run live `rappin doctor` when auth/reachability
needs checking, rather than adding it to every operation.

Discover the requested target, then read relevant context:

```sh
rappin servers list
rappin channels resolve general --server "$SERVER_ID"
rappin messages list "$CHANNEL_ID" --limit 25 --max-items 10 --fields id,author_id,content,created_at
```

Inspect the `{ok,data}` / `{ok:false,error}` envelope, not just the exit code;
doctor can succeed while reporting missing authentication. Retain exact IDs and
scope names to a server. Incomplete discovery cannot establish uniqueness: read
[discovery and permissions](references/discovery.md) when resolving names,
continuing a bounded search, or handling a management/permission operation.

Use `rappin tools list` or MCP `tools/list` for installed operation names and
argument schemas; do not guess them from SDK method names. Focused HTTP schemas
are available through `rappin schema --method POST --path '/channels/{id}/messages'`.
The bundled schema describes that installed release, not guaranteed live platform
support. A dry run validates a payload, not authorization or rollout eligibility.

For requested writes, preview with `--dry-run` when useful; existing authorization
does not need another confirmation just because a preview was used. Pass JSON
using `--body-file -` or `tools call NAME --arguments-file -`; construct arguments
as data, without shell expansion of message text. The raw `request METHOD /path`
escape hatch still requires the same target scope and authorization.

Read only the reference needed:

- [Messages](references/messages.md): rich payloads, uploads and partial writes.
- [Events](references/events.md): durable delivery, receipts and acknowledgement.

MCP starts with `rappin mcp --cursor-file PATH`; that state path is operator-chosen,
not selected by tools. Rich tool bodies are inline objects, not model-selected
filesystem paths. Consume structured output or its compatible text copy once.
The public SDK guide is at https://docs.rs/rappin-rs/; select docs matching the
installed version when exact API behavior matters. These references also work
without a source checkout.
