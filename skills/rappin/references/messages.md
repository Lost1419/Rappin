# Message operations

Read exact objects with `rappin messages get CHANNEL MESSAGE`. Use
`rappin members resolve SERVER NAME --max-pages 2` for bounded member discovery.
The MCP equivalents are `get_message`, `list_members`, `resolve_member`, and
`get_member`. Consult `rappin tools list` for their argument names and schemas.

Plain text:

```sh
rappin --dry-run messages send "$CHANNEL_ID" --content "Requested reply" --reply-to "$MESSAGE_ID"
```

For embeds, buttons or Components V2, pass CreateMessage JSON via `--body-file -`.
MCP `send_message` accepts exactly one of `content` or `body`, plus `channel`,
optional `reply_to`, `split`, and `dry_run`. `edit_message` accepts `channel`, `message`,
and an EditMessage `body`. Omit fields to keep them; explicit null removes
embeds/components. Deletion and reaction changes are separate named operations.
These verbs use the bot's API permissions; read access alone does not authorize
sending messages or moderation.

`--split` performs multiple sends. If one fails, inspect `completed_steps` and
`failed_step` (one-based). Do not replay the whole workflow. A failed step with
`outcome: unknown` might have executed; inspect recent history before deciding
whether to send again. History can help reconcile, but matching text is not proof
of identity and the API provides no documented idempotency key.

Errors distinguish `server_advised` waits, request corrections, and unknown write
outcomes. Respect `retry_after`; a permission error needs changed permissions.
The CLI already applies its bounded retry policy, so avoid an additional blind
retry loop around writes.

Collection controls are `--fields`, `--max-items`, and `--max-bytes`. IDs and
creation timestamps remain available for continuation. A byte budget too small
for one item plus metadata returns an error. Limits never apply to event batches
or write results, where silently dropping records could hide work.
