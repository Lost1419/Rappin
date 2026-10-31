# CLI and MCP workflows

The CLI keeps its resource commands (`servers`, `channels`, `members`, `messages`,
`events`, `dm`, `request`). A shared operation catalog powers both MCP and the
`tools` CLI, including validation, inline payload handling, and output schemas.

## Discover the installed interface

```sh
rappin capabilities
rappin tools list
rappin schema Message
rappin schema --method POST --path '/channels/{id}/messages'
```

Capabilities report the installed version, compiled features, CLI tree, and MCP
operation definitions. They describe the interface, not the bot's actual server
permissions. Endpoint schema lookup includes the operation and recursively
referenced component definitions; `rappin schema` still prints the full spec.
MCP `get_schema` requires a component or method/path to keep discovery focused.

The same operation works from CLI JSON or MCP:

```sh
printf '%s\n' '{"channel":"00000000-0000-0000-0000-000000000000","content":"Preview","dry_run":true}' |
  rappin tools call send_message --arguments-file -
```

MCP calls `send_message` with that object directly. It accepts exactly one of
`content` or a CreateMessage `body`, plus optional `reply_to`, `split`, and `dry_run`. `split:true` accepts long plain text
as multiple sends; rich bodies cannot be split.
`edit_message` takes `channel`, `message`, and an EditMessage `body`; null fields
retain the SDK's explicit-clear semantics. Bodies are inline JSON, not paths.
The catalog rejects unknown top-level arguments before invoking commands.
Inline arguments are limited to 128 KiB.

## Available MCP operations

| Workflow | Operations |
|---|---|
| Discovery | `list_servers`, `get_server`, `list_channels`, `resolve_channel`, `get_channel` |
| Members | `list_members`, `resolve_member`, `get_member` |
| Name searches | `search_members`, `search_channels`: candidates plus complete/unique/ambiguous/incomplete status |
| Messages | `read_messages`, `get_message`, `send_message`, `edit_message`, `delete_message` |
| Message state | `add_reaction`, `remove_reaction`, `pin_message`, `unpin_message` |
| Event delivery | `poll_events`, `ack_events` |
| Diagnostics | `doctor`, `get_capabilities`, `get_schema` |
| Server management | Role/permission writes, invites, counts and scheduled events; see [server management](server-management.md) for the full catalog and test-server restrictions |

Writes support `dry_run`. Destructive annotations identify edits, deletion,
removal and cursor acknowledgement. Annotations describe behavior and do not
replace application authorization or the bot's API permissions. CLI raw requests
remain available; MCP exposes the named catalog, not arbitrary API or filesystem
access.

## Bound collection output

```sh
rappin messages list "$CHANNEL_ID" --limit 100 --max-pages 2 \
  --max-items 20 --max-bytes 16384 --fields id,author_id,content,created_at
rappin servers list --max-items 10 --offset 10
```

MCP collection tools accept `max_items`, `max_bytes`, and comma-separated `fields`.
Server/channel, role, invite, override and scheduled-event lists also accept `offset`. These controls apply only to collection
reads; applying them to writes or event batches is rejected before execution.
They bound returned data, not necessarily the provider response size or number of
requests: set `limit`/`max_pages` to bound fetching.

`max_bytes` measures the compact `{ok,data}` JSON envelope after redaction,
before MCP's text/structured transport encoding. IDs and creation timestamps
remain present when selecting fields. Associated authors are limited to returned
messages. Responses report `returned_count` and `truncated`.

- Messages retain chronological output. Backward reads retain the newest bounded
  slice; forward reads retain the oldest. Continuation uses the boundary actually
  returned, not a hidden discarded page.
- Output trimming keeps equal-timestamp messages together. A budget too small
  for the next timestamp group returns an error rather than inventing a cursor
  that skips those records. The upstream API still has timestamp-only pagination;
  page-boundary ties in the provider itself remain a documented limitation.
- Members continue after the last returned user ID.
- Servers/channels and the server-management lists use `next_offset` and `continuation_kind: client_offset`.
  This re-fetches the provider list; changes to it can shift offsets.

## Event receipts and checkpoints

```sh
rappin events poll --cursor-file ./state/events.json --wait 25
# Process the returned events, then use the returned watermark and receipt:
rappin events ack "$NEXT_CURSOR" --batch-id "$BATCH_ID" \
  --cursor-file ./state/events.json
```

The numeric cursor file remains compatible with the Rust SDK. A companion
`.pending.json` file stores the delivery receipt, bot/base binding and unhandled
events. It is written atomically with mode 0600 on Unix. Restrict the containing
directory/ACL on other systems. These files can contain interaction credentials;
keep them private and outside source control.

Polling replays pending events until acknowledged. Only an ID present in that
batch or its final watermark can be committed. `batch_id` rejects stale receipts;
when omitted, the ID is still checked against the current pending delivery.
Partial acknowledgement rotates the receipt and retains remaining events.
Repeating an already committed acknowledgement is idempotent and does not rotate
the current receipt again.
Completed receipts retain the binding with their event payloads removed.

OS-managed locks prevent simultaneous processes from mutating one cursor;
process exit releases the lock. In-process MCP event operations are serialized.
Crash recovery reconciles a numeric checkpoint written before its receipt update,
so already committed IDs are pruned before replay. Separate bots and consumers
need separate state files. Token/base changes require intentional recovery.

The explicit CLI `events ack ID --cursor-file PATH --force` bypasses receipt
validation for operator recovery while still refusing backwards cursors. It may
skip unprocessed data and is never exposed through MCP. Old scripts that directly
set a cursor must either obtain a delivery first or deliberately opt into recovery.

Start MCP with `rappin mcp --cursor-file PATH`. Its tools cannot select a local
path. A poll without persistent state can inspect events, but acknowledging
requires the startup-selected state file.

## MCP execution and cancellation

Results include `structuredContent` conforming to each tool's `outputSchema`,
plus the equivalent serialized JSON text for compatibility. Consume one copy.
Tool failures set `isError` and return the same CLI error envelope.

Up to 16 tool calls execute concurrently. Responses are correlated by request ID
and may arrive out of order. Long polling does not block pings or other tools.
The reader preserves partial frames and rejects frames larger than 256 KiB.
Duplicate active request IDs and excess concurrent calls return protocol errors.

`notifications/cancelled` aborts read operations; it does not acknowledge events.
Cancellation of submitted writes is ignored because the service may already have
applied them. Their eventual result is returned. Disconnects/crashes can still
leave a write outcome unknown; cancellation is not a rollback mechanism.

## Error recovery

Existing `kind`, `status`, `message`, and `retry_after` fields remain. Errors also
report `outcome` and `retry_safety`:

| Value | Meaning |
|---|---|
| `safe_read` | Retrying this read cannot duplicate a write; this does not guarantee recovery |
| `server_advised` | Retry-After was supplied for a supported 429/503 response; the client already exhausted its bounded retry policy |
| `fix_request` | Correct arguments, configuration or permissions before trying again |
| `unknown_write_outcome` | A transport, decoding or server failure may follow an executed write; inspect context before a deliberate retry |
| `do_not_replay_workflow` | A multi-step write stopped; confirmed steps must not be replayed |

A failed CLI split send returns `kind: partial_write`, `completed_steps`, a
one-based `failed_step`, and the failed step's status/outcome. Completed steps
include the API responses and posted IDs. Do not resend the entire split. An
unknown failed step needs reconciliation before retrying; matching history text
alone is not an idempotency guarantee.

Doctor additionally reports `active_base_url` and its own `last_request` timing,
retry count and advised waits. It does not inspect the telemetry of another
running bot, and REST response-header timing is not gateway heartbeat RTT or
end-to-end UI latency.
