# Read availability and role hierarchy

## Startup and intake availability

Availability recovery is opt-in. Existing client retry settings and bot startup
behavior stay unchanged when no policy is configured.

```rust,no_run
use rappin::{AvailabilityPolicy, Bot, Result};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let policy = AvailabilityPolicy::new(6, Duration::from_secs(120))?
        .backoff(Duration::from_millis(500), Duration::from_secs(10))?
        .jitter_percent(20)?;
    let bot = Bot::builder(std::env::var("YAPPER_BOT_TOKEN").expect("token"))
        .availability_policy(policy)
        .build();
    let handle = bot.handle();
    let mut changes = handle.subscribe_availability();
    tokio::spawn(async move {
        while changes.changed().await.is_ok() {
            let status = changes.borrow().clone();
            eprintln!("{:?} {:?}, attempts {}/{}, retries {}",
                status.phase, status.state, status.attempts,
                status.max_attempts, status.total_retries);
        }
    });
    // Another task can call handle.stop() to cancel a read or backoff.
    bot.run().await
}
```

The policy applies only to startup `GET /me`, the initial `GET /events` cursor
probe when there is no saved cursor, and subsequent polling/gateway reads. It
does not retry cog loading, command synchronization, handlers, cursor writes or
mutations. Successfully read batches still use the ordered completion/checkpoint
policy described in [handler failures](handler-failures.md).

One episode covers a read and its recovery attempts. The first attempt counts;
a successful read ends the episode and the next read gets a new budget. The
elapsed limit includes network time, shared rate-limit waits and backoff. Queue
admission is backpressure and is outside this budget. Choose an elapsed budget
longer than the normal 25-second long poll plus expected recovery time. Defaults
are five attempts, 120 seconds, 500 ms initial/10 s maximum exponential backoff,
and 20% downward jitter. Limits are 1–1000 attempts and positive durations.

Retryable failures are connection/request/body transport errors, HTTP
408/500/502/503/504/520/521/522, and 429 with a server retry hint. Gateway disconnects
and transient close codes 1000/1001/1006/1011/1012/1013 can recover too. Authentication,
configuration, malformed payloads, permanent statuses and the client's exhausted
shared/advised wait budget stay terminal. Existing gateway fallback remains
available in automatic mode; gateway-only mode reconnects within this policy.

**Retry ownership:** client response/network replays are disabled inside each
availability attempt. The policy owns the attempt count, avoiding multiplied
client/pipeline retries. Client failover may select a healthy origin for the next
policy attempt but cannot add an inner replay. Ordinary API calls retain their
existing retry configuration. Attempts in one episode share a single
`max_retry_wait` allowance for server-advised/shared rate-limit waits; it does not
reset on availability retry. Only the advised portion of backoff charges that
allowance; additional outage backoff is bounded by the availability deadline.
Exceeding either limit stops recovery. Retry-After is a delay floor; it can exceed the
backoff cap, but a delay that cannot fit the elapsed budget exhausts immediately.

`Bot` and `BotHandle` expose `availability_status()` and
`subscribe_availability()`. Snapshots contain phase, state, configured limits,
last structured cause, current attempts/delay/elapsed/backoff and cumulative
attempt/retry/recovery/exhaustion/cancellation counters. Watch updates can
coalesce; cumulative counters do not lose attempts. `Recovered` means the read
succeeded; `recoveries` counts only reads needing more than one attempt.
Elapsed/backoff snapshots update at transitions, rather than ticking continuously.
These observations are separate from command and HTTP request metrics.

Exhaustion/cancellation returns a structured `availability` remote error.
`YappinError::availability_failure()` and `RemoteError::availability_failure()`
recover `AvailabilityFailure`, including phase, termination reason, limits and
last observed cause/status/retry hint. Startup cancellation returns that error;
stopping intake gracefully drains accepted handlers and checkpoints only completed
work. In-flight reads are cancelled without inventing a response status. Existing
handler/storage failures stay distinct; unfinished events are never implicitly
completed. Cancellation or exhaustion never restarts a process or replays a write.

## Fresh role hierarchy evidence

`RoleHierarchyPolicy` supplies `ExternalCheck::RoleHierarchy` for `delete_role`.
`RoleHierarchyPolicy::assess(server, actor, role, principal)` returns an individual
`RoleHierarchyAssessment`; use `ValidatedOperation::assess_with_policy` to include
both requester and bot, required permissions, resource scope and rollout checks.
An Allowed hierarchy assessment alone is not complete operation authorization.

The bundled upstream OpenAPI 1.2 describes these public facts:

| Evidence | Endpoint/field | Meaning |
| --- | --- | --- |
| Authenticated bot | `GET /me`, `id` | Bot identity; never requester authority. |
| Server owner | `GET /servers/{id}`, `server.id`, `server.owner_id` | Owner identity and scope. |
| Role inventory | `GET /servers/{id}/roles`, `roles[]` | Documented highest-first, unpaginated list. |
| Rank | `roles[].position` | Higher numeric values are more powerful. |
| Protected roles | `roles[].auto` | Explicit null/`none` is normal; automatic/everyone/system roles are not deletable under this policy. |
| Assigned roles | `GET /servers/{id}/members/{actor}`, `member.id`, `member.roles[]` | Independently fetched for each nonowner actor. |
| Server permission | Role `perms`/`permissions` maps | Existing `PermissionSnapshot` evaluates Manage Roles, including Administrator and owner server-permission handling. |
| Delete restriction | `DELETE /servers/{id}/roles/{role}` | Manage Roles, below the bot's highest role, `bot_roles` rollout/test-server restriction. |

The default `RoleHierarchyPolicy::new(&api)` uses documented evidence only.
It returns Unknown for arbitrary requester hierarchy authority, equal-position
tie ordering, administrator peers and an otherwise-needed owner/admin rank
exemption. Server permission bypass does not imply hierarchy bypass. Missing
positions, automatic-role classification, permission maps, unresolved assigned
role IDs, duplicated inventory or unsupported pagination also remain Unknown.
Explicit conflicting IDs/server scopes are denied. A successful bot read is not
an access grant to another actor.

Applications can explicitly choose `.owner_admin_strict()`:

- Owner may manage explicitly normal roles, including administrator roles.
- Every nonowner needs Manage Roles. No nonowner may modify an administrator
  role, including another administrator, regardless of numeric position.
- Other targets must be strictly below the actor's highest assigned role.
  Equal positions are denied. Administrator never bypasses this requirement.
- Protected everyone/automatic/system roles remain blocked, even for the owner.

This is an application policy, not an assertion of undocumented platform rules.
Requester and bot follow it independently: an owner's request cannot elevate a
nonowner bot above an administrator target.

There is no public endpoint proving arbitrary requester hierarchy precedence,
tie ordering, all implicit-role ranks, or test-server/`bot_roles` eligibility.
Missing assigned-role evidence prevents an allow; `required_roles_complete`
means all roles needed for that decision resolved, not that hidden/global
inventory completeness was established. Do not infer everyone from a role name.
`OperationContext::test_server_access(true)` must come from trusted configuration
or independent host evidence, never a model/user argument. Leave it unset when
unknown; `RoleHierarchyPolicy` does not manufacture rollout evidence.

## Native and worker composition

The [runnable example](../examples/role_hierarchy.rs) assesses by default:

```sh
YAPPER_BOT_TOKEN=... cargo run --example role_hierarchy -- SERVER REQUESTER ROLE
# Only after independently establishing test-server eligibility:
RAPPIN_TEST_SERVER=1 YAPPER_BOT_TOKEN=... cargo run --example role_hierarchy -- SERVER REQUESTER ROLE --execute
```

`RoleHierarchyPolicy::execute_deletion` composes the policy with
`ValidatedOperation`, forces `Freshness::Refresh`, rechecks requester and bot, and
issues the DELETE through `Api::request_once`. For a composed host policy, use
`execute_role_deletion(&api, &operation, &context, &policy)` instead. Read errors
and structured authorization reports propagate; Denied and Unknown both block
the mutation. No API credentials are needed to validate metadata offline.

The [worker example](../examples/role_hierarchy_worker.rs) assesses a role from
`role-check ROLE_UUID`, using server/requester identities from the message:

```sh
cargo build --example role_hierarchy_worker --features hot-reload
```

Load it using `WorkerProject::example("Cargo.toml", "role_hierarchy_worker")`
with `.features(["hot-reload"])` and `HotReload::load`, as in the hot-reload guide.
It deliberately leaves rollout evidence Unknown and performs no deletion.
Workers must opt in with `WorkerCog::single_attempt_api()` (protocol v6).
Existing workers retain v4/v5 behavior. `Api` works with Client, Arc, native
contexts and WorkerContext; custom adapters must explicitly implement
`request_once` or execution fails closed. The parent retains the token, endpoint
confinement, proxy request bounds and structured error sanitization. Trusted
workers can otherwise use ordinary APIs; this is not a sandbox.

No API transaction combines requester assessment and mutation. Access can change
after the final read; fresh reassessment narrows that race but cannot eliminate
it. Yapin still makes its final bot-side authorization decision. Persist operation
receipts in application-owned storage. A failed DELETE is attempted once even
with Retry-After; uncertain transport/response outcomes remain Unknown. Structured
causes/status/hints/receipts survive workers and failure callbacks. A successful
status with an unreadable body is also uncertain. Do not automatically retry a
deletion or the enclosing command; reconcile the resulting role state first.

No optional feature is required for native policies; `gateway` enables WebSocket
intake and `hot-reload` enables worker composition. See the existing
[capability bindings](capability-bindings.md) and [hot reload](hot-reloading.md)
guides for registration and failure dispositions.
