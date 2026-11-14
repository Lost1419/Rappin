# Shared webhook, polling and gateway ingress

Shared ingress is opt-in with `BotBuilder::shared_ingress(IngressConfig::default())`.
`BotHandle::ingress()` returns its cloneable `EventIngress`. Wait for `wait_ready()`
after starting `Bot::run`; then use `admit(VerifiedEvent)` for authenticated webhook
events. The normal polling pipeline remains unchanged when this option is absent.
Legacy `Bot::process_event` remains an independent trusted-host dispatch API and
must not be mixed with shared ingress for the same delivery stream.

`webhook::verify_bot_event(secret, expected_bot, received_bot, signature, raw_body)`
checks both HMAC and bot identity. The optional `webhook-middleware` feature also
inserts this verified type alongside the existing `Event` extension when its
configured bot ID is a UUID. The ingress checks it against `/me`'s authenticated
identity. Invalid IDs and conflicting retained payloads fail before event effects.

## Admission, effects and HTTP acknowledgement

`Admission::{status, receipt, error}` separates admission from handler completion.
Statuses are `Accepted`, `Duplicate`, `Full`, `Stopped` and `Error`.
`EventReceipt::{status, wait}` reports event state, durability, commit state and the
original structured failure. `EventIngress::receipt(id)` retrieves a retained
receipt, including after an admission error. Handler/check callbacks keep their
existing failure classification, reply ownership and explicit Handled/Stop policy.
A completed rejection is not replayed; failed replies and uncertain effects remain
incomplete. Consumer failure callbacks can persist their own reconciliation/audit
data before returning Handled.

Admission reserves the existing bounded queue and claim before any cache update,
collector delivery, interaction tracking/automatic acknowledgement, listener or
native/worker command effect. Intake preparation occurs before handler permits so
collectors can receive answers while a command occupies the sole handler slot.
`Started` therefore means event effects may have begun, not that its command body
has necessarily run. This also protects automatic acknowledgements from duplicate
delivery. Admission does not await a command, but can perform bounded receipt/intake
preparation. Its overall wait is at most two seconds or the configured persistence
timeout, whichever is shorter. Application collector filters must remain nonblocking.

An HTTP adapter should answer promptly: 202 for accepted/active duplicates, 503 for
full/stopped/unavailable admission, and an explicit conflict/reconciliation response
for conflicting or failed claims. It must inspect the receipt's durability.
Without `IngressStore`, acceptance is **volatile**, even if a cursor file is used.
Do not describe a 202 as durable processing or completed execution. Cancellation
of admission after intake effects start stops intake with a retained failed/uncertain
receipt; inspect it before any
explicit resubmission. An Error never authorizes replay of an uncertain mutation.

The default bounds are 4,096 retained claims, 1 MiB per event and a two-second
persistence timeout. Set `IngressConfig::{max_claims, max_event_bytes,
persistence_timeout}` before building; invalid bounds fail startup. Queue bounds
and handler concurrency are the existing `pending_batches` and
`handler_concurrency` settings. A reserved polling read occupies one queue slot;
choose at least two slots to admit webhooks during an outstanding long poll.
Claim exhaustion returns Full to webhook callers and stops polling/gateway intake
with an explicit capacity error; unresolved claims are never evicted. `status()`
provides cumulative admission/dedup/conflict/full counts and current retained,
unresolved and completed-but-uncommitted counts.

## Ordering and polling checkpoints

Claims are scoped to the authenticated bot and event ID. Concurrent arrivals share
one claim; duplicates of admitting/running/completed/failed work execute no further
effects. Conflicting payloads are compared using a canonical event fingerprint.
Resolved claims at/below a confirmed checkpoint can be retired to bound memory.
An older duplicate then reports `payload_retained: false`: its payload can no longer
be compared, and it is discarded under the authoritative committed cursor.

Webhook and gateway IDs do not advance polling checkpoints. Shared ingress always
runs authoritative polling recovery; optional GatewayPreferred delivery joins the
same claims. GatewayOnly is rejected with shared ingress because it cannot provide
that polling authority. Sparse IDs are allowed: no assumption of consecutive IDs
is made. A higher webhook can complete while earlier polling events remain unseen;
it stays completed-but-uncommitted until polling covers it. A lower unfinished
claim fences a checkpoint, even when it was admitted outside the polling batch.
The admission/checkpoint fence prevents a concurrent claim crossing cursor
publication. Handler ordering follows admitted batches and the configured executor;
webhooks are not reordered into a fabricated globally ordered event stream.

Stopping closes admission, cancels intake and drains already accepted executor
work. Queued work not executed remains explicit; if intake effects already began,
it retains an uncertain barrier. Abrupt task cancellation also marks active receipts
uncertain. Failed cursor/receipt persistence stops intake with structured causes,
retains completion evidence and never reruns a handler. This provides bounded
at-most-once execution within a live claim ledger, not a cross-process exactly-once
promise.

## Consumer-owned durable receipts

`BotBuilder::ingress_store(Arc<dyn IngressStore>)` enables shared ingress using an
application-owned implementation. The asynchronous interface is:

- `load(bot_id) -> Result<IngressRecovery>` returns the polling checkpoint and
  uncommitted `IngressRecord`s.
- `record(IngressRecord) -> Result<()>` durably records the full event and receipt
  transition. Queued is persisted before admission; Started is persisted before
  any intake/handler effects; Completed/Failed follows processing.
- `checkpoint(bot_id, cursor) -> Result<()>` durably advances the authoritative
  polling checkpoint. Only then may records below it be pruned.

Records include original structured errors and partial receipts. Storage failures
have `kind: ingress_storage`, a phase and source cause. Store operations must be
atomic/durable as promised, retain identity and uncommitted outcomes, and return
consistent snapshots. Use one active runner per bot/store; this interface does not
provide a distributed execution lease. Storage implementation and schema remain
consumer-owned.

An empty store is initialized from the configured cursor file (or zero) before
admission opens. Afterwards its checkpoint is authoritative; a cursor file is a
mirror. A file may advance just before store checkpoint persistence fails, but
restart still uses the older durable checkpoint and completed records to avoid
executing them again. Missing checkpoints alongside stored records are rejected.
Completed records suppress replay; only durable Queued/Unprocessed records known
to precede Started can resume automatically. Started/Failed records block startup
until the consumer reconciles them. No uncertain write or handler is retried.
Persistence cannot atomically transact arbitrary external API effects with a local
receipt; a crash after Started requires reconciliation.

## Runnable loopback example

[shared_webhook_polling.rs](../examples/shared_webhook_polling.rs) combines the
signature middleware, bounded asynchronous ingress, polling recovery, a cursor file
and graceful shutdown. Run:

```sh
cargo run --features webhook-middleware --example shared_webhook_polling
```

Set `YAPPER_BOT_TOKEN`, `YAPIN_BOT_ID`, `YAPIN_WEBHOOK_SECRET` and optionally
`YAPIN_CURSOR_FILE`. It listens on `127.0.0.1:8080/webhook` and explicitly reports
volatile acceptance. For production restart protection, supply your own durable
store. DNS/tunnels, hosting, storage engines and application command/approval
policies are outside the SDK. Native ingress needs no optional feature; gateway
delivery adds `gateway`, workers add `hot-reload`. Existing worker protocol versions
and failure contracts remain compatible.

## Payload conflicts and bounded recovery

Yapin documents gateway `d` as exactly one `/events` item. The public event model
contains `id`, `type`, `data` and outer `created_at`. The outer timestamp and a
`data.created_at` value are distinct fields; the SDK does not substitute one for
the other. A missing/null outer timestamp versus a supplied timestamp remains a
conflict. No transport-specific requester, server, message, command or content
change is accepted as equivalent.

Fingerprints are SHA-256 of recursively key-sorted serialized `Event` JSON.
Object property order is immaterial, even when a consumer enables serde_json's
`preserve_order`. Existing typed outer timestamp parsing compares equivalent
instants consistently. Array order, scalar values, missing versus null payload
fields, and different event kinds or identities remain meaningful. Existing
`Event` deserialization/legacy aliases are unchanged. Restoring an old receipt
recomputes its evidence without rewriting the event or completion.

`Error::IngressConflict { conflict }` has JSON `kind: ingress_conflict`.
`Error::ingress_conflict()` recovers its report from native, remote and
pipeline errors, including nested persistence failures. `IngressConflict` exposes:

- `bot_id`, `event_id`, `existing_source`, `recovered_source`, `incoming_source`;
- `existing_fingerprint`, `incoming_fingerprint`, `retained_stage`, `disposition`;
- `differences` (JSON pointers and existing/incoming `PayloadType`s) and
  `differences_truncated`.

`IngressSource` distinguishes Webhook, Gateway, Polling and Recovery. Old stored
records have unknown original provenance; reports say Recovery with no
`recovered_source`, rather than inventing a transport. The original source is
retained in new `IngressRecord::source: Option<IngressSource>` values.

Reports contain no scalar values, source excerpts or arbitrary object keys.
Unknown keys are rendered as `*`; array indices are retained. Evidence is bounded
to 128 schema nodes and 16 levels per claim, with at most 32 reported differences.
A truncated comparison explicitly says so; it never implies equality. The claim
ledger retains bounded hashes/schema evidence rather than a second raw payload.

`EventIngress::conflicts()` returns bounded reports, including resolved ones.
`IngressStatus::{conflicts, pending_conflicts, conflict_reports_dropped}` exposes
cumulative conflict count, currently fenced claims and retired diagnostic count.
`IngressConfig::max_conflict_reports` defaults to 32 (1–1024); unresolved reports
are not evicted. Exhaustion stops intake with an explicit error. Claims remain
subject to their existing independent capacity bound.

### Default and opt-in disposition

The default `IngressConflictPolicy::Stop` rejects contradictory payloads and
halts intake. Fast-path webhook conflicts now also signal this failure to the
pipeline; older versions returned only the HTTP admission error on that path.
A webhook error retains the original receipt in `Admission::receipt`. No policy
runs a replacement handler, modifies the authenticated context, changes a
completed receipt to failed, or marks an uncertain effect completed.

For an already Completed claim, applications can explicitly configure:

```rust,no_run
use rappin::{Bot, IngressConfig, IngressConflictPolicy};
use std::time::Duration;
let mut config = IngressConfig::default();
config.conflict_policy = IngressConflictPolicy::RetainCompletedAndPoll;
config.conflict_recovery_timeout = Duration::from_secs(60);
config.max_conflict_reports = 32;
let bot = Bot::builder("read-token-from-your-private-configuration")
    .configure(|settings| settings.persistence.ingress = Some(config))
    .build().expect("valid bot configuration");
```

A conflicting webhook/gateway delivery still receives an explicit conflict error
and is recorded as `AwaitingPolling`. The original Completed receipt stays intact;
no event effects are repeated. The checkpoint is fenced before that claim. The
poll reader revisits the unresolved range, waiting one second between readbacks
when necessary, until polling supplies the original canonical payload. That
read is deduplicated against the existing completion, the disposition becomes
`PollingConfirmed`, and ordinary ordered checkpoint persistence can proceed.
If polling already established that exact payload, the completed claim can retain
that proof without another read. Contradictory polling payloads always stop.

An empty poll or a higher `last` does not prove payload equivalence. The polling
watermark can advance while the committed cursor remains fenced. Readback never
forces a cursor forward and never invokes a completed handler. The default
recovery deadline is 60 seconds (greater than zero through 300 seconds); expiry
records `TimedOut` and stops intake. Readbacks are cancellable; graceful stop
drains accepted work, and in-flight persistence remains bounded by its configured
timeout. Neither the process nor a handler is restarted or retried.
Queued/Started/Failed/uncertain claims cannot use this completed-only policy.

### Durable conflict storage

Existing `IngressStore` implementations continue compiling. The additive hook is:

```text
async fn record_conflict(&self, conflict: IngressConflict)
    -> Result<AdmissionDurability>;
```

Its default returns Volatile and promises no storage. A durable ingress using
completed-conflict recovery requires an implementation returning Durable **after**
atomic persistence. It must load unresolved reports through the new
`IngressRecovery::conflicts` field (Serde defaults to empty for legacy snapshots).
Use bot/event ID, incoming source and the two fingerprints to identify a conflict;
persist disposition updates atomically. Return consistent receipt/checkpoint/
conflict snapshots. Resolved reports may be pruned once their checkpoint is durable.

Conflict records are separate from `IngressRecord`; recording or resolving a
conflict must preserve the original event, completion and mutation receipts.
Persistence failure retains both the conflict and storage error under
`FailureHandling`. An interrupted evidence write retains a failure barrier.
A failed checkpoint retains completion/confirmation evidence for the next run.
No failed store operation authorizes another handler or API mutation.

On restart, the explicit recovery policy can reopen a stored non-polling conflict
only when its fingerprint matches the retained Completed receipt. A matching
polling payload is still required. Unresolved contradictory polling records,
missing receipts, conflicting stored identities and uncertain effects remain
blocked. Volatile mode provides no cross-process guarantee.

### Generic receiver example

The existing loopback receiver prints only bounded conflict reports on shutdown
and returns structured conflict metadata without raw payloads. Run the default
strict policy with the command above, or explicitly enable completed recovery:

```sh
cargo run --features webhook-middleware --example shared_webhook_polling -- \
  --retain-completed-and-poll
```

It still advertises volatile admission. Supply an application-owned durable store
with the hook/load contract above for persistent recovery. The core APIs require
no optional feature; the Axum receiver uses `webhook-middleware`, gateway delivery
uses `gateway`, and worker handlers use `hot-reload`. Worker protocols remain
unchanged. Storage engines, hosting and application reconciliation decisions
remain consumer-owned.

See [the event contract reproduction](event-contract-conflicts.md) for the exact
upstream guarantees needed when transports disagree. A missing live competing
payload cannot be reconstructed from a legacy fingerprint-only error.

### Explicit legacy timestamp reconciliation

A completed legacy null outer timestamp can be approved against one fresh polling
fingerprint through the separate [timestamp reconciliation API](timestamp-reconciliation.md).
This requires atomic consumer storage and explicit approval. It preserves original
events/completions, waits for matching normal polling, and never replays handlers
or forces cursors. Other payload/identity conflicts remain terminal.

### Admission authority and failed-work assessment

[Draining and operator retirement](draining-and-retirement.md) documents
`IngressAdmissionPolicy::PollingOnly`, response-pump lifetime, explicit shutdown
bounds and the separate failed/uncertain-work operator API. The Completed timestamp
API remains narrow; neither a source-policy change nor an empty poll silently
reconciles a failed mutation.
