# Legacy evidence policy example

For API 1.4, use `ApiExt::get_member_permissions` and
`ComputedPermissionsPolicy` for fresh actor-specific channel access. The
[API 1.4 guide](resources.md) documents response scope, missing evidence, native/worker
composition and rollout. Category-specific authority and hierarchy questions may
still need application policy.

The runnable [access policy example](../examples/access_policy.rs) retains the
pre-1.4 evidence-composition demonstration for applications on older platform
contracts. Its manual role/override reads deliberately produce Unknown for
channel edits and category actions. Its evidence inventory and limitations below
record the contract checked on 1 November 2026. Select the computed helper for
current API 1.4 channel authorization.

The source has three parts:

- [`policy.rs`](../examples/access_policy/policy.rs): `PublicApiPolicy`, generic
  execution, and native/worker command integration.
- [`fixture.rs`](../examples/access_policy/fixture.rs): synthetic in-process data
  for the offline demonstration; never a production ACL oracle.
- [`tests.rs`](../examples/access_policy/tests.rs): grant/deny/unknown, scope,
  inheritance uncertainty, refresh, revocation and error coverage.

## Run it

From a checkout, with no credentials or extra Cargo features:

```sh
cargo run --example access_policy --locked --offline
cargo test --example access_policy --locked --offline
```

The default demo observes server grants, assesses a channel rename, and prints a
structured `operation_authorization` error with `decision: "unknown"` and zero
writes. The fixture is clearly labelled in its output.

For GET-only inspection of the first visible server/channel, set
`YAPPER_BOT_TOKEN` through your usual private credential mechanism, then run:

```sh
cargo run --example access_policy --locked --offline -- --inspect
```

This prints field names and HTTP outcomes, not names, IDs or credentials. It also
tries the guide's ambiguously described category-role GET as a diagnostic; that
response never authorizes an action. The `--offline` Cargo flag prevents registry
access; `--inspect` still contacts Yapin.

To assess a specific operation without executing it:

```sh
cargo run --example access_policy --locked --offline -- \
  --assess "$SERVER_ID" "$REQUESTER_ID" edit_channel arguments.json
```

`arguments.json` can contain
`{"id":"<channel UUID>","body":{"name":"Renamed"}}` with a real UUID substituted.
These two live modes perform reads only. The command-line identities are trusted
operator inputs for diagnostics; production handlers derive them from their
authenticated event, never model arguments.

## Available evidence

Checked against the [developer guide](https://www.yapin.gg/developers),
[upstream OpenAPI](https://www.yapin.gg/openapi.json), and
[official JavaScript SDK](https://www.yapin.gg/sdk/yapper.js) on 1 November 2026.
Paths below are relative to Yapin's bot API base.

| Read | Fields used or available | What it establishes |
|---|---|---|
| `GET /me` | `id` | Identity of the authenticated bot, independently of the requester |
| `GET /servers/{server}` | `server.id`, `server.owner_id` | Server identity and recorded ownership |
| `GET /servers/{server}/members/{actor}` | `member.id`, `roles`, `timeout` | Returned actor identity, assigned role IDs and timeout data |
| `GET /servers/{server}/roles` | `roles[].id`, `permissions`, legacy `perms`, `manage_server`, `manage_messages`, `auto` | Inputs to the SDK's existing **server** grant evaluator |
| `GET /channels/{channel}` | `channel.id`, `server_id`, `category_id`, `private`, `read_only`, `slowmode` | Channel/server scope; successful reading establishes this credential's visibility only |
| `GET /servers/{server}/channels` | `categories[].id`, `name`, `position`; visible channel rows | Bot-visible discovery within a server, not a requester's category ACL |
| `GET /channels/{channel}/permissions` | `overrides[].role_id`, `allow`, `deny`; null `role_id` means everyone | Configured channel rules, subject to endpoint availability; no actor-specific effective result |

The public channel override read is documented as test-server-only. The guide
groups GET/PUT/DELETE with category-role permission paths but gives no precise
category-read response contract. Neither Rappin nor the official JavaScript SDK
has a typed category-override reader. `Api::request` can make raw requests, but
that does not establish their semantics or completeness.

A bounded live read during validation returned channel overrides successfully
(an empty list), while `GET
/servers/{server}/categories/{category}/permissions/everyone` returned 404 for a
category present in the visible inventory. This is an observation of one
credential/server, **not proof that every category read is unsupported**. The
channel returned no effective-permission or private-role allowlist fields;
categories contained only ID, name and position.

## Precedence: known server behavior, incomplete effective-access behavior

`Api::permission_snapshot(server, actor, Freshness::Refresh)` requests fresh
membership/role evidence. `PermissionSnapshot::evaluate()` is the existing shared
**server** evaluator. Its current Rust contract is:

1. Matching ownership establishes the server-owner case.
2. For non-owners, require matching membership, unique role IDs, and all assigned
   roles present in the snapshot.
3. Consider assigned roles plus roles marked `auto: "everyone"`.
4. Within each role, a present `permissions[key]` wins over legacy `perms[key]`,
   then the old `manage_server`/`manage_messages` booleans. Explicit new-format
   false does not fall back to a legacy true.
5. Combine grants across applicable roles. A false entry on one role is not a
   channel deny and does not cancel a grant on another role. The `administrator`
   grant and ownership affect this server evaluator's `allows()` result.

Those are SDK server rules; they are **not a published algorithm for effective
channel/category access**. The policy records these grants but never promotes
them into requester visibility, management authority, category access, or an
owner/Administrator channel bypass.

The official JavaScript SDK gives a high-level inheritance note: category rules
apply to channels that do not have their own rules. It does not resolve all of
the following facts needed for a correct evaluator:

- Whether inheritance is selected per channel, role, or permission, and how empty
  overrides or synchronization state affect that choice.
- Precedence between everyone and multiple assigned roles; allow/deny conflicts
  within/across roles; and interaction with server owner/Administrator.
- The complete category rule set and a guaranteed-complete category read contract.
- Effective private-channel role/user allowlists. `role_ids` and `post_role_ids`
  are writable channel fields, but the documented channel read omits them.
- Exact category access prerequisites for destination assignment, rename and
  deletion. SDK `CategoryPermissions` is a policy question, not a documented
  platform permission bit; its `permissions` list is currently empty.
- An actor-scoped, authoritative effective result incorporating privacy,
  inheritance, overrides and applicable restrictions such as timeout state.

An empty override list, a successful bot read, or a server Manage Channels grant
cannot fill those gaps. A read's 403/404 concerns the authenticated credential and
resource visibility; it is not proof that the requester is denied.

## Policy decisions and errors

`PublicApiPolicy` checks each `AccessQuestion` independently for its principal
and actor ID. It verifies resource/server identity again and fetches a fresh
permission snapshot. It handles `ChannelPermissions` for edits/deletes and
`CategoryPermissions` for destination-category assignment and category edits/
deletes. Null category removal has no destination category question; the channel
access question still applies.

The sole positive resource fact the example can establish is the authenticated
bot's own `view_channel`, when that is the entire question and its fresh channel
GET succeeds. The SDK already handles that narrow bot-read fact for ordinary
bindings. Edits/deletes additionally require management access, so the example
does not approve them. Grant tests cover the bot-read fact; mutation access for
these checks remains unknown.

Identity or cross-server mismatches fail closed. Server prerequisite denials are
kept in the SDK's separate requester/bot checks. Missing effective-access
evidence returns `CheckResolution::unknown`. API failures propagate as typed
errors from the policy, so assessment records unknown with structured causes.
Other checks, such as `OverrideAuthority` on privacy edits, also stay unknown.

On execution, `ValidatedOperation::execute_with_policy` raises
`Error::OperationAuthorization` if any required check is denied or unknown.
`error.to_json()` retains:

- `decision` and `authorization.checks`, with separate principals;
- each failed evidence read's structured error and HTTP status;
- outer status 403 for denied, or null for unknown;
- `outcome: "not_applied"` when authorization stopped the mutation.

Keep this structure through your error boundary. A nested evidence-read 403 does
not mean the outer unknown decision is a requester-denied result.

## Native and worker integration

The [policy module](../examples/access_policy/policy.rs) includes compiled helpers:

- `execute(api, context, call)` works with any public `Api` implementation.
- `native(ctx, call)` derives the server/requester from `MessageContext`.
- `native_command(binding)` binds a serializable permanent command without
  modifying its metadata. Use the binding's existing `preflight` and the ordinary
  cog install/reload path; native aliases remain part of the binding.
- With `hot-reload`, `worker(ctx, call)` derives identities from the command's
  message event, and `worker_cog(binding)` constructs a dynamic worker command.
  Worker schemas retain their existing single-name/no-local-alias constraint;
  parent routing can retain friendly aliases.

Workers make reads through `WorkerContext` and its parent-owned API proxy; no
credential is passed to a worker. Fresh snapshots use the parent's existing
`Freshness::Refresh` path. Custom adapters/caches must honor that contract.
Run both feature configurations with:

```sh
cargo test --no-default-features --example access_policy --locked --offline
cargo test --features hot-reload --example access_policy --locked --offline
```

An effective-access helper needs the platform facts listed above. Existing `Api`,
`ApiExt::api_json`, `PermissionSnapshot`, `Freshness`, `OperationPolicy` and
validated bindings already provide the necessary composition points. The
example introduces no new SDK runtime API or dependency. Its `execute` helper
forces `Freshness::Refresh` for the SDK's server checks as well as the policy's
own evidence reads.

## Revocation and consistency limits

Assessment is diagnostic. Execution assesses again; the tests show a granted
server prerequisite revoked between those steps stops execution. A test-only
synthetic attestation first makes a full assessment allowed, solely to exercise
the SDK's reassessment contract. It is not production evidence or an API-derived
effective ACL grant.

Fresh reads are not an atomic authorization transaction. Changes after the last
read can still race the mutation. Yapin authorizes the final request's **bot
credential**; that is not an atomic check of the original requester. The
documented API provides no delegated requester check or ACL-revision precondition
for these writes. The conservative policy cannot close that platform gap and
does not approve the ambiguous mutation in the first place.

Keep application aliases, saved definitions and durable receipts outside this
policy. It neither persists decisions nor replays uncertain writes. A useful
platform addition would be an actor-scoped effective-access result with specified
inheritance/conflict rules, plus a mutation precondition covering requester ACL
changes. Until then, this example deliberately reports the missing evidence.
