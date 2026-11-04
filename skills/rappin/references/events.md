# Durable agent event delivery

Choose a separate state file per bot/consumer. Polling with `after=0` starts from
now rather than replaying all history:

```sh
rappin events poll --cursor-file ./state/events.json --wait 25
rappin events ack "$NEXT_CURSOR" --batch-id "$BATCH_ID" --cursor-file ./state/events.json
```

A poll returns `events`, `next_cursor`, and `batch_id`. Process the batch before
acknowledging its watermark. Partial acknowledgements may use an ID actually in
the batch. Poll again to obtain the remaining events and the rotated receipt.
Without acknowledgement, the next poll replays the pending batch.

Only acknowledge a completed prefix. Failed/uncertain work needs reconciliation
before acknowledging its ID; a later successful event does not make that earlier
work complete. Store durable effects/receipts in application-owned storage.
Receiving or logging an event alone does not establish completed processing.

Receipts persist beside the numeric checkpoint and are bound to the bot token
and configured API base. They can contain interaction credentials: do not print
or publish the state files. Keep them outside source control. Cursor locks are
released by the OS if the process exits. A caller-selected state file is an
operator trust boundary; MCP tool arguments cannot choose it.

MCP exposes `poll_events` and `ack_events`. Register the fixed cursor path when
starting the server. Reads can be cancelled, and a long poll does not block other
tools. Cancellation is not acknowledgement. Write cancellation is ignored once
submitted; wait for its result rather than assuming it was rolled back.

The explicit CLI `events ack --force` is for operator-requested recovery. It
bypasses delivery checks and can skip data; it is unavailable through MCP. Never
use it merely to silence an acknowledgement error. Tokens rotated for the same
bot still change the binding; retain the checkpoint and use a new receipt sidecar
only as part of an intentional recovery, or choose a new state path.
