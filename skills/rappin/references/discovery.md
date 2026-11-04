# Discovery, scope and permission evidence

## Resolve without guessing

Use stable IDs already provided by the user/context when available, verifying
resource/server scope as needed. Scope ambiguous names to the intended server.
`resolve_channel` / `resolve_member` return a unique result or an error; never
replace a failed resolver with the first partial candidate.

`search_members` exposes candidates with `status`, `complete`, `scanned`, `pages`,
`next_cursor` and `stop`. A result can contain one match and still be
`status: incomplete` / `complete: false`. `page_limit` or `no_progress` does not
prove the rest of the server lacks another match. `ambiguous` requires a narrower
name or stable ID. `not_found` applies only to a completed search in that scope.
Bound any additional reads and preserve the candidates/continuation evidence if
the search cannot finish. Ask for a stable ID when more scanning is not warranted.

For exact details use `members get SERVER USER`, `channels get CHANNEL`, or the
corresponding named MCP operations from the installed tool catalogue. Member
pagination cursors are user UUIDs; message cursors are timestamps. Follow the
returned direction with `before` or `after`. Locally limited server/channel lists
use `next_offset` / `--offset`, which refetches a potentially changed inventory.
Output truncation and incomplete server search are separate facts; inspect both.

## Keep operator and platform authority separate

A requested action establishes task scope, not missing platform permissions.
A successful bot read does not establish another user's effective access.
Server-wide Manage Channels is not proof of requester channel/category ACLs.
Runtime operation metadata describes required permissions and availability; it
does not grant them. CLI/MCP operations run with the bot's credentials; do not
claim they automatically enforce a separate requester's authorization as the
library's `ValidatedOperation` workflow can.

For channel/category management, verify the channel, destination category and
referenced roles belong to the intended server. Editing `category_id` with JSON
null clears it; omission keeps it. Preserve explicit false booleans. Resolve
nullable/body bounds using the installed schema rather than guessing aliases.

401 points to credentials; 403 can reflect permission/hierarchy/override rules or
test-server/rollout restrictions. 404 can mean wrong scope or lack of visibility.
Do not retry these blindly, expand privileges, or change policy to make a call
succeed. Report which check/evidence is absent when the API cannot establish it.
Use existing trusted host facts for restrictions; never infer test-server
eligibility from one unrelated successful read or a user/model argument.
