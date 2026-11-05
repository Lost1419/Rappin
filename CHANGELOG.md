# Changelog

## 0.1.26 — 2026-11-05

- Add explicit, fingerprint-bound reconciliation for durable Completed server-message events whose only difference is an outer null timestamp supplied by fresh polling. Preserve original events/completions and require consumer approval, atomic audit storage and matching normal polling before checkpoint release.
- Add the default-deny `IngressStore::commit_timestamp_reconciliation` hook and transaction validation. Reject stale decisions, changed payloads/identity and incomplete or uncertain receipts; keep ambiguous durable commits inspectable without replaying handlers.
- Include a standalone maintenance example and Unix journal adapter with atomic persistence, exclusive leasing and retained original-receipt audit evidence. Existing ingress defaults and worker protocols are unchanged.

## 0.1.25 — 2026-11-05

- Return structured shared-ingress payload conflicts with transport/recovery sources, event IDs, canonical SHA-256 fingerprints and bounded schema differences. Keep payload values and arbitrary object keys out of diagnostics; compare JSON objects independently of key order, including when consumers enable serde_json `preserve_order`.
- Add opt-in completed-receipt recovery that waits for the original payload through polling, with a fenced checkpoint, bounded deadline and readback interval. Preserve completed effects and reject contradictory polling payloads; an advanced watermark alone cannot resolve a conflict.
- Add durable conflict records and optional receipt provenance to the consumer storage contract. Preserve source compatibility for existing stores; durable recovery requires explicit conflict persistence/load support. Keep original receipts and both conflict/storage causes when persistence fails.
- Halt intake consistently on fast-path webhook conflicts under the default Stop policy; previously that path only returned an admission error. Existing non-conflicting delivery and worker protocols remain unchanged. Do not merge missing envelope timestamps with payload timestamps without a platform equivalence contract.

## 0.1.24 — 2026-11-05

- Add bounded consumer operation IDs to checked command preparation and execution. Keep authenticated message identity unchanged; expose the ID to native checks/handlers and retain it in prepared plans, outcomes and structured incomplete-invocation errors.
- Propagate IDs through opt-in worker protocol v8 without changing public command-event layouts. Reject identified calls to older workers before handler execution; existing worker modes and explicit-command defaults remain compatible. Nested calls inherit the current ID unless explicitly overridden.
- Document consumer-owned argument binding, stable-key journals and uncertain-effect reconciliation, with a runnable two-step example. Operation IDs grant no permission, suppress no handlers and never enable automatic replay.

## 0.1.23 — 2026-11-05

- Normalize native Windows compiler spans to project-relative paths while continuing to omit outside-project and traversal paths. Recognize the platform cursor-lock contention code so competing CLI/MCP consumers receive the correct error.
- Preserve MSVC discovery/library variables only during Windows cog compilation; runtime workers keep the existing restricted environment.
- Add opt-in shared ingress for verified webhooks, polling and gateway events. Deduplicate before collector/ack/handler effects, fence authoritative polling checkpoints around unfinished claims, and expose bounded admission receipts plus an application-owned durable store contract. Retain completed and uncertain outcomes across restart instead of replaying handlers.
- Add opt-in gateway JSON API requests when `ready.requests` advertises support. Correlate out-of-order responses, enforce eight in-flight requests and 100 frames per ten seconds, and share existing validation/rate-limit budgets with HTTPS. Preserve single-attempt worker calls and uncertain disconnect/timeout outcomes without HTTP fallback or mutation replay.

- Add checked invocation of installed native/worker commands from active message contexts. Reuse group/alias resolution, parsing, cog/registered checks and cooldowns; reject identity substitution, detached calls, excessive recursion and stale prepared bindings.
- Add advisory preparation for dynamic schemas and read-only command checks. Defer opaque parsers, checks and cooldowns; recheck authorization at execution. Nested failures retain structured causes and receipts without sending duplicate framework replies or replaying work.

## 0.1.21 — 2026-11-04

- Update the bundled OpenAPI to 1.3.0. Add `get_message_emojis` across Client, shared APIs, scoped bindings, CLI and MCP, with independent history-access checks, message/channel scope verification and test-server availability metadata (57 operations total).
- Add select menus, modal inputs/forms, structured emoji references and lossless UI message bodies/responses. Keep existing component/button/callback types source-compatible; new request schemas accept documented numeric/string aliases and server-assigned component IDs. CLI/MCP rich send/edit/history retain the new data.
- Add opt-in typed select/modal/slash-interaction handlers and fallible cog hooks. Extended workers advertise protocol v7; existing v4/v5/v6 modes remain supported. Slash interaction registration is explicit and uses the same transactional command-publication path.
- Enforce initial-response-only modals, callback expiry and source-message requirements for updates. Retain Uncertain after ambiguous callback failures instead of reopening the response slot. Cover validation bounds, patch nulls, worker causes, mixed/duplicate events and ordered checkpoints over polling and WebSocket fixtures.

## 0.1.20 — 2026-11-03

- Add opt-in `AvailabilityPolicy` for startup identity/cursor reads and event intake. Bound attempts, elapsed time and jittered backoff; recover transient 520/521/522 outages while keeping authentication, configuration, decode and exhausted client wait budgets terminal. Cancel reads/backoff through `BotHandle::stop`, with structured exhaustion causes and separate cumulative telemetry.
- Give availability recovery one retry budget by suppressing nested client replays inside its reads. Keep normal API retry settings, ordered checkpoints, handler completion and `max_retry_wait` behavior unchanged. Preserve gateway handshake statuses and retry hints under the policy.
- Add fresh `RoleHierarchyPolicy` assessments for requester and bot independently. Default to Unknown for undocumented hierarchy facts; an explicit owner/admin policy denies administrator peers and requires other targets to be strictly below the actor. Reject protected roles and conflicting scopes; leave rollout eligibility to trusted host evidence.
- Add `Api::request_once` and validated role-deletion composition with fresh checks and no automatic DELETE replay. Workers opt into protocol v6 with `single_attempt_api`; existing worker defaults remain supported. Keep structured authorization errors, retry hints and uncertain response outcomes across callbacks and workers. Include native/worker examples and policy documentation.

## 0.1.19 — 2026-11-03

- Reuse canonical operation schemas during validation and permanent-command binding. Schemas initialize per operation on demand; exported schema values remain independent and editable.
- Deserialize built-in collectors and DM events directly from borrowed JSON, avoiding intermediate payload-tree copies while preserving DM normalization and scope rejection.
- Remove temporary vectors and template strings from rate-limit key/send classification. Route redaction, bucket scope and retry behavior remain unchanged.
- Remove the unused guide-only schema supplement. Add an opt-in compact local build profile without debug symbols or incremental caches; existing dev/test and application workspace profiles are unchanged.

## 0.1.18 — 2026-11-03

- Add an opt-in server-message gate before prefix resolution, group help, cog checks and native/worker commands. Continue, Handled and Stop outcomes distinguish fallthrough, completed rejection and incomplete processing.
- Add fallible EventHandler/Cog message listeners and a separate message-failure callback. Preserve typed rejection replies, original causes, partial receipts and retry hints; failed replies retain both errors. Legacy message hooks and command/button failure APIs remain supported.
- Add WorkerCog::on_message with explicit protocol v5 outcomes. Existing worker handlers retain protocol v4. Missing completion outcomes leave an uncertain cursor barrier instead of becoming implicit success.
- Check public Rust API compatibility against the latest published crate in CI, including optional features.

## 0.1.17 — 2026-11-02

- Add multipart message uploads over Client/ApiExt, native contexts and worker proxies. Validate four-file/10 MiB-per-file/12 MiB-total limits; retain every confirmed message and turn partial 201 responses or missing per-file confirmations into PartialWrite receipts. Files cross workers in bounded 64 KiB chunks.
- Add custom emoji CRUD with moderation-pending results, bot-authenticated webhook management/posting, public version/changelog reads and API/build response-header observations. Refresh the upstream OpenAPI to 1.2.0 and replace the guide-only schema supplement with the upstream definitions.
- Dispatch typed DMs separately from server commands; retain attachment and DM reaction metadata. Fallible DM and cog reaction hooks propagate failures to ordered checkpoints. Worker protocol v4 carries the new events, public reads and upload transfers.
- Add 12 shared Rust/CLI/MCP operations (56 total), scoped emoji/webhook references, independent requester/bot checks, operator-only filesystem uploads and bounded inline MCP data. Public discovery needs no token.
- Bound total advised/shared rate-limit waits to a configurable 30 seconds; retain status and Retry-After when the budget cannot fit. Bot-webhook sends participate in global message waits. Preserve image proxy metadata in responses while stripping it from typed outbound payloads.
- Avoid whole-spec copies for focused schema discovery, per-template route allocations, unused request-notification clones and upload-buffer copies on retries.

This is the one-time compatibility transition described in README. Changed response types and event/error/capability enums are extensible; update old struct literals/exhaustive matches and rebuild parents/workers together. Future compatible updates preserve the public API and defaults; later breaking changes require a new Cargo compatibility line.

## 0.1.16 — 2026-11-01

- Return `HotReloadFailure` for failed worker candidates, with compilation/startup/protocol/registration/publication phases, bounded compiler codes/messages/spans, observed exit status, timeout and candidate/publication state. Preserve reports in remote errors, failure callbacks and watcher status.
- Drain Cargo stderr without forwarding raw build-script output. Omit rendered diagnostics/source excerpts, redact environment values and path-like message tokens, and expose project-relative source locations. Report truncation rather than silently losing bounded diagnostics.
- Add an offline diagnostics example and document explicit completion of a rejected reload. Failed candidates remain unexpected errors by default; no build, handler or uncertain publication is automatically replayed. Compilation may execute trusted build scripts/macros and is not a sandbox.

Exhaustive error matches must handle `YappinError::HotReload`. `WatchStatus` adds `reload_failure` and retains `PartialEq` rather than `Eq` because structured retry hints are floating-point values.

## 0.1.15 — 2026-11-01

- Add command/button failure callbacks returning `Default`, `Handled` or `Stop`. Typed input/check rejections complete after a successful reply; definition errors and unexpected failures stay unprocessed by default. Legacy `()` callbacks remain observers. Failed replies retain original and handling causes, including partial reply receipts.
- Route typed worker parse errors through the parent policy and carry `command_kind` in structured errors. Preserve HTTP status, retry waits, authorization reports and partial writes. Lost worker completion, timeouts and unwinding panics have an explicit uncertain outcome; handlers are never retried automatically.
- Checkpoint the ordered completed event prefix instead of discarding all progress in a failed batch. Stop starting work after an unprocessed failure, drain active handlers, and report failed, not-started and completed-but-uncommitted events alongside intake/cursor errors. Graceful stop drains accepted work; cancellation does not complete unfinished events.
- Restrict polling intake retries to transient read failures. Authentication, permanent status, decode, preparation and cursor-write failures remain outside command completion callbacks. Cooldown checks no longer construct rejected handlers or ignore failed rejection replies.

Exhaustive `YappinError` matches must handle `EventProcessing`, `FailureHandling` and `HandlerInterrupted`. Rebuild workers for rejection subtypes; unclassified legacy command errors are not assumed to be input rejections. See [handler failures](docs/handler-failures.md) for callback migration, cursor boundaries and recovery limits.

## 0.1.14 — 2026-10-31

- Fix source-watcher races when files or directories disappear during scanning. Ignore disappearing non-source entries, retain required `watch_path` roots, and include path and phase in scan errors. Retry interrupted source snapshots up to three attempts; missing required inputs and access failures still surface.
- Expose `WatchGuard::status()` with separate scan and candidate-reload errors. Healthy scans clear recovered scan errors without requiring a changed digest; failed builds, startups and publications remain visible until a changed candidate installs. Active workers and routes remain installed on failure.
- Add `create_channel`, `edit_channel`, `delete_channel`, `create_category`, `edit_category` and `delete_category` to the shared Rust/CLI/MCP catalogue. Reuse typed request bodies, separate requester/bot Manage Channels checks, verify category/role references, and retain unknown effective-access checks. Channel edits preserve explicit null category removal and false booleans; uncertain writes are not replayed.
- Add `EditCategoryRequest` and validate the documented 0–21600-second slowmode range in typed channel edits as well as bindings. Permanent commands derive parsing/help from these same schemas and retain registration preflight and transactional replacement.

Exhaustive matches on `ExternalCheck` must handle `CategoryPermissions`.

## 0.1.13 — 2026-10-31

- Document capability bindings at `docs/capability-bindings.md`, with operation discovery, scoped execution, authorization policies and permanent-command examples. Update README and rustdoc links to the public guide. No runtime or API changes.

## 0.1.12 — 2026-10-31

- Keep scheduled-event destination access separate from server-scope verification in capability bindings. Matching channel/server IDs do not establish requester or bot access; require host evidence for both principals before creation, with regression coverage.

## 0.1.11 — 2026-10-31

- Move the 38-operation catalogue into the default-feature Rust library. CLI/MCP consume the same names, schemas, permission rules and availability metadata; filtered inventories can also validate incoming operation choices.
- Add validated bindings over `Api`, with host-owned server scope and separate requester/bot permission reports. Check referenced resources, retain unknown channel/hierarchy/availability checks, and require explicit host evidence before execution. Uncertain mutations are not replayed; split-send receipts remain structured.
- Add serializable permanent-command bindings that derive parsing and help from operation argument schemas, preflight ownership, and install through existing cog replacement. Parameters cannot overwrite fixed arguments. Existing typed commands and worker protocol remain compatible.
- Share bounded output and lookup engines with the library bindings. Sanitize model-facing results and access-report causes without adding model or database dependencies.

Exhaustive matches must handle `YappinError::OperationAuthorization` and `ArgumentType::Schema`. Named CLI/MCP calls now validate rich payload schemas before dispatch. See [capability bindings](docs/capability-bindings.md) for public APIs, access limits and integration examples.

## 0.1.10 — 2026-10-31

- Add typed role create/edit/delete, @everyone permissions, channel/category overrides, live member counts, invite lists and scheduled-event APIs from the developer guide. Yapin currently restricts these endpoints to test servers.
- Preserve online counts and invite attribution. Dispatch invite create/delete events to handlers, cogs, collectors and hot workers. Permission checks recognize the new role `permissions` dictionary, including explicit false values.
- Expose 14 corresponding CLI/MCP operations with typed inline bodies, dry runs and bounded list output. Supplement schema discovery with clearly marked guide-derived definitions; refresh the upstream biography limit to 300 characters.
- Invalidate member/role lookup caches after role and category-permission writes, including failed writes whose server outcome may be unknown. Custom cache backends retain ownership of storage and expiry.
- Share server permission evaluation across command checks, arbitrary members and the bot. Handle owners, assigned Administrator grants, @everyone and permission-field precedence consistently; reject incomplete role snapshots. Native and worker lookups honor the configured cache backend.
- Add a common `Api`/`ApiExt` interface for clients and native/worker contexts. Worker protocol v3 retains structured status, retry/outcome details and partial-write receipts while accepting v1/v2 workers. Command/button failures preserve their status instead of becoming configuration strings.
- Add serializable runtime command schemas with bounded argument parsing, generated help and existing checks. Expose registration preflight for aliases, ownership and advertised-command limits; reject known cog conflicts before lifecycle hooks run.
- Count HTTP executions and attempts independently of watch subscribers. Add sequenced attempt events with explicit lag reporting and account for cancelled calls/waits. Bounded member searches now distinguish unfinished scans from unique matches; CLI/MCP search tools expose candidates and completion.
- Retry transient executable-busy errors during worker startup with a bounded 310 ms backoff. Other spawn failures remain immediate; the current cog stays installed until replacement succeeds.

New optional model fields, invite enum variants, `Remote` and `IncompleteSearch` errors require updates to manual struct literals and exhaustive matches. Rebuild workers with the updated host. Bounded resolvers can now reject previously accepted but unproven matches. See [server management](docs/server-management.md) and [shared SDK workflows](docs/sdk-workflows.md) for examples and migration details.

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
