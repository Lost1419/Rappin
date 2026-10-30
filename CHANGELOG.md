# Changelog

## 0.1.9 — 2026-10-29

- Bind agent event acknowledgements to persisted delivery receipts. Replay pending batches, reject unseen/stale IDs, reconcile interrupted commits, and lock cursor files across processes. Numeric SDK checkpoints remain compatible; the explicit CLI `--force` recovery path is unavailable through MCP.
- Run up to 16 MCP calls concurrently, correlate out-of-order responses, preserve partial input frames, and support read cancellation. Submitted writes finish rather than implying that cancellation undoes them.
- Share the named operation catalog between MCP and `rappin tools`. Add member discovery, exact resource reads, rich message editing/sending, deletion, reactions and pins, with inline JSON bodies, structured results and output schemas.
- Add offline capabilities and focused method/path schema lookup. Doctor reports the active API host and its own request timing/retries. Bound collection output by items, bytes or fields while retaining usable continuation boundaries and timestamp groups.
- Return confirmed results and the failed step for interrupted split sends, with explicit retry/outcome information. Make message-history futures usable in spawned tasks.
- Add portable operator, Rust bot-development and diagnostics skills, including a Rust starter.

### Migration

- MCP responses may arrive out of order; correlate them by JSON-RPC ID. Existing text results remain alongside structured output.
- Cursor-setting scripts must poll for a receipt before acknowledging, or deliberately use the CLI-only recovery override. Keep receipt sidecars private.
- Exhaustive matches on `YappinError` must handle `PartialWrite`. Existing single-send errors retain their original kind.

See [CLI/MCP workflows](docs/cli-mcp.md) for contracts and recovery behavior.

## 0.1.8 — 2026-10-29

- Use `bots.yapper.website` as the default REST host. Connection failures and HTTP 502/504 activate the website API for ten minutes. Only GET/HEAD can retry across hosts; ambiguous writes return their original error. Custom bases never fail over; `api_fallback(false)` pins the default host.
- Reuse shared request-body buffers instead of copying JSON or image uploads for every attempt. Parse interaction events once for intake/dispatch, and avoid cloning their JSON tree before decoding.
- Make lookup-cache reads check only the requested entry; remove expired entries when queried or when capacity is needed. Skip response clones when lookup caching is disabled. Add `LookupCache` and builder hooks for application-owned SQL/Redis caches, with explicit freshness and mutation/event invalidation.
- Prepare static prefixes once and index command roots when routes change. Dynamic prefix resolvers still run per message; cog replacement updates handlers and the index together.
- Skip disk writes for unchanged event cursors. Advancing checkpoints retain ordered completion, atomic replacement and file synchronization.
- Preserve additional message/reaction/member metadata and interaction expiry. Track server-issued deadlines before callbacks and never extend them when an interaction is observed again. Add credential-free webhook metadata through `WebhookClient::get_info()`.
- Accept biographies up to the developer guide’s 300-character limit. OpenAPI still says 190; stricter server errors remain visible. Refresh the bundled specification’s API origin.

### Migration

- The default REST origin changes; explicitly configured bases are preserved.
- Manual literals of `MessageEvent`, `InteractionEvent`, `ReactionEvent` and `MemberLeaveEvent` need the new optional fields. Existing JSON payloads remain compatible.

See [transport and cache configuration](docs/transport-and-cache.md) for custom backends, prefix behavior and fallback controls.

## 0.1.7 — 2026-10-27

- Add typed CommandSpec / FromArgs commands, quote-aware arguments, bounded numbers/durations/IDs, aliases, local subcommand paths, dynamic/mention prefixes, reusable owner/server/role/permission checks, and live help metadata across native cogs.
- Separate bounded event intake from globally bounded handlers. Preserve ordered completed-batch checkpoints, keep polling/heartbeats active during ordinary handler work, and resume after gateway backpressure.
- Add typed bounded collectors with timeout, idle timeout, filters, limits, cancellation, and explicit overflow errors. Add scoped context helpers and parent-owned prompt/wait workflows for hot workers.
- Expose configured/active transport, fallback reasons, WebSocket heartbeat RTT, cursor/backlog status, and sanitized REST timing observations. Add gateway-only mode.
- Coordinate interaction response state across Client clones and worker proxies; add defer, opt-in early auto-ack, idempotent acknowledgements, and conservative handling of cancelled/ambiguous writes.
- Share scoped Retry-After deadlines across Client clones, recognize HTTP dates and global message waits, and add opt-in bounded member/role TTL caches with explicit freshness and mutation/event invalidation.
- Add a compiled typed-bot example, worker typed/wait examples, a Rust workflow guide included in rustdoc, and regression coverage. Worker protocol v2 adds help/waits; the host continues accepting v1.

### Migration

- Existing closure commands remain available. `ctx.argv` now respects quotes and escapes; `ctx.args` remains the original argument string.
- Exhaustive matches on `YappinError` must handle the new `Command` and `Wait` variants.
- Worker protocol v2 requires an updated host; the new host still accepts v1 workers. Rebuild workers when upgrading the SDK.


## 0.1.6 — 2026-10-27

- Fix channel decoding when Yapin returns both `kind` and `type`. Keep support for either field alone, reject conflicting values, and preserve `kind` in serialized output. This fixes typed channel endpoints and CLI/MCP channel resolution.
- Accept omitted MCP tool arguments as an empty object, as permitted by the protocol. Continue rejecting explicit null/array arguments and missing required fields. Simplify MCP dispatch without changing its fixed cursor-path boundary.

## 0.1.5 — 2026-10-27

- Add optional `hot-reload`: compile edited Rust worker cogs and swap commands, button routes, and listeners while the parent bot keeps its event loop and credentials.
- Keep the working cog available during compilation and registration; reject failed builds, invalid startup, conflicting commands, and failed slash-command synchronization before changing local routes. Existing invocations finish on their original worker.
- Provide a Rust `WorkerCog` builder with reply, interaction, listener, and proxied JSON API helpers; add source watching, explicit reload/unload, and runnable examples. Worker subprocesses receive an allowlisted environment without the bot token.
- Make native cog replacement transactional and clarify that `reload_cog` replaces linked instances rather than recompiling source.

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
