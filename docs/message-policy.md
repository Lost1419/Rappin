# Server-message gates and fallible listeners

`BotBuilder::message_gate` is an opt-in shared gate for native and worker commands
and server-message listeners. It runs on `EventData::Message`, after bot/target
filters and before prefix resolution, generated group help, cog checks or command
handlers. It also runs for ordinary natural-language messages with no command.
DMs use the separate `on_dm` contract. A missing `server_id` does not bypass the
gate; an application that requires server scope should reject that case itself.

The gate receives an owned `MessageContext`; `command`, `args` and `argv` have not
been resolved yet. Read the original event for platform-supplied command metadata.
Custom prefixes, permission policies, cache freshness and storage remain owned by
the application. Per-cog Boolean checks still run for commands that pass the gate.

## Outcomes and ordering

Gates and the new fallible listeners return `Result<MessageDisposition>`:

| Result | Dispatch and checkpoint behavior |
|---|---|
| `Continue` | Run the next stage; this alone does not complete the event |
| `Handled` | Required work is complete; skip all remaining command/listener stages and complete the event |
| `Stop` | Stop with structured `message_stopped`, leave the event incomplete and retain the cursor barrier |
| `Err(cause)` | Apply the message-failure policy below, preserving the original cause |

`Stop` is deliberately different from graceful `BotHandle::stop()`: graceful stop
drains accepted events; a message `Stop` leaves its current event incomplete. Use
`Err(cause)` to preserve a more specific failure. Explicit `Stop` outcomes do not
invoke a failure callback. No outcome retries a gate, listener or handler.

After an allowed gate, dispatch keeps the existing order: command/help processing,
the global event handler, then cog listeners. Relative order among cogs is
unspecified. `EventHandler::on_message_result` and `Cog::on_message_result` are the
fallible hooks. Their defaults call the existing `on_message -> ()` hook exactly
once and return `Continue`. Existing implementations and default behavior remain
supported. Override the new hook for required I/O; when overriding it, explicitly
call any legacy hook you still need. Errors deliberately discarded inside a legacy
`()` callback cannot be recovered by the SDK.

```rust,no_run
use rappin::prelude::*;
struct NaturalLanguage;
#[async_trait::async_trait]
impl EventHandler for NaturalLanguage {
    async fn on_message_result(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
        if ctx.command.is_none() && ctx.content() == "hello" {
            ctx.reply("Hello!").await?;
            return Ok(MessageDisposition::Handled);
        }
        Ok(MessageDisposition::Continue)
    }
}
let allowed_user = uuid::Uuid::nil(); // load application policy from your own storage
let bot = Bot::builder("yb_token")
    .message_gate(move |ctx| async move {
        if ctx.author_id() != allowed_user {
            ctx.reply("Access denied.").await?;
            return Ok(MessageDisposition::Handled);
        }
        Ok(MessageDisposition::Continue)
    })
    .event_handler(NaturalLanguage)
    .build();
```

The complete executable example is `examples/message_gate.rs`. Set
`YAPPER_BOT_TOKEN` and `BOT_ALLOWED_USER`, then run `cargo run --example message_gate`.
It uses a local application allowlist; it does not infer platform permissions.

## Failure contract

`BotBuilder::on_message_failure` receives a context and `MessageFailure` with the
transport `event_id`, `MessageStage::{Gate, Listener}`, handler identity (`gate`,
`event_handler`, or cog name), rejection classification and the sanitized original
`RemoteError`. It uses the existing `FailureDisposition` without adding variants
to the existing `HandlerKind` enum or changing command/button callbacks.

- `Default`: reply once to a typed `CommandError` input/check/clarification
  rejection; complete that invocation only if all reply segments succeed.
  Unexpected errors remain incomplete and stop intake.
- `Handled`: explicitly complete/reconcile the failed invocation. For a gate,
  skip all downstream dispatch. For a listener, run remaining listeners; another
  failure can still leave the event incomplete. This differs from a listener
  returning `MessageDisposition::Handled`, which claims the rest of the message.
- `Stop`: leave the event incomplete and return its original cause.
- `Err(error)`: preserve both the original and handling causes in `FailureHandling`.

A rejected gate never falls through to commands, even after a successful default
reply or `Handled` callback. Rejection at a post-command listener does not undo
an already completed command. Partial/uncertain effects, HTTP status, retry hints
and confirmed write receipts remain structured in failures and callbacks. A
successful error reply does not establish reconciliation of an unexpected write.
Await required I/O and store durable ownership/receipts before returning `Handled`.
Do not turn uncertain effects into a typed rejection just to keep intake running.
Panic unwinds and lost worker completion remain uncertain failures.

```rust,no_run
use rappin::prelude::*;
let bot = Bot::builder("yb_token").on_message_failure(|ctx, failure| async move {
    if failure.class == FailureClass::Rejected {
        ctx.reply(failure.error.to_string()).await?;
        Ok(FailureDisposition::Handled)
    } else {
        eprintln!("{}", failure.error.to_json());
        Ok(FailureDisposition::Stop)
    }
});
```

## Workers

With `hot-reload`, opt in using `WorkerCog::on_message`. The parent gate always
runs before invoking either native or worker commands. Worker message failures
return through the parent `on_message_failure` policy with the registered cog name.
The parent retains credentials and performs API requests.

```rust,no_run
# #[cfg(feature = "hot-reload")]
# mod worker_example {
use rappin::{hot_reload::WorkerCog, MessageDisposition, Result};
async fn worker() -> Result<()> {
    WorkerCog::new("conversation")
        .on_message(|ctx| async move {
            let message = ctx.event.message().expect("message listener");
            if message.content.as_deref() == Some("hello") {
                ctx.say("Hello from a Rust worker!").await?;
                return Ok(MessageDisposition::Handled);
            }
            Ok(MessageDisposition::Continue)
        })
        .run().await
}
# }
```

This registers `Listener::ServerMessage` and uses protocol v5 for explicit outcomes.
Upgrade the parent before opting in. New parents accept older workers, and workers
using only existing handlers retain protocol v4. Existing `listen(Listener::Message,
...)` keeps its legacy observation/logging behavior; migrate required I/O to
`on_message`. A worker cannot register both message contracts at once. Missing or
invalid outcome/completion frames are not implicit success: retain an uncertain
barrier, poison the exchange and reconcile before explicit replacement. Workers
and their build scripts remain trusted code, not a sandbox.

## Intake, duplicates and replay boundaries

Polling and WebSocket intake use the same dispatch/completion path. Only the
ordered completed prefix is persisted. An incomplete gate/listener prevents the
cursor crossing that event; failed cursor persistence remains a storage failure.
Concurrent work already started is drained, so other events can finish after a
failure. Set `handler_concurrency(1)` for strict event ordering. Cancellation never
implicitly completes unfinished work.

Intake deduplicates transport event IDs within batches and drops previously received
IDs during that run. `process_event` is a low-level custom-transport API: repeated
calls dispatch again, and its caller owns deduplication/checkpoints. Different
event IDs containing the same message/content are not automatically duplicates.
After an explicit restart, failed or completed-but-uncommitted events can arrive
again. Store durable message/workflow receipts in application storage; the SDK
does not promise exactly-once side effects or replay uncertain writes itself.

The gate controls command/listener dispatch, not raw intake, cache maintenance or
`EventHub` collectors, which receive authenticated events before dispatch. Use
appropriate collector filters and application authorization for those workflows.
Authentication, transport, preparation, cursor and storage errors do not enter the
message failure callback. See [handler failures](handler-failures.md) for ordered
checkpoint and recovery details shared with commands/buttons.
