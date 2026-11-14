# Troubleshooting paths

Use the application's selected SDK version. Availability/hierarchy/single-attempt
helpers below require Rappin 0.1.20+; older deployments need their matching contract.

## Slow replies

Separate REST header timing, full-call/body decode time, gateway heartbeat RTT,
event delivery age and handler execution. Doctor's last request measures only its
own request. Inspect the running application's `transport_status()` and Client
telemetry for its actual path. Include waits and attempts before attributing time
to the network/service; use a few paced observations rather than a load probe.

`Client::request_metrics()` retains exact cumulative totals across clones.
`subscribe_request_events()` provides a bounded attempt stream; record sequence
gaps / broadcast `Lagged` errors. Legacy `subscribe_requests()` is latest-only;
counting its notifications can underreport a burst. Do not derive latency
percentiles or traffic totals from a stream with unreported losses.

## Missing events or reconnects

Check configured/active transport and fallback reason. Gateway is feature-gated;
access denial may cause expected polling fallback. Distinguish received from
committed cursor, pending batches and completed-but-uncommitted handlers. A
consumer's pending events replay until acknowledgement; acknowledge only processed,
delivered IDs. A failed handler can retain a checkpoint barrier while a completed
prefix advances. Check backlog/saturation before blaming transport. Keep one owner
per cursor file and a separate file per bot/consumer.

## Startup and intake availability

Identify the failing read: startup identity (`GET /me`), initial cursor probe
(`GET /events` without a saved cursor), or intake. Existing behavior remains unless
`BotBuilder::availability_policy` is configured. Bounded opt-in recovery includes
520/521/522, cancellable backoff and reads; it never restarts handlers/processes.

Inspect `availability_status()` / `subscribe_availability()` for phase, state,
configured limits, attempts, last cause and cumulative retries/exhaustions.
Notifications can coalesce while counters remain exact. Recover the structured
`AvailabilityFailure` with `availability_failure()` on native or remote errors;
retain its cause/status/hint and termination reason. `Recovered` is a successful
read, while the recovery counter counts reads requiring additional attempts.

Availability owns retry attempts and disables nested client retries in those reads.
Its elapsed limit includes request time, shared waits and backoff; choose longer
than a normal 25-second long poll. Attempts share the client's `max_retry_wait`
allowance for server-advised/shared waits. Additional outage backoff has its own
elapsed budget. These are distinct limits: increasing one cannot renew the other.
Authentication/configuration/decode and exhausted client wait budgets stay terminal.

`BotHandle::stop()` cancels reads/backoff. Startup cancellation returns a structured
availability failure; intake stop drains accepted work and commits only completed
prefixes. Keep intake failures separate from handler failure callbacks and cursor
storage errors. Do not add a catch-all retry loop or tune budgets without evidence.

## Handler failures and stopped intake

Determine whether the failure came from a command, button, server-message gate or
post-command listener. `on_command_failure`, `on_button_failure` and
`on_message_failure` supply explicit dispositions; legacy `()` callbacks observe
failures without completing them. Typed `CommandError` input/check rejections
complete after a successful reply; failed replies and unexpected errors remain
incomplete by default. Preserve `FailureHandling.source` and `handling_error`.

A shared `message_gate` runs before native/worker command dispatch. Gate Handled
skips downstream handlers; Stop is incomplete. Fallible natural-language listeners
use `on_message` or `WorkerCog::on_message`. Handled failure callbacks
complete a listener invocation but do not claim the remaining listeners; returning
`MessageDisposition::Handled` from the listener does. Post-command rejection does
not undo a command's earlier effects.

Inspect `EventProcessing` reports: failed IDs, not-started events, completed but
uncommitted work, and independent intake/cursor causes. Drain/cancellation does
not silently complete unfinished events. Worker timeout/completion loss is
uncertain. Reconcile durable application receipts before explicit replay; a
cursor-write error does not make completed work safe to repeat. Custom
`process_event` calls own their deduplication and checkpoint behavior.

## Authentication and permissions

401 requires corrected credentials, not retries. For 403 distinguish bot server
permissions, requester authorization, effective channel/category access, hierarchy,
override/grant restrictions and endpoint rollout. 404 may hide an invisible or
foreign resource. Keep bot and requester separate; a bot read or server-wide
Manage Channels does not establish requester ACLs. Use scoped fresh membership/
role evidence rather than stale custom caches when diagnosing revocation.

`ValidatedOperation` reports Allowed, Denied and Unknown required checks. Missing
public ACL precedence, hierarchy or rollout evidence remains Unknown; do not
replace it with a blanket allow policy. `test_server_access` needs trusted host
evidence. `RoleHierarchyPolicy` defaults conservative; `owner_admin_strict()` is
an explicit application rule, not proof of undocumented platform behavior.
Owner requests do not elevate the bot. A read-before-delete assessment is not an
atomic reservation; preserve a later 403 or uncertain delete result as returned.

## Writes and rate limits

Inspect `status`, `retry_after`, `retry_safety`, `outcome` and confirmed receipts.
Normal Client calls already apply bounded supported server-advised retries;
`request_once` and validated role-deletion composition intentionally do not replay
the mutation. Never retry a whole command because one request supplied Retry-After.
502/504, write cancellation/lost completion or malformed successful responses may
leave an unknown outcome. Reconcile state before deciding on a deliberate retry.
Matching message text alone is not proof of which request created it. Split-send
errors retain completed steps and failed-step index; replay duplicates successes.

## Worker builds and watchers

Inspect `hot_reload_failure()` on native/remote errors or callbacks. Retain phase:
Compilation, Startup, Protocol, Registration or Publication. Useful evidence is
compiler code/message/project-relative spans, `exit`, `timed_out`,
`candidate_started`, `publication_attempted`, `routes_published`, `source`, and
`truncated`. An absent exit is not success. Unpublished local routes do not prove
remote rollback; failed candidates normally leave active routes installed.

`WatchGuard::status()` separates `scan_error`, `reload_error` and `reload_failure`.
Recovered scans clear only scan errors, even with unchanged digest; a failed
candidate stays visible until a changed candidate installs. Use path/phase context
for missing files; distinguish transient runtime churn from required source/input
loss. Do not suppress every ENOENT or repeatedly rebuild to clear an alert.

Check parent and worker revisions. Both require protocol 11 and must be rebuilt
together. Earlier versions fail at startup. UI callbacks, operation identities
and single-attempt requests are available on all worker routes. Stdout belongs
to IPC. Review diagnostics before sharing. Builds, procedural macros and workers
execute trusted code and can produce effects even when compilation fails.

API 1.4 computed permission reads can establish requester and bot access
independently. Use `get_member_permissions` with the actual server/user/channel.
A missing flag stays unknown. Rollout-related 403 responses concern availability.
Resource lifecycle worker listeners require protocol 11. See the
[API 1.4 guide](https://github.com/Lost1419/Rappin/blob/dev/docs/resources.md).

## API origin

Doctor reports configured and active hosts. Default REST failover is restricted
to documented bot/website API origins; custom bases never fail over. Preserve
custom endpoint choices and do not re-enable redirects as a speculative fix.
