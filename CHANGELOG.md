# Changelog

## 0.1.3 — 2026-10-27

- Add a JSON CLI and MCP stdio tools for discovery, name resolution, bounded history, message writes, dry runs, schemas, and explicit event acknowledgement.
- Add durable polling cursors, bounded handler concurrency, and optional WebSocket gateway transport with resume, heartbeats, and polling fallback.
- Add authenticated inbound webhook parsing, channel-webhook payloads, and optional encrypted voice playback through the official Yapin SDK and a local Node/ffmpeg runtime.
- Fix message-history timestamp cursors, retry status preservation, webhook success handling, OAuth profile requests, PKCE, bot profile fields, explicit-null interaction updates, and component deserialization.
- Validate message, embed, button, and Components V2 limits locally. Add Unicode-safe message splitting and bounded member/history helpers.
- Release cog registry locks before awaiting callbacks and wait for handlers before committing event checkpoints.
- Include an agent guide, companion skill, OpenAPI snapshot, voice setup script, and expanded CI coverage.

### Migration

- Requires Rust 1.88 or newer.
- `Client::get_messages` uses `Option<DateTime<Utc>>` for `before` and `after`, replacing UUID cursors.
- Interaction content, embeds, and components use `Option<Option<T>>` to distinguish omission, replacement, and explicit null.
- `BotProfile` has additional fields; update struct literals accordingly.
- OAuth identity lookup returns `OAuth2User` and supports only the `identify` scope.
- Command and button handlers are awaited with bounded concurrency. Failed handlers leave their event batch uncommitted for replay after restart.

See [the agent guide](docs/agents.md) for setup and migration details.
