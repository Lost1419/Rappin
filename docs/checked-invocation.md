# Checked invocation of installed commands

`BotHandle::invoke_command(&MessageContext, name, arguments)` runs an installed
command through the same dispatcher as an explicit command. It resolves aliases
and groups, runs the owning cog check, then the registered parser, checks,
cooldowns and handler. The original requester, server, channel, message and reply
target remain unchanged. It does not manufacture an event, run listeners again,
or advance a checkpoint. Normal explicit commands keep their existing behavior.

Invocation requires the active authenticated message task, after its message gate
has allowed processing. Native command handlers, fallible message listeners and
message failure callbacks can use their existing context. Changing its event,
client or bot rejects invocation. Spawned/detached tasks and saved contexts cannot
invoke commands. Consumers using `Bot::process_event` still own authentication and
deduplication of their custom transport; that API is a trusted host entry point.
This is an application API, not a sandbox against arbitrary Rust host code.

## Preparation and execution

- `prepare_invocation(&ctx, name, arguments) -> Result<PreparedInvocation>` resolves
  the current binding and returns an opaque, single-use plan. Its `command()`,
  `arguments()` and `report()` methods expose the resolved request and advisory
  assessment. Registration preflight remains a different operation.
- `PreparationReport::{assessed, deferred}` lists what was checked. Dynamic
  `CommandSchema` validation and built-in `Check` assessments are supported.
  Built-in permission/role checks retain their configured freshness policy
  (refresh by default). Applications needing fresh owner data must account for
  the existing owner-profile cache policy.
- Custom `CommandCheck::prepare` defaults to deferred. An implementation can return
  a read-only assessment future, but must not reply, mutate state, or reserve a
  cooldown. Ordinary closure checks, cog checks, opaque handlers, worker parsers,
  arbitrary `FromArgs` implementations and handler-owned validation are deferred.
  Quote syntax can be checked without executing a custom parser.
- `invoke_prepared(plan) -> InvocationOutcome` rejects unload/replacement or alias
  rebinding. Prepare again after a change. It rechecks the message gate and all
  normal execution checks; the assessment grants no authority. A replacement
  published while an asynchronous gate runs is rejected before dispatch.
- `invoke_command(&ctx, name, arguments) -> InvocationOutcome` performs resolution
  and fresh execution directly. No preparation is required.

For several steps, prepare all targets before executing the first. Deferred checks
can still reject later steps. This is neither a transaction nor a guarantee that
all effects will succeed. Consumers own application allowlists, approvals, state,
planning and durable receipts. A selected binding is an in-flight snapshot when
execution begins; an already executing handler can finish while its cog reloads,
just like ordinary dispatch. Registry locks are released before application code.

## Completion, replies and failures

`InvocationOutcome::status` is `NotFound`, `Denied`, `Completed` or `Failed`.
`error` retains the original structured `Error` where one exists.
`effects_possible` is conservative: arbitrary gate/check/handler code may already
have replied or mutated state, even if its final result is a denial. False means
execution was rejected before those callbacks; it does not describe earlier work
in the outer handler. A Boolean cog denial has no structured cause to recover.

Nested invocation does not call framework error callbacks or send automatic error
replies. Target handlers/checks still own their own replies. Use
`outcome.into_result()?` to leave incomplete work visible to the outer handler's
failure policy and checkpoint barrier. This produces `Error::Invocation`,
which preserves its source, HTTP status, retry hint and partial receipts in error
JSON and across remote serialization. Its replay safety is always
`do_not_replay_workflow`. It is an unexpected failure to the default outer policy,
including when the inner cause was a typed rejection: the framework cannot know
whether nested application code already replied or completed earlier steps.

To complete a rejection, explicitly reconcile the result and any earlier effects,
reply once if needed, and return `Ok(())`; a failed reply must remain an error.
Alternatively use the existing outer failure callback and return `Handled` only
after reconciliation succeeds. Do not return an inner typed source directly if
that could trigger a second automatic rejection reply. No invocation is retried.
Cancellation or uncertain effects require application reconciliation; do not
replay a multi-step request automatically.

`MAX_INVOCATION_DEPTH` is eight active command/check chains, including the outer
explicit command. Nested gate invocation is rejected. Same-task asynchronous
composition preserves this bound; detached execution is rejected.

## Features and worker support

The API requires no optional feature for native commands. Targets installed with
`hot-reload` use the existing parent-owned worker proxy, authorization context and
structured error protocol; gateway event intake remains optional. There is no
protocol bump and existing workers remain compatible. Worker metadata does not
claim to expose an executable read-only parser/check chain: those checks are
reported as deferred and run inside the worker at execution. Existing worker
restrictions on group/alias registration still apply. This API is on `BotHandle`
in the parent, not a worker-to-parent nested invocation RPC.

See [the runnable example](../examples/checked_invocation.rs):
`cargo run --example checked_invocation` with `YAPPER_BOT_TOKEN` configured.

## Consumer operation identity

`OperationId` is consumer-supplied metadata for a logical operation or durable plan
step. It is separate from the original requester, server, channel and message ID.
It grants no permission, does not alter reply targets, and does not deduplicate,
authorize or replay anything. Two calls with the same ID still execute the normal
checks and handler twice. Consumers own stable key generation, storage, approval,
argument binding, receipts and reconciliation.

Public APIs (no optional feature for native commands):

```text
OperationId::new(value: impl Into<String>) -> Result<OperationId>
OperationId::MAX_BYTES // 128
OperationId::as_str(&self) -> &str
BotHandle::prepare_invocation_with_operation_id(
    &self, ctx: &MessageContext, name: &str, arguments: &str, operation_id: OperationId,
) -> Result<PreparedInvocation>
BotHandle::invoke_command_with_operation_id(
    &self, ctx: &MessageContext, name: &str, arguments: &str, operation_id: OperationId,
) -> InvocationOutcome
MessageContext::operation_id(&self) -> Option<OperationId>
PreparedInvocation::operation_id(&self) -> Option<&OperationId>
InvocationOutcome::operation_id(&self) -> Option<&OperationId>
```

Values are 1–128 UTF-8 bytes with no control characters. Serde deserialization
applies the same validation. Values are case-sensitive and not normalized; avoid
putting secrets in keys. Preparation attaches the ID before read-only check
assessment and retains it in the single-use plan through `invoke_prepared`.
Prepare every step and inspect all assessed/deferred checks before starting the
first handler. A plan is still advisory, not a transaction or permission grant.
Execution rechecks the message gate, owning cog, registered parsing/checks,
cooldowns and handler-owned authorization against the unchanged authenticated
context. Existing replacement/unload protection and recursion bounds still apply.

Ordinary explicit commands and listeners have no operation ID. Existing
`prepare_invocation` and `invoke_command` inherit the active operation ID when
nested; the explicit-ID methods override it for that call and its descendants.
Returning from a nested call restores the caller's ID. A wrapper can therefore
retain one logical operation; distinct child mutations need distinct consumer
step keys. MessageContext reads are task-scoped: cloned contexts used outside the
authenticated task return None, and detached execution remains rejected.
Preparing a different step does not change the caller's current ID.

Outcomes retain their ID even for missing commands, denial, stale bindings and
incomplete effects. `into_result()` wraps identified incomplete invocations in
`Error::IdentifiedInvocation { operation_id, source }` around the existing
`Invocation` cause. Native source chains remain available; structured remote and
failure-callback JSON keep `kind: command_invocation`, `operation_id`, status,
retry hints, original partial receipts and unknown outcomes. This wrapper never
triggers another typed-rejection reply or changes checkpoint/replay policy.
Cancellation still has no returned outcome: the consumer must already have
persisted a Pending claim before starting an effect.

A safe consumer journal binds its scoped operation key to the installed command
and normalized arguments (and any relevant definition/approval version). Reusing
a completed key with matching arguments can return its saved result; different
arguments must be rejected. Pending/Unknown requires reconciliation, including
when a write may have succeeded before the error. These decisions belong in the
consumer and must not bypass fresh authorization. Merely supplying a stable ID
makes no API request idempotent and does not make a payment safe to resend.

### Workers and compatibility

Workers require `hot-reload` and explicit `WorkerCog::operation_identity()`, which
advertises **protocol v8**. Upgrade the parent first. Existing worker modes v4–v7
and ordinary explicit calls remain unchanged. `single_attempt_api()` and `ui_api()`
keep their independent opt-ins; operation identity does not enable either helper.

```rust,no_run
use rappin::{hot_reload::WorkerCog, Result};
async fn worker() -> Result<()> {
    WorkerCog::new("records").operation_identity()
        .command("record", "Record a recipient", |ctx| async move {
            let operation = ctx.operation_id(); // Option<&OperationId>
            let message = ctx.event.message().expect("command message");
            // Apply consumer authorization and durable journal policy here.
            // operation is metadata; message still carries the original identity.
            let _ = (operation, message, ctx.args());
            Ok(())
        }).run().await
}
```

The ID travels in the private parent invocation envelope; the public
`CogEvent::Command` layout is unchanged. `WorkerContext::operation_id()` reads an
invocation snapshot. A v8 worker accepts it only on command frames. Preparation
and direct invocation reject identified calls to older workers before executing
a worker handler; IDs are never silently dropped. Prepared plans cannot bypass
this check by reloading to an older worker. Parent-side checked invocation can
target workers; there is still no worker-to-parent nested invocation RPC.

Run the standalone native consumer example with:

```sh
cargo run --example operation_identity
```

Set `YAPPER_BOT_TOKEN`, then send `!record-plan`. The example uses an explicitly
volatile in-memory journal for a local record mutation. Replace that journal with
consumer-owned durable storage for restart safety and external effects; model
planning and application approval policies remain outside the SDK.
