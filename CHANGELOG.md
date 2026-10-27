# Changelog

## 0.1.4 — 2026-10-27

- Confine authenticated API requests to their configured origin and path prefix, reject unsafe endpoint URLs, and refuse redirects in default HTTP clients and the voice bridge.
- Remove public access to the bot token; mark authorization headers sensitive and redact credentials in diagnostics, reflected API errors, deserialization errors, and CLI/MCP JSON keys and values.
- Restrict MCP tools to validated arguments and an event-state path selected at process startup, closing arbitrary file-selection paths.
- Fall back to REST when gateway raw-frame tracing is enabled. Limit voice subprocess environment inheritance and ffmpeg protocols.
- Update SHA-256/HMAC dependencies and add token-exposure regression tests, including an independent HMAC test vector and poisoned-environment voice checks.
- Restrict the example cog-reload command to the bot owner and document credential trust boundaries.

### Migration

- `Client::token()` is no longer public. Keep token ownership in application setup rather than command handlers.
- Configure MCP event state with `rappin mcp --cursor-file PATH`; `poll_events` and `ack_events` no longer accept tool-selected file paths.
- Remote credential endpoints require HTTPS/WSS. Explicit loopback development endpoints can still use HTTP/WS. Raw request paths cannot escape the configured API prefix.
- The default transports refuse redirects. Custom HTTP clients remain trusted operator overrides and must preserve redirect/TLS/logging protections.

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
