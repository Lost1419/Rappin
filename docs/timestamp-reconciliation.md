# Explicit reconciliation of a completed legacy timestamp

Use this maintenance API when a durable **Completed** server-message event has
outer `created_at: null`, but fresh authenticated polling returns a timestamp.
It records a consumer-approved exception for exactly two canonical fingerprints.
It does not assert that Yapin timestamps are generally interchangeable. Normal
intake and the default Stop conflict policy retain their existing behavior.

The original Event, completion, source provenance and checkpoint are not rewritten.
There is no handler replay, mutation retry, timestamp inference or cursor override.
This mechanism cannot reconcile Started, Failed, uncertain, volatile, committed or
missing receipts, interaction events, or changed message content/identity.

## Public API

All APIs are exported from `rappin` and `rappin::bot`, with **no optional feature**.
Maintenance runs in the host with a `Client` and the consumer's durable
`IngressStore`. Existing store implementations still compile; approval fails
closed until they implement the new atomic hook. Workers and their handlers
do not participate in this host-side maintenance API.

```rust,no_run
use rappin::{Client, IngressStore, OperationId, Result, TimestampReconciliationRequest, prepare_timestamp_reconciliation};
use std::sync::Arc;
use uuid::Uuid;

async fn approve_one(
    client: &Client, store: Arc<dyn IngressStore>, bot: Uuid, event: u64,
    retained_sha256: String, polling_sha256: String,
) -> Result<()> {
    // Hold the consumer's exclusive runner/store lease; intake must be stopped.
    let request = TimestampReconciliationRequest::new(
        bot, event, retained_sha256, polling_sha256,
    );
    let prepared = prepare_timestamp_reconciliation(
        client, store, request, Default::default(),
    ).await?;
    let evidence = prepared.evidence(); // state Prepared; no durable approval yet
    assert_eq!(evidence.event_id, event);
    // The consumer evaluates its own human/administrative approval policy here.
    prepared.approve(OperationId::new("maintenance/approved-change-1")?).await?;
    Ok(())
}
```

- `TimestampReconciliationRequest::new(bot_id, event_id, retained, polling)` binds
  the authenticated bot/event and two expected lowercase SHA-256 fingerprints.
- `TimestampReconciliationOptions` bounds each operation (default 10 seconds,
  maximum 120), preparation lifetime (default/maximum 300 seconds) and polling
  `max_age` (default 86400; `None` omits the filter). `polling_wait` defaults to **1
  second** and accepts 0–25. A zero-wait snapshot can omit available historical
  events, so an empty result does not establish expiry. `preparation_timeout`
  optionally caps the entire read-only preparation (maximum 360 seconds); `None`
  derives three times the per-operation timeout, or 30 seconds by default.
- `prepare_timestamp_reconciliation(&Client, Arc<dyn IngressStore>, request,
  options)` reads the store, fresh `GET /me`, and `GET /events` with `wait=1` by
  default and `after=event_id-1`. This is a stateless evidence probe, never a persisted cursor
  change. Event 1 is unsupported because `after=0` bootstraps an empty snapshot.
  Each API read uses a single client attempt, including transient HTTP errors.
  There is no zero-wait fallback or retry. Per-operation and overall deadlines
  cancel a pending poll even when its server wait is longer. Dropping preparation
  has no effects; read failures/deadline exhaustion write no decisions or cursors.
- `PreparedTimestampReconciliation::{evidence, approve}` exposes content-free
  evidence, then commits once with a bounded consumer audit reference (`OperationId`).
  Expired preparations and changed receipts, reports or checkpoints are rejected.
- `TimestampReconciliation` is the serializable audit decision. Its
  `validate_write(&IngressRecovery)` validates an atomic approval/confirmation
  transition, including original receipt digest and exact fingerprints.
- `IngressRecovery::timestamp_reconciliations` defaults to `[]` for older stores.
  Load approved and confirmed decisions alongside their retained records/reports.
- `IngressStore::commit_timestamp_reconciliation(decision)` is the only new
  persistence hook. Its default returns an unsupported error.

Preparation compares **all canonical SDK Event fields** (`id`, `type`, `data`,
outer `created_at`), not just the bounded diagnostic paths. Every nested data
field and JSON value/type must match. Only outer null→timestamp is permitted;
the timestamp must actually occur in the polling response and match the expected
polling fingerprint. Missing/null handling and timestamp instant canonicalization
follow the existing Event decoder; discarded raw wire fields cannot be recovered.
Require valid server/channel/requester/message UUIDs, a matching durable receipt,
and an untruncated `/created_at` Null→String conflict report. Original transport
provenance may be unknown in a legacy receipt; it remains unknown in the audit.

## Atomic storage contract

In the same exclusive transaction that inserts/transitions the decision:

1. Load the current checkpoint, original Event/completion, conflict reports and
   decisions. Call `decision.validate_write(&current)` **inside that transaction**.
2. Insert an Approved decision only if the bound receipt, reports and checkpoint
   still match preparation and no competing decision exists. Confirmation must
   transition that same Approved decision to Confirmed; immutable fields cannot change.
3. Durably commit the decision separately. Preserve original Event/completion and
   conflict evidence. Do not update the checkpoint or rewrite a conflicting payload.

A load followed by an unconditional write does not close the race. The adapter
must enforce the transaction/lease itself; this trait is not a distributed lease.
Identical writes validate idempotently for adapters, but the SDK does not retry an
ambiguous commit. Its error may follow a successful durable write. Reload the
journal/decision before determining the next action. Cancellation has the same rule.
Approval IDs are audit references, not remote idempotency keys or permission grants.

On restart, an Approved decision opens intake with a checkpoint barrier. Conflicts
show `AwaitingReconciliationPolling`. Webhooks, the original null-timestamp payload,
empty polls and higher watermarks cannot release it. Normal intake must receive
the exact approved polling fingerprint and durably transition the decision to
Confirmed **before** clearing the barrier (`ConflictDisposition::Reconciled`).
The regular ordered polling/checkpoint mechanism then progresses; this API never
sets its cursor. Any other payload remains a conflict and stops intake.

If confirmation persistence fails or is cancelled, the completion stays retained
and the barrier stays closed. If the commit succeeded but its response was lost,
restart loads Confirmed evidence and still uses normal polling coverage to commit.
A failed checkpoint write retains the original completion and confirmed decision;
restart does not rerun the handler. Authentication, intake and storage errors keep
their normal failure behavior. Remote errors preserve phase, cause, HTTP status,
retry hints and a do-not-replay marker. Approval/confirmation write errors report
an unknown commit outcome; this does not turn a Completed handler into uncertain work.

While a receipt remains loadable above the checkpoint, retain its decision and
reports too. At durable checkpoint pruning, atomically archive the original
Event/completion and decision/audit evidence before removing their active recovery
entries. Do not leave an active decision without its required receipt. Consumer
retention, approval, storage and process supervision remain outside the SDK.

## Runnable consumer

[reconcile_event_timestamp.rs](../examples/reconcile_event_timestamp.rs) supplies
a small Unix JSON journal adapter with an exclusive lease, a mutex, fresh disk
reads, validation inside the transaction, fsync + same-directory atomic rename,
and an append-only audit containing both decision and original receipt. It is an
example for consumers already using that journal as their source of truth, not a
database export/import procedure. Database consumers implement the same hook in
their own transactions. The adapter fails closed on a stale crash lease and caps
the journal at 32 MiB; operators must establish lease ownership before recovery.
The SDK API itself is portable.

```sh
cargo run --example reconcile_event_timestamp -- --help
# Prepare only, with token supplied through the environment:
cargo run --example reconcile_event_timestamp --   ingress.json BOT_UUID EVENT_ID RETAINED_SHA256 POLLING_SHA256
# After the consumer's approval policy permits that exact change:
cargo run --example reconcile_event_timestamp --   ingress.json BOT_UUID EVENT_ID RETAINED_SHA256 POLLING_SHA256   --approve maintenance/approved-change-1
```

Resume with the same store and lease after approval. The example does not start a
runner or invoke handlers. Do not substitute an edited snapshot for durable state.
Its journal contains private payloads; it creates replacement files with mode0600.
It does not print payloads, tokens or environment values.

## When polling cannot supply evidence

An empty result is not proof even if `last` exceeds the retained event. Retention,
filters or expiry can make the event unavailable. The SDK then returns structured
reason `authoritative_event_unavailable` and writes nothing. A nested message
creation time or a report containing only hashes cannot establish the missing
outer timestamp.

The required platform support is an authoritative historical lookup scoped to the
authenticated bot and event ID that returns the immutable full event, or an explicit
platform equivalence/reconciliation contract binding both representations without
changing authority, content or execution meaning. The SDK currently requires the
actual event through polling; it does not accept an unimplemented guarantee as evidence.
A generic report can include:

```text
A durable Completed server-message event is blocked by a legacy outer timestamp
conflict: Recovery -> Polling, /created_at Null -> String, no truncated differences.
Retained fingerprint: <sha256>; observed polling fingerprint: <sha256>.
Checkpoint: <cursor>; event: <id>; original completion and effects remain intact.
A fresh GET /events probe after <id-1>, wait=<configured wait>, max_age=<filter> returned <IDs>, last=<id>.
Please document event immutability/representation and retention, and provide an
authoritative way to recover this exact event when polling no longer returns it.
No handler replay, receipt rewrite or manual cursor advancement has been performed.
```

Offline tests cover full payload/scope comparison, wrong fingerprints, expiry and
stale snapshots, ineligible receipts, approval rollback/lost responses, confirmation
rollback/lost responses/cancellation, failed checkpoints, restart, unchanged original
completion and zero repeated handlers/mutations. The journal example also tests
exclusive lease behavior, failed transactions and a durable reopen round trip.

Offline probe coverage includes a zero-wait empty snapshot followed by a one-second
wait returning the historical event, genuinely missing evidence, wrong authenticated
bot/fingerprints, changed content, single-attempt HTTP failure, invalid bounds,
per-operation/overall deadline exhaustion and cancellation. All leave approval and
checkpoint state untouched.
