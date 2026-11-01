# Command and button failure policy

Rappin distinguishes a completed rejection from an unfinished invocation. Native
and worker commands/buttons share this policy on polling and WebSocket intake.
Only the ordered completed prefix of events can be checkpointed.

## Defaults and classification

| Failure | Default disposition |
|---|---|
| `CommandError::Quoting`, `Missing`, `Invalid`, `Extra`, `Denied` | Reply with the error/usage (ephemeral for buttons), then complete the rejection if the reply succeeds |
| `CommandError::Definition` | Unexpected; leave unprocessed and stop intake |
| API, transport, validation, storage or other handler error | Unexpected; leave unprocessed and stop intake |
| Failed rejection reply | Leave unprocessed; preserve the original rejection and reply failure |
| Worker timeout/lost completion or unwinding panic | Completion is uncertain; stop intake and retain a checkpoint barrier |

Use typed errors for expected input, permission-check or clarification rejection
when no unfinished workflow remains:

```rust,no_run
use rappin::prelude::*;
let bot = Bot::builder("yb_token")
    .command("select", "Choose a unique target", |_ctx| async move {
        Err(CommandError::Invalid {
            name: "target".into(),
            reason: "Several targets match; run the command again with an ID".into(),
        }.into())
    });
```

The rejection completes once; a new user command is a new event. A clarification
reply or Retry-After hint never schedules a handler retry. Do not convert a
transport/database error, partial write or unknown mutation into a typed denial
to keep intake alive. An API 403 might concern the bot's access rather than the
requester's, so HTTP statuses are not automatically user rejections.

## Explicit completion callbacks

`BotBuilder::on_command_failure` and `on_button_failure` receive a context and
`HandlerFailure`: global `event_id`, handler kind, route/prefix,
`FailureClass::{Rejected, Unexpected}`, and the original structured `RemoteError`.
Both return `Result<FailureDisposition>`:

- **`Default`:** apply the table above. Useful for recording the original cause
  without changing default handling.
- **`Handled`:** explicitly complete or reconcile this invocation. Await required
  replies, storage and reconciliation before returning it.
- **`Stop`:** leave this event incomplete, stop intake and return the cause.
- **`Err(error)`:** handling itself did not finish; retain both the original and
  callback/reply errors and stop intake.

```rust,no_run
use rappin::prelude::*;
let bot = Bot::builder("yb_token")
    .on_command_failure(|ctx, failure| async move {
        if failure.class == FailureClass::Rejected {
            ctx.reply(failure.error.to_string()).await?;
            Ok(FailureDisposition::Handled)
        } else {
            eprintln!("event {}: {}", failure.event_id, failure.error.to_json());
            Ok(FailureDisposition::Stop)
        }
    })
    .on_button_failure(|ctx, failure| async move {
        if failure.class == FailureClass::Rejected {
            ctx.ephemeral(failure.error.to_string()).await?;
            Ok(FailureDisposition::Handled)
        } else {
            Ok(FailureDisposition::Stop)
        }
    });
```

An application can complete an unexpected failure after reconciling effects or
taking durable ownership of unfinished work. The SDK does not infer completion
from a log entry or a successful error reply. Do not detach required work and
return `Handled` early. Keep receipts and transactions in your own storage.

Pre-execution `OperationAuthorization` and lookup/clarification errors can retain
their structured causes while the application completes a known rejection. For
a route that performs one validated operation with no earlier writes:

```rust,no_run
use rappin::prelude::*;
let bot = Bot::builder("yb_token").on_command_failure(|ctx, failure| async move {
    // Application contract: this route does nothing before its single operation.
    let preflight = failure.handler == "single_operation"
        && failure.error.kind == "operation_authorization"
        && failure.error.details.get("outcome").and_then(|v| v.as_str()) == Some("not_applied");
    if preflight {
        eprintln!("{}", failure.error.to_json()); // retain the denied/unknown report
        ctx.reply("The operation was not applied; check access or clarify the target.").await?;
        Ok(FailureDisposition::Handled)
    } else {
        Ok(FailureDisposition::Default)
    }
});
```

The same approach can complete a known lookup-only route needing clarification.
Keep that application contract explicit: one operation's `not_applied` or
`rejected` outcome does not prove a larger handler has no earlier effects.

`Handled` completes the failed invocation, not the rest of the event implicitly.
Remaining matching button handlers and message/interaction observers still run.
Another failure there can still leave the event incomplete.

### Legacy observers

`on_command_error` and `on_button_error` still observe unexpected failures.
Returning `()` does not complete them. Expected rejections use the default path;
use a new failure callback returning `Default` to observe those too. Legacy
callbacks cannot return failed I/O, so migrate required replies to the new
callbacks. The SDK cannot recover errors application code deliberately discards.

For each handler kind, the latest registration selects either its legacy
observer or its explicit failure callback. Registering one replaces the other;
two competing error-reply callbacks are not run.

## Structured causes and workers

Error JSON retains HTTP status, retry hints, authorization reports, partial-write
receipts and unknown outcomes. Known bot credentials are scrubbed without
removing structured authorization objects; applications must redact their own
additional secrets before logging receipts.

- `YappinError::FailureHandling` has separate `source` and `handling_error` JSON.
  Multi-part automatic replies keep confirmed messages and the failed step in
  the handling error's `PartialWrite`.
- `HandlerInterrupted` reports worker timeout/completion loss or an unwinding
  panic with `outcome: "unknown"`, `retry_safety: "do_not_replay_workflow"`, and
  any available underlying cause. An incomplete worker connection is not reused;
  reconcile before explicitly replacing it.
- A completed worker error frame leaves its connection usable. A handled failure
  can be followed by another command on that same worker process.
- Structured command errors include `command_kind`. Definition errors and
  legacy remote command errors without a subtype are not assumed to be user
  rejections. Rebuild workers for current classification. This metadata uses
  existing protocol v3 structured fields.

Typed worker parse errors now reach the parent policy. No worker-side callback
or credential is required. A source error's outcome describes that operation,
not every earlier effect in the handler. The Client's bounded server-advised
per-request retries remain separate: Rappin never retries a handler or an entire
workflow because one request has Retry-After.

## Ordered checkpoints and mixed batches

Configure `.cursor_file(...)` for restart continuity. Without durable storage,
progress is process-local and a new Bot uses its normal latest-cursor bootstrap.
A cursor file has one owner. `handler_concurrency(1)` preserves handler order
across batches; higher concurrency permits overlap, but checkpoints stay ordered.

| Events in delivery order | Checkpoint |
|---|---|
| 10 succeeds, 12 is handled rejection, batch watermark 15 | 15 after both complete |
| 10 succeeds, 12 is handled rejection, 14 fails unprocessed | 12; 14 remains replayable |
| 10 succeeds slowly, 12 fails, 14 already completed | 10 after draining; 14 remains completed but uncommitted |
| 10 fails, 12 waits to start | Prior cursor; do not start 12 |

A full batch can commit its watermark, including gaps in global IDs. A partial
batch commits only before the first failed or not-started event. Completed IDs
are discarded from memory after checkpointing, rather than accumulated forever.

A handled rejection before a barrier can commit without replaying it on explicit
restart. Completed events after the barrier, crashes or failed cursor writes can
still cause repeat delivery. This is at-least-once delivery, not a durable
exactly-once ledger. Use application receipts/idempotency where effects matter.

`Bot::process_event` applies the same invocation policy but does not store a
cursor. Custom transports must maintain their own ordered acknowledgement rules.

## Stop, drain and recovery

An unprocessed failure stops intake when observed. Events waiting for execution
capacity are not started; in-flight handlers are awaited and their results are
retained. Prefetched but unaccepted batches are not dispatched during draining.

`BotHandle::stop()` and Ctrl+C are graceful: stop intake, close waiters, and drain
accepted work. Await `Bot::run()` to finish draining. Aborting/dropping that future
is hard cancellation: pending handlers and intake/automatic-ack tasks are dropped
without declaring unfinished events complete. Submitted writes can remain
uncertain. Unwinding panics are interrupted processing; process aborts cannot be
recovered in-process.

Startup login, definition and cursor-load failures return directly before an
event-processing report exists; they are outside per-command callbacks.

Both transports share the dispatch/checkpoint path. Authentication and permanent
intake failures bypass command completion callbacks. Polling read retries are
restricted to transport errors, rate limits and HTTP 408/429/500/502/503/504 with
backoff. Other status, decode and preparation errors stop intake. Gateway-only
errors remain terminal; gateway-preferred reconnect/fallback keeps its existing
read-only transport policy. None of these paths replays a handler or restarts
the process.

Failed cursor writes stop the run without advancing its in-memory committed
cursor. `YappinError::EventProcessing` carries an `EventProcessingFailure` report:

- `committed_cursor` and `received_cursor`;
- `failed_events`, each with its event ID and structured cause;
- `unprocessed_events`, whose handlers were not started;
- `completed_uncommitted_events`, whose work finished beyond durable progress;
- independent `intake_error` and `cursor_error` causes.

Top-level status/retry helpers use the cursor cause, then intake cause, then the
earliest failed event. Nested causes retain their own independent statuses and
retry hints. Check the report rather than classifying a compound failure from
only its top-level status.

Its outer outcome is `incomplete` and retry safety is `do_not_replay_workflow`.
Inspect nested causes and receipts before recovery. A command callback cannot
convert an authentication or cursor-storage failure into completion. Repair the
relevant intake/storage problem and reconcile uncertain effects before an
explicit restart; there is no catch-all restart loop.
