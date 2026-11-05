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
`error` retains the original structured `YappinError` where one exists.
`effects_possible` is conservative: arbitrary gate/check/handler code may already
have replied or mutated state, even if its final result is a denial. False means
execution was rejected before those callbacks; it does not describe earlier work
in the outer handler. A Boolean cog denial has no structured cause to recover.

Nested invocation does not call framework error callbacks or send automatic error
replies. Target handlers/checks still own their own replies. Use
`outcome.into_result()?` to leave incomplete work visible to the outer handler's
failure policy and checkpoint barrier. This produces `YappinError::Invocation`,
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
`cargo run --example checked_invocation` with `YAPIN_BOT_TOKEN` configured.
