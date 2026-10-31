# Shared SDK workflows

## Server permissions for any actor

`ApiExt::evaluate_permissions(server, actor, freshness)` works for arbitrary
members, including the bot. `evaluate_bot_permissions` obtains the bot's ID
first. The command `Check::permissions` uses this same evaluator.

```rust,no_run
use rappin::{Api, ApiExt, Freshness, Permission, Result};
use uuid::Uuid;

async fn can_moderate(api: &(impl Api + ?Sized), server: Uuid, actor: Uuid) -> Result<bool> {
    let permissions = api.evaluate_permissions(server, actor, Freshness::Refresh).await?;
    Ok(permissions.allows(&Permission::manage_messages()))
}
```

Server owners pass all permission checks. An assigned role granting
`administrator` also passes them. Other grants are combined across assigned
roles and the role explicitly marked `auto: "everyone"`. Other automatic roles
are not implicitly assigned. Within each role, the newer `permissions` dictionary
takes precedence over `perms`, then the legacy management booleans; an explicit
false never falls back to an older true. A false on one role does not remove a
grant from another role.

Non-owner evaluation requires matching membership and a complete, nonduplicated
role snapshot. Missing assigned roles cause an error rather than an inferred
allow. These are **server-level grants**, not a prediction that a channel action
will succeed: channel/category overrides, timeouts, visibility, role hierarchy
and other server rules remain enforced by Yapin.

For application-owned storage, construct `PermissionSnapshot` and call
`evaluate()` without I/O, implement `Api::permission_snapshot`, or supply a
`LookupCache` backend to Client/BotBuilder. Freshness defaults remain explicit;
worker permission requests use the parent's cache hooks and credentials.
Prefix resolution and database ownership are unchanged.

## One API interface for native and worker code

`Api` is implemented by `Client`, `MessageContext`, `InteractionContext`,
`WorkerContext` and `Arc<T: Api>`. It is object-safe; generic helpers can instead
use `ApiExt::api_json` for typed decoding:

```rust,no_run
use rappin::{Api, ApiExt, Result};
use serde_json::Value;

async fn profile(api: &(impl Api + ?Sized)) -> Result<Value> {
    api.api_json::<Value, ()>("GET".parse().unwrap(), "/me", None).await
}
```

Requests retain the configured origin/path restrictions. Workers proxy through
the parent and receive no credential getter. Custom implementations remain an
application trust boundary.

Worker protocol v3 carries `RemoteError`: kind, message, HTTP status, retry wait,
outcome hints and partial-write details. `YappinError` status/retry helpers work
on it. Error JSON is redacted before crossing the parent/worker boundary. Legacy
string fields remain for v1/v2 workers; rebuild workers to obtain structured
errors. The new host accepts all three versions; older hosts reject v3 workers.
Command/button dispatch preserves error status when returning a handler failure,
including when an error callback consumes the original error. Command validation
errors still produce usage replies.

## Runtime command schemas and preflight

`CommandSchema` is serializable application data. It supports string lengths,
integer/finite-number bounds, booleans, user/channel IDs, duration bounds,
choices, optional arguments, a final rest string, native aliases and subcommands.
It uses the existing quote-aware parser and command registry; no generated Rust
type or macro is required.

```rust,no_run
use rappin::{BotHandle, CogBuilder, CommandSchema, DynamicCommandSpec, Result};

async fn install(handle: &BotHandle, config_json: &str) -> Result<()> {
    let schema: CommandSchema = serde_json::from_str(config_json)?;
    schema.validate()?;
    handle.preflight_commands(&[schema.info()], Some("features")).await?;
    // Perform application preparation only after the definition preflight.
    let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
        ctx.reply(format!("Parsed: {values:?}")).await?;
        Ok(())
    })?;
    handle.add_cog(CogBuilder::new("features").registered_command(command).build()).await
}
```

Example schema:

```json
{
  "name": "feature run",
  "description": "Run a configured feature",
  "aliases": ["frun"],
  "arguments": [
    {"name":"count", "kind":{"type":"integer", "min":1, "max":10}},
    {"name":"label", "kind":{"type":"string", "max_length":100}, "required":false, "rest":true}
  ]
}
```

`DynamicCommandSpec::check` accepts the existing permission/custom checks.
`BotBuilder::registered_command` and `CogBuilder::registered_command` install the
bound result. `WorkerCog::dynamic_command` shares the runtime parser and help;
workers retain their existing single-name routing constraint and reject local
aliases/subcommands rather than silently dropping them.

`preflight_commands` validates schemas' help metadata, aliases, ownership and the
100 advertised-root limit without running handlers, lifecycle hooks or HTTP.
Validate the full CommandSchema first to include its argument type/bounds checks.
`preflight_cog` checks currently exposed command definitions before `on_load`.
Cogs that initialize metadata in `on_load` receive a second check afterward.
Preflight returns an advisory `RegistrationPlan`, **not a reservation or a
database transaction**. Another task can change the registry afterward;
installation revalidates under its update lock and preserves old routes on
failure. Applications own their transaction/recovery policy for external writes.

## Request totals and observations

`Client::request_metrics()` returns a coherent snapshot shared across clones.
It counts started/completed/cancelled/in-flight logical HTTP executions,
successful/failed executions, attempts/retries, transport failures, 429s and 5xx
responses. `started == completed + cancelled + in_flight`; completed equals
succeeded plus failed. Cancelled executions and their elapsed rate-limit waits
are accounted for when their futures are dropped.

These metrics cover HTTP execution, including advised retries/waits. Successful
HTTP execution ends at response headers, before successful response-body decoding;
API errors include reading their error bodies. They do not measure UI delivery,
command-handler duration or gateway heartbeat RTT. Completed duration totals
exclude currently in-flight elapsed time. Invalid paths can fail before a network
attempt; use `attempts` for attempted network traffic.

`subscribe_request_events()` is a 256-entry broadcast stream. Each event includes
a monotonic sequence, logical request ID, individual attempt duration and the
existing cumulative RequestObservation. Slow readers receive `RecvError::Lagged`;
sequence gaps also reveal loss. Counters stay exact regardless of notification
loss. Routes remain sanitized templates, never raw credentials or query strings.
The existing `subscribe_requests()` watch API remains **latest-value only**;
do not count notifications from it to derive traffic totals.

## Bounded lookups

`search_members(server, name, max_pages)` and `search_channels(server, name)`
return `LookupResult<T>` with candidates, completion, pages, scanned records,
stop reason and continuation cursor. `status()` distinguishes unique, not found,
ambiguous and incomplete. Two matches prove ambiguity even during an incomplete
search. One or zero candidates cannot prove uniqueness/absence until the scope
is exhausted. A full final member page therefore remains incomplete.

`resolve_member` now fails with `IncompleteSearch` when its page budget prevents
a uniqueness decision. Increase the budget or use a stable UUID. A repeated or
nonadvancing member page reports `NoProgress`; it never establishes exhaustion.
Exact-ID reads search only that resource and do not enumerate all members. These
results concern the bot-visible scope, not resources hidden by API permissions.

CLI `members search` / `channels search` and MCP `search_members` /
`search_channels` expose the candidates and status explicitly. Existing resolver
tools retain their successful response shapes, and return structured
`incomplete_search` errors instead of guessing. Output trimming marks truncation;
the status describes the search before display limits. `next_cursor` is the last
member page boundary and can be passed to member listing; callers combining
subsequent searches must retain prior candidates when deciding uniqueness.
