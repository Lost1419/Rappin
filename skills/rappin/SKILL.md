---
name: rappin
description: "Operate Yapin through the Rappin CLI or MCP tools: discover servers and members, read messages, send requested replies, and consume events. Use for Yapin chat operations, not Rust bot development."
---

Use the installed `rappin` from any working directory. Start with
`rappin doctor --offline` and `rappin capabilities`. If missing, install with
`cargo install rappin-rs --features cli,gateway`. Configure `YAPPER_BOT_TOKEN` in
the environment or `~/.config/rappin/config.toml`; never put credentials in arguments.
`rappin doctor` checks authentication and the active API origin.

Resolve the requested target, then read context before acting:

```sh
rappin servers list
rappin channels resolve general --server "$SERVER_ID"
rappin messages list "$CHANNEL_ID" --limit 25 --max-items 10 --fields id,author_id,content,created_at
```

Inspect the `{ok,data}` / `{ok:false,error}` envelope, not just the exit code
(doctor can succeed while reporting missing auth). Preserve returned IDs.
Disambiguate names within a server. Message cursors are timestamps; member cursors
are user UUIDs. Use `next_cursor` with its direction; server/channel output limits
instead return `next_offset` for `--offset`.

For writes the user requested, `--dry-run` previews the payload. Existing user
authorization carries forward; a preview does not create an extra approval step.
Use `--body-file -` for JSON from stdin, and avoid shell interpolation of message
contents. `rappin request METHOD /path` remains the explicit API escape hatch.
Read [message workflows](references/messages.md) for rich payloads, MCP equivalents,
and partial-send recovery. Read [event delivery](references/events.md) before
polling and acknowledging a durable event stream.

MCP uses `rappin mcp --cursor-file PATH` with the state path chosen at startup.
`rappin tools list` describes the same operations and schemas as MCP; invoke one
from the CLI with `rappin tools call NAME --arguments-file -`. Rich bodies are
inline MCP objects, never tool-selected filesystem paths. MCP responses include
both structured results and a compatible text copy; consume one copy.

Use `rappin schema --method POST --path '/channels/{id}/messages'` to inspect only
one operation and its referenced definitions. The full API guide is at
https://docs.rs/rappin-rs/latest/rappin/; these bundled references work without
access to the Rappin source checkout.
