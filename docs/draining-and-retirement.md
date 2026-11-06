# Draining handlers and retiring uncertain work

Stopping event admission and disconnecting API requests are different operations.
The ordinary and shared pipelines stop intake first, retain the gateway response
and heartbeat pump while accepted work drains, then close it. An event queue
stall/overflow suspends event delivery without destroying request correlations.
Preferred mode can use polling for event coverage; gateway-only event intake may
stop and drain instead. Submitted writes are never moved to HTTPS or replayed by
shutdown. Existing documented known-unapplied 429/503 recovery remains available;
use `Api::request_once` or worker `api_once` when every call must be one attempt.

## Shutdown contract

```rust,no_run
use rappin::{Bot, Result};
use std::time::Duration;
async fn run(token: String) -> Result<()> {
    let bot = Bot::builder(token)
        .drain_timeout(Some(Duration::from_secs(30)))
        .build();
    let handle = bot.handle();
    // A supervisor requests handle.stop(), then awaits the run future.
    // handle.is_draining() distinguishes that phase from normal event intake.
    bot.run().await
}
```

`BotBuilder::drain_timeout(Option<Duration>)` bounds draining after intake stops.
`None` preserves the previous unbounded default; `Some` must be greater than zero
and at most one hour. Individual API request deadlines still apply independently.
Graceful stop drains batches already accepted by the executor. An unexpected
handler/intake failure also halts the execution gate: handlers already running can
finish, while work that has not started remains unprocessed. No new intake batches
are accepted. Collectors close with intake; scheduled acknowledgements for accepted
interactions remain available during draining and are cancelled before final API
shutdown. The shared poller may hold one raw response while waiting for admission;
its IDs remain available as unprocessed evidence if shutdown interrupts that wait.

At the drain deadline, remaining handler futures are cancelled and return explicit
`event_drain` failure evidence with reason `drain_timeout`, outcome `unknown`, and
`retry_safety: do_not_replay_workflow`. Prior handler errors and intake causes remain
in the processing report; a second shutdown error does not replace the first cause.
Known completions remain completions, queued work is not promoted to handled, and
Started durable receipts remain blocking when a forced cancellation cannot finish
an asynchronous persistence operation.

Dropping/aborting the run future is **hard cancellation**, not graceful draining.
Cancelled futures cannot return a result or run an error callback. Shared in-memory
receipts retain interruption evidence; durable Started receipts remain uncertain
and block recovery. Ordinary intake has no durable receipt ledger: the consumer
must journal effects if it requires crash/restart reconciliation. Never restart a
process or replay a command solely because a handler was cancelled.

With feature `gateway`, `GatewayRequestStatus::last_disconnect` retains a structured
connection cause. Request error JSON keeps `kind: gateway_request`, `submitted`,
`outcome`, `retry_safety`, `reason`, HTTP/close status and retry hints where available,
plus a structured `source`. Reasons distinguish request/response/queue deadlines,
local `pipeline_shutdown`, `drain_timeout`, `run_cancelled`, `gateway_dropped`, peer
closure, I/O/TLS/reset, send failure and heartbeat timeout. A deadline is not a
claim that the platform failed to execute. Partial-write errors keep their confirmed
`completed_steps` and the failed step's original structured cause through native
callbacks and worker RPC. A cancelled future only exposes its available receipt and
connection telemetry; it does not fabricate a missing response or business receipt.

## Cross-transport identity

Keep strict fingerprint matching unless an authoritative representation contract
establishes equivalence. The public gateway contract describes `d` as a polling
item, and the bundled OpenAPI requires an outer event timestamp. This does not
establish permission to replace missing timestamps from nested message data, ignore
content/identity changes, or infer why a particular consumer's two payloads differ.
The platform needs to define immutable bot/event identity and exact canonical fields
for polling, gateway, webhook and redelivery, including optional envelope timestamps.
A captured source/path/type report identifies a difference, not its upstream cause.

An explicit supported alternative is **one admission authority**:

```rust
use rappin::{IngressAdmissionPolicy, IngressConfig};
let mut config = IngressConfig::default();
config.admission_policy = IngressAdmissionPolicy::PollingOnly;
```

`IngressAdmissionPolicy::AnyVerified` remains the default. With `PollingOnly`, only
polling admits work. Verified webhooks return `AdmissionStatus::PollingRequired`
without a claim, intake effects, durable acknowledgement, payload equivalence or
cursor change. Gateway events are not dispatched; the connection can still carry
API responses and heartbeats. Applications choose how their webhook HTTP receiver
represents that notification (the example returns 202 with `polling_required`).
Both null/non-null arrival orders therefore leave execution to the canonical polling
payload. This selects authority rather than normalizing different inputs. Within
admitted polling data, genuine identity/content contradictions still stop intake.
Existing conflicts and Started/Failed receipts are not erased by selecting this
policy. Unconfirmed early-source queued recovery fails closed rather than silently
executing it under a different source policy.

## Explicit operator retirement

The narrow Completed timestamp API remains unchanged and rejects failed/uncertain
work. The separate retirement API authorizes **no replay of one assessed execution**,
not a declaration that it succeeded. Stop the runner and hold its exclusive store
lease throughout preparation and approval.

Public core APIs require no optional feature:

- `canonical_event_fingerprint(&Event) -> Result<String>` is the canonical SDK
  content address, with a 16 MiB event bound. It is not authentication, namespace or
  remote-write proof; bot identity is checked separately.
- `EventRetirementRequest::new(bot, event, original_sha256, polling_sha256)` selects
  `ExactPollingEvent`: fresh authenticated polling must return the full target.
  It must match the original Event exactly, or only supply a previously missing
  outer timestamp. No content, actor, server, channel, message identity or existing
  non-null timestamp may change.
- `EventRetirementRequest::retained_execution(bot, event, original_sha256)` explicitly
  selects `RetainedExecutionCoverage` when historical event data is unavailable.
  It requires a saved nonzero checkpoint and fresh authenticated polling coverage
  from that checkpoint through the target. Any returned target must match the
  retained fingerprint exactly. No timestamp is inferred and no representations
  are declared equivalent.
- `prepare_event_retirement(&Client, Arc<dyn IngressStore>, request, options)` makes
  only GET/store reads. Options reuse `TimestampReconciliationOptions` read/expiry
  bounds via the `EventRetirementOptions` alias: polling wait 1 second, per-operation
  timeout 10 seconds, overall preparation budget 30 seconds by default, expiry 300 seconds.
  Reads are single-attempt and cancellable. Exact missing evidence writes nothing;
  the API never silently downgrades it to coverage-only retirement.
- `PreparedEventRetirement::evidence()` returns a Prepared proposal. `approve(
  approval_reference, assessment, evidence_reference)` commits the consumer's
  explicit assessment with bounded `OperationId` audit references. Assessments
  are `AttestedApplied`, `AttestedNotApplied`, or `AcceptUnknown`; none is SDK proof
  of a remote outcome. Coverage-only mode requires **AcceptUnknown**. Preparation
  expiry and changed receipt/error/conflict/checkpoint facts reject approval.
- `IngressStore::commit_event_retirement(EventRetirement)` defaults to unsupported.
  Its implementation must call `decision.validate_write(&transaction_snapshot)`
  **inside the same atomic transaction** that writes the separate decision and
  immutable audit. Preserve the original event, stage, error, partial receipts,
  conflicts and checkpoint. A preliminary load plus an unconditional later write
  does not close the race. `IngressRecovery::event_retirements` defaults to `[]`.
- `EventIngress::retirements()` exposes active decisions. `EventRetirementState`
  progresses Prepared→Approved→PollingConfirmed. That last state confirms event
  identity/coverage, **not mutation success**. Normal polling must persist it before
  releasing the checkpoint fence. The regular ordered checkpoint path still waits
  for other unfinished lower work; approval never sets or edits a cursor.

Exact mode requires the approved full fingerprint in normal polling. Coverage-only
mode requires a normal polling watermark through the retained execution, and checks
any returned target against the original. The original receipt stays Started/Failed
with its original error; when committed its receipt has `committed: true` without
being changed to Completed. `EventReceipt::wait` can return that explicitly retired
committed state. Atomic checkpoint pruning must archive the original and decision
before removing active records. Failed/uncertain effects remain identified as such;
retirement is permission to move on without replay under the recorded assessment.
Other unapproved failures or new conflicts still block recovery.

An approval/confirmation error, timeout or cancellation may follow a durable commit.
Load the decision/audit before determining what happened. The SDK never retries an
ambiguous durable write, replays the handler, runs its callbacks again, or searches
messages by text to manufacture a receipt. Store confirmation/checkpoint failures
retain the original failure and barrier. Consumer model planning, storage, evidence
quality, approval authority and later business remediation remain consumer-owned.

## Platform limits and consumer example

The current [OpenAPI](https://www.yapin.gg/openapi.json) says commands older than 15
minutes and expired button clicks may be skipped while the cursor advances. A
normal watermark can establish event-stream coverage; it cannot prove payload
identity or a mutation outcome. Exact mode stays blocked if the event is absent.
Coverage-only retirement is a separately approved acceptance of that limitation.

The public contract does not supply a durable mutation receipt lookup/idempotency
contract that can identify an unknown submitted write from a consumer operation ID
or a lost gateway response. A known resource ID may permit a scoped resource read,
but matching message text is not request identity and absence is not proof that a
write never executed. Where authoritative evidence is missing, retain `AcceptUnknown`
or stay stopped. Resolving that uncertainty needs platform-backed operation IDs,
idempotency semantics and receipt/reconciliation queries, including retention and
partial-write behavior. No SDK can reconstruct a lost response from a local label.

Run the standalone [operator example](../examples/reconcile_event_retirement.rs):

```sh
cargo run --example reconcile_event_retirement -- --help
cargo run --example reconcile_event_retirement -- \
  ingress.json BOT_UUID EVENT_ID ORIGINAL_SHA256 retained-execution \
  --approve maintenance/decision-1 --assessment accept-unknown \
  --evidence operator/retained-failure-assessment-1
```

Without approval arguments it only prepares. Exact mode accepts a polling fingerprint
instead of `retained-execution`. The example shares the Unix atomic journal/lease
adapter with [timestamp maintenance](timestamp-reconciliation.md), preserves full
original receipts and audit during fsync/rename transactions, and never starts a
runner. It operates on that consumer-owned source of truth; do not export/edit another
database into it. Database consumers implement the same hook in their own transaction.

Feature `gateway` enables the API response pump; `hot-reload` enables executable
workers. Worker protocols 4–8 remain supported with no new RPC protocol. Protocol 6+
provides `single_attempt_api`/`api_once`; UI/operation-identity versions retain their
existing requirements. Native and worker command authorization, parsing, checks,
cooldowns and reply ownership stay in their existing layers.
