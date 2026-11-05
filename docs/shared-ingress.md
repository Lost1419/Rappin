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

Set `YAPIN_BOT_TOKEN`, `YAPIN_BOT_ID`, `YAPIN_WEBHOOK_SECRET` and optionally
`YAPIN_CURSOR_FILE`. It listens on `127.0.0.1:8080/webhook` and explicitly reports
volatile acceptance. For production restart protection, supply your own durable
store. DNS/tunnels, hosting, storage engines and application command/approval
policies are outside the SDK. Native ingress needs no optional feature; gateway
delivery adds `gateway`, workers add `hot-reload`. Existing worker protocol versions
and failure contracts remain compatible.
