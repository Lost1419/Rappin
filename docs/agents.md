# Rappin for agents

Install a binary from this checkout:

```sh
cargo install --path . --features cli,gateway,voice
# Alternative: make install-local (add ~/.local/bin to PATH)
rappin --json doctor --offline
```

The library has no default optional features. `cli` adds the command and MCP server;
`gateway` adds WebSocket transport; `voice` adds the Node playback bridge. No Node
runtime is needed for REST, CLI, MCP, polling, or gateway use.

For Rust bot development, optional `hot-reload` rebuilds and swaps edited Rust
worker cogs while the parent bot stays running. Start with
`cargo run --features hot-reload --example hot_reload_bot` and edit
`examples/hot_cog_worker.rs`. See the [Rust hot-reloading guide](hot-reloading.md)
for the two-binary setup, reply helpers, state boundaries, and credential handling.
Available since `0.1.5`; native `reload_cog` replaces linked instances and does not
compile source edits.

## Authentication and output

Set `YAPPER_BOT_TOKEN`. Precedence is environment, then `token` in
`~/.config/rappin/config.toml` (or `--config path`). Prefer environment credentials.
Do not put tokens in command arguments. A TOML config may contain:

```toml
base_url = "https://www.yapin.gg/api/bot/v1"
# token = "yb_..." # optional; restrict this file's permissions if used
```

Base URL precedence: `--base-url`, `YAPPER_API_BASE`, config, default.
`rappin doctor` checks auth/reachability; `--offline` only checks local configuration.
Doctor is a diagnostic report: inspect its `authenticated`, `reachable`, and `setup`
fields, even when the command exits successfully.

All commands emit JSON by default; `--json` documents that intent. Help/version are
text. Output is one success envelope `{"ok":true,"data":...}` or one error envelope
`{"ok":false,"error":{"kind":"validation","message":"...","field":"content","status":null,"retry_after":null}}`.
Data is the API response unless a collection/workflow wrapper is described below.
Diagnostics go to stderr. The configured bot token is redacted from JSON strings.
Exit codes: 0 success (including empty lists and doctor reports), 2 invalid input or
configuration, 1 API/transport/I/O failure. No implicit confirmations or prompts.

## Discover, resolve, read

```sh
rappin --json servers list
rappin --json channels list --server "$SERVER_ID"
rappin --json channels resolve general --server "$SERVER_ID"
rappin --json members resolve "$SERVER_ID" alice --max-pages 2
rappin --json messages list "$CHANNEL_ID" --limit 50 --max-pages 2
rappin --json messages get "$CHANNEL_ID" "$MESSAGE_ID"
rappin --json schema Message
```

`schema` returns the bundled official OpenAPI snapshot; `schema Message` returns
one component. The spec calls send/edit payloads inline rather than defining a
`CreateMessage` component, so use `schema` to inspect those paths.
Name resolution rejects ambiguous names. Member resolution searches a bounded
number of pages and reports no match within that scope; use a UUID for exact lookup.

Message cursors are RFC3339 timestamps, **not message UUIDs**. Specify `--before`
to read backward or `--after` to read forward, never both. Returned messages are
oldest first, with authors. Collections include `next_cursor`, `direction` (messages),
`page_count`, and `may_have_more`. Pass the cursor with that direction to continue.
The API exposes only timestamp message cursors: multiple messages at exactly the
page-boundary timestamp may require an overlapping query and ID deduplication.
`next_cursor` is a candidate continuation when a full page is returned, not proof
that another page exists. Limits: messages 1–100; members 1–500; max pages 1–100.
The library also provides bounded `message_history` and `member_pages` helpers.

## Preview and write

```sh
rappin --dry-run messages send "$CHANNEL_ID" --content "Hello"
printf '%s' '{"content":"Hello","reply_to":"00000000-0000-0000-0000-000000000000"}' |
  rappin --dry-run messages send "$CHANNEL_ID" --body-file -
rappin messages send "$CHANNEL_ID" --content "Requested reply" --reply-to "$MESSAGE_ID"
rappin messages edit "$CHANNEL_ID" "$MESSAGE_ID" --body-file edit.json
rappin messages react "$CHANNEL_ID" "$MESSAGE_ID" '👍'
rappin dm open "$USER_ID"
rappin dm send "$USER_ID" --content "Requested DM"
rappin --dry-run channels create "$SERVER_ID" --body-file channel.json
```

Dry runs perform local typed validation and return the proposed method/path/body;
they need no token or network. Sends return `data.messages`, an array of API send
responses (or dry-run requests). `--split` splits plain text on Unicode boundaries
into messages of at most 2000 characters; each is a separate write. A later failure
can leave earlier chunks posted: inspect history before resending a failed batch.
The library's `split_message` and `send_long_message` expose the same operation.

Use `{"embeds":null,"components":null}` to clear existing message fields. Omitted
PATCH fields remain unchanged. `InteractionCallback::clear_content`, `clear_embeds`,
and `clear_components` produce explicit nulls. Local validation checks message,
embed, button, and V2 component limits. The server remains authoritative for
permissions, media-host allowlists, moderation, and API changes.

Bot roles determine permissions: message deletion/pins require Manage messages
unless deleting its own; role changes require Manage roles; channel/category
changes require Manage channels; nicknames require Manage nicknames; kicks/bans/
timeouts require their matching permissions. Voice movement requires Move Members
and sufficient role hierarchy. `403` cannot be fixed by repeating the request.

Raw escape hatch for endpoints without a named CLI command:

```sh
rappin request GET "/servers/$SERVER_ID/roles"
rappin --dry-run request POST "/servers/$SERVER_ID/timeouts" --body-file timeout.json
```

Raw requests preserve supplied JSON and bypass typed payload validation. They use
the same auth, bounded retries, and error contract. Use only requested writes.
The client retries 429/503 responses only with a valid numeric `Retry-After`, up to
its configured retry budget, with delays at most 60 seconds. Headerless 429s are
returned immediately. Transport errors and other 5xx responses aren't blindly
retried because writes may already have executed. Preserve message IDs and check
history before retrying an ambiguous send; the API has no documented idempotency key.

## Resume events

```sh
rappin events poll --after 0
rappin events poll --after 1234 --wait 25
rappin events ack 1234 --cursor-file ./state/events.json
rappin events poll --cursor-file ./state/events.json --wait 25
```

Poll returns `events`, `next_cursor`, and `acknowledged:false`. It never commits a
batch automatically. With no saved cursor or `--after`, 0 starts from now. Persist
the returned watermark after handling **all** events up to it (including empty
batches where the API skipped expired events). One owner per cursor file. Never
reuse a file between different bot tokens. Cursor writes use atomic replacement.

`EventConsumer` redelivers a pending batch until acknowledged and validates IDs
against that batch. `Bot::builder(token).cursor_file(path).handler_concurrency(8)`
waits for all batch handlers before checkpointing. A failed command/button stops
the runner without advancing that batch. Restarting may replay successful siblings:
handlers must tolerate duplicates. Set concurrency to 1 for ordered processing.
`max_event_age` deliberately skips older events; the API also expires old commands
and interactions. Bot listeners returning `()` must handle their own failures.
Shutdown stops polling and drains the active batch; it waits for handlers to finish.

With `gateway`, opt in through `.gateway(rappin::gateway::DEFAULT_GATEWAY)`.
The transport identifies/resumes, maintains heartbeats, reconnects from the last
handled ID and falls back to polling. Beta denial retries after 5 minutes; invalid
auth stops. The same cursor works for both transports. Gateway event filters also apply
to fallback polling; the bot builder uses all event types by default.

## Inbound and channel webhooks

For an inbound HTTP endpoint, compare `X-Yapper-Bot` to the expected bot ID and call
`webhook::verify_event(secret, signature, raw_body)` **before** trusting/parsing the
event. The helper accepts the webhook's `event` field as an alias for polling's
`type`. Enqueue verified events, return 200 within 3 seconds, then await
`bot.process_event(event)`. Maintain deduplication by event ID when also polling;
webhook delivery itself does not retry. Do not advance a polling watermark merely
because a later webhook ID arrived: earlier events may still need processing.
Your HTTP server, queue, and replay policy remain application-owned.

Outbound channel webhook URLs are `/api/webhooks/<id>/<token>`.
`WebhookClient::send` uses `wait=true` to return a message;
`send_no_wait(&WebhookMessage)` accepts a successful 204. `WebhookMessage` supports
username/avatar overrides, allowed mentions, embeds, and Discord-shaped link
buttons. Channel webhooks cannot invoke bot callback buttons. They do not upload
files; use permitted HTTPS URLs. Never log webhook URLs.

## OAuth

`OAuth2Client::get_user_profile` calls `/api/oauth/me` and returns `OAuth2User`.
Use only `identify`; `servers` is not supported. Public apps use
`OAuth2Client::public`, `Pkce::new`, `authorization_url_pkce`, and
`exchange_code_pkce`. Keep the verifier and random state in the login session and
validate the callback state. Tokens currently have no refresh-token flow.

## MCP

Configure an MCP client with command `rappin`, args `["mcp"]`, and an environment
containing `YAPPER_BOT_TOKEN`. The stdio server exposes server/channel discovery,
channel resolution, bounded history, message sending, event polling and explicit
acknowledgement. Configure a fixed cursor at process startup with
`rappin mcp --cursor-file ./state/events.json`; tool arguments cannot choose local
file paths. Without that option, acknowledgement is disabled. It uses newline-delimited JSON-RPC; stdout is protocol-only.
Writes accept `dry_run:true`. Tool errors set `isError` and carry the CLI envelope.
Example configuration (supply the environment credential through your client):

```json
{"mcpServers":{"yapin":{"command":"rappin","args":["mcp"]}}}
```

## Encrypted voice

Build with `voice` and install the optional runtime explicitly:

```sh
make voice-setup
# Setup includes a local ffmpeg binary.
rappin voice "$SERVER_ID" "$VOICE_CHANNEL_ID" ./music/song.mp3 \
  --sdk .rappin-voice/yapper.mjs --node-modules .rappin-voice/node_modules
```

The setup script downloads the official SDK and installs `@livekit/rtc-node` plus `ffmpeg-static` in a
local ignored directory; review the SDK/dependency updates as with other runtimes.
Rust calls `Client::play_audio` with `VoiceOptions`, which spawns Node using an
embedded bridge. Credentials travel through stdin, not process arguments. The
official SDK joins LiveKit, distributes the E2EE key, and publishes an encrypted
track; ffmpeg decodes local audio into 48 kHz stereo PCM. The bridge pads final
partial frames, waits for playout, and leaves on success/failure. Playback timeout
is configurable (one hour by default). Use Connect/Speak permissions and a local
audio file you can play. Bots cannot receive audio or publish video. Abrupt process
termination may require an explicit `leave_voice` call; graceful signals attempt
cleanup. No music is downloaded by Rappin.

## Compatibility changes

History cursor argument types changed from UUID to `DateTime<Utc>`. Interaction
PATCH-like fields now use `Option<Option<T>>` (omit/set/null). New `BotProfile`
fields require updates to struct literals; prefer deserialization for API objects.
OAuth returns `OAuth2User` rather than the broader public-profile model. Command
and button handlers are bounded and awaited rather than detached. All 35 REST path
templates (51 operations) remain available. Source of truth: bundled
[OpenAPI](openapi.json), [API reference](https://www.yapin.gg/developers/api), and
[developer guide](https://www.yapin.gg/developers).

## Credential hardening (0.1.4)

Credential handling is hardened in 0.1.4. The framework is not a sandbox for
application code: handlers, custom transports, configured endpoints, local SDKs,
and the host operating system must be trusted.
`Client::token()` is no longer public. Default clients refuse redirects and insecure
remote endpoints. Custom transports, endpoints, handlers and voice SDK paths must
be trusted operator configuration. MCP cursor paths are supplied at startup rather
than in tool calls. Upgrade from 0.1.3 to receive these changes.

Authorization headers are marked sensitive; token-bearing diagnostic models and
reflected API errors redact credentials. CLI/MCP redaction covers JSON keys and
values. The gateway uses REST fallback instead of authenticating while raw frame
tracing is enabled. The voice bridge uses an environment allowlist, refuses HTTP
redirects, and limits ffmpeg to local-file/pipe protocols. Keep dependency wire
logging disabled and do not log serialized OAuth, interaction, or voice tickets.
The SHA-256/HMAC dependencies have been updated to the maintained RustCrypto chain.
These protections and regression tests reduce known exposure paths; they do not
prove absence of every vulnerability or protect against malicious trusted code.
