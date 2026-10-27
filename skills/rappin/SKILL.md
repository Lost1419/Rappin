---
name: rappin
description: Use the Rappin CLI or MCP tools to read Yapin servers, channels, messages and events, send requested bot replies, or manage Yapin bot resources. Applies to Yapin chat automation, not other chat platforms.
---

Use the installed `rappin` command from any directory. Verify with `command -v rappin`
and `rappin --json doctor --offline`. Set `YAPPER_BOT_TOKEN` through the environment;
never print it or put it in process arguments. `rappin doctor` verifies auth/network.
If missing, install from the Rappin checkout with `cargo install --path . --features cli,gateway,voice`.

Discover servers, resolve the requested channel in its server, then read context:

```sh
rappin --json servers list
rappin --json channels resolve general --server "$SERVER_ID"
rappin --json messages list "$CHANNEL_ID" --limit 25
```

Every command emits `{ok,data}` or `{ok:false,error}`. Inspect `ok` and preserve
returned IDs. Name resolution errors on ambiguity; narrow to a server or use a UUID.
Message continuation cursors are RFC3339 timestamps, not message IDs. Bound reads
with `--limit` and `--max-pages`; reuse `next_cursor` with the returned direction.

For a requested write, preview the exact content before executing it:

```sh
rappin --dry-run messages send "$CHANNEL_ID" --content "Requested text"
rappin messages send "$CHANNEL_ID" --content "Requested text" --reply-to "$MESSAGE_ID"
```

Do not infer authorization for sending messages or moderation from read access.
Honor authorization already given by the user; previews need no extra confirmation.
Use `--body-file -` with piped JSON for complex payloads. Raw escape hatch:
`rappin request GET "/servers/$SERVER_ID/roles"`. `rappin schema` exposes the full
OpenAPI snapshot. CLI help lists the narrow write verbs.

For events, poll with a saved cursor and acknowledge only after handling the complete
batch. `after=0` starts from now. One consumer per cursor file; separate bots need
separate files. Do not retry an ambiguous send without checking history: the API has
no documented idempotency key. A permission failure needs changed permissions, not
more retries. Voice playback is outbound only and requires the optional Node/ffmpeg
runtime; `rappin voice --help` describes paths. `rappin mcp` exposes stdio tools when
an MCP client is preferred. Supply its fixed state file at startup with
`rappin mcp --cursor-file ./state/events.json`; tool calls cannot select local paths.

See the repository's `docs/agents.md` for JSON shapes, retry behavior, permissions,
webhooks, OAuth PKCE, gateway fallback, migration details, and voice setup.
