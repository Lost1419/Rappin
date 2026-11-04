# Event completion, recovery and reload

Use APIs matching the selected dependency; availability and single-attempt worker
support require Rappin 0.1.20+. These features are opt-in. Do not change transport,
retry settings or failure completion policy as an unrelated implementation choice.

## Choose features and transport

| Need | Cargo feature / API |
| --- | --- |
| Native commands, API, bindings, availability | Default library, no optional feature |
| WebSocket with REST fallback | `gateway`, `.gateway(rappin::gateway::DEFAULT_GATEWAY)` |
| WebSocket without REST fallback | `gateway`, `.gateway_only(...)` |
| Source reload while parent keeps running | `hot-reload`, `WorkerProject` / `HotReload` |
| CLI/MCP executable | `cli` |
| Encrypted voice playback bridge | `voice`, with separately configured runtime |

Polling remains the default. REST header time, whole-call/body time, gateway
heartbeat RTT and end-to-end message latency measure different boundaries.

## Gate before commands; complete failures explicitly

Use `BotBuilder::message_gate` for a shared fallible server-message decision
before prefix/help/checks and native/worker command dispatch. Per-cog Boolean
checks cannot replace it. Gate context has not resolved command/args/argv yet;
validate server scope from the original event. DMs use their separate hooks.

`MessageDisposition::Continue` falls through, `Handled` completes the message and
skips remaining dispatch, and `Stop` leaves it incomplete with a cursor barrier.
For required natural-language I/O, use `EventHandler` / `Cog::on_message_result`
or `WorkerCog::on_message`; legacy `on_message -> ()` cannot propagate failures.

`on_command_failure`, `on_button_failure` and `on_message_failure` return explicit
`FailureDisposition` values. Typed `CommandError` input/permission/clarification
rejections complete after a successful reply. Failed replies and unexpected
errors remain incomplete by default. `Handled` must mean the invocation was
completed or reconciled, not merely logged. Keep both causes when error handling
itself fails; never relabel an uncertain write as an input rejection.
A Handled listener failure completes that listener invocation and lets remaining
listeners run; a listener returning `MessageDisposition::Handled` claims the rest
of dispatch. A handled gate failure never falls through to commands.

The cursor advances only across the ordered completed prefix. Already-started
concurrent handlers drain; set `handler_concurrency(1)` for strict ordering.
`BotHandle::stop()` gracefully drains accepted work; a message Stop is incomplete.
Cancellation and failed cursor persistence do not make unfinished effects safe to
replay. Custom `process_event` callers own deduplication and checkpoints. Retain
application-owned durable receipts for at-least-once delivery across restarts.

## Read availability

Opt into `BotBuilder::availability_policy(AvailabilityPolicy::default())` when
bounded startup/intake recovery is wanted. Customize attempts, elapsed deadline,
backoff and jitter. Allow more than the normal 25-second poll duration in the
elapsed budget. `BotHandle::stop()` cancels reads/backoff.

Only identity/cursor startup reads and event intake are covered, including
transient edge 520/521/522 errors. Authentication/configuration/decode and exhausted
client wait budgets stay terminal. The policy owns attempts and suppresses nested
client replays. One episode shares `max_retry_wait` for advised/shared waits;
additional outage backoff uses the separate elapsed budget. Successful reads end
an episode. Never wrap handlers or mutations in an availability retry loop.

Observe `availability_status()` / `subscribe_availability()` and recover
`AvailabilityFailure` via `availability_failure()`. Snapshot notifications can
coalesce; cumulative counters retain attempts/retries. Normal API retries and
existing defaults remain unchanged outside those opt-in reads.

## Reload Rust sources

Native `reload_cog` replaces instances of already-linked code; it does not load
new machine code. `cargo-watch` restarts the process. For source reload without
restarting the parent, put a cog in a separate `WorkerCog` executable and use
`WorkerProject` / `HotReload`. Parent credentials stay in the parent. Worker state
resets; preserve durable state in the application's storage.

Worker protocol requirements are opt-in: existing handlers use v4,
`WorkerCog::on_message` uses v5, and `single_attempt_api()` requires v6. Upgrade the
parent before enabling newer worker APIs. Do not combine the legacy message
listener with the new fallible message hook in the same worker. Worker stdout is
the protocol stream; use stderr for sanitized diagnostics.

Keep the watcher guard alive. Inspect `WatchGuard::status()` fields `scan_error`,
`reload_error`, and `reload_failure`. Scan recovery clears only the scan error;
failed candidate compilation/startup/protocol/registration/publication remains
visible until a changed candidate installs. Failed candidates keep active routes.
`hot_reload_failure()` exposes bounded codes/messages/spans, exit/timeout state,
candidate startup and publication facts. Local unpublished routes do not prove
that an attempted remote publication or build had no effects.

Workers, Cargo build scripts and procedural macros are trusted executable code,
not a sandbox. Do not trigger repeated builds of untrusted source as diagnostics.
The version-matched `message_policy_guide`, `handler_failures_guide`,
`availability_and_hierarchy_guide` and `hot_reload` Rust APIs provide full contracts;
a checkout also includes `examples/message_gate.rs` and hot-reload examples.

## API 1.3 UI events (0.1.21+)

Use `UiMessageBody` / `UiApiExt` for lossless select messages and typed modal forms.
`on_select`, `on_modal`, or `on_command_interaction` opt into extended event routing;
worker `on_ui_interaction` requires protocol v7 and parent `interactive_events(true)`.
Slash interaction mode is separate from existing MessageContext commands and their
server-message gate; enforce the application's authorization inside UI handlers.
A modal must precede acknowledgement or any other response. Match custom IDs,
retain typed submitted values and propagate failures through the existing explicit
completion policy. See the version-matched `api_1_3_guide` and included forms example.
