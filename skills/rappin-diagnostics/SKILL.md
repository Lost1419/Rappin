---
name: rappin-diagnostics
description: "Investigate running Yapin/Rappin bot failures: latency, missing events, stopped intake, authorization, retries, and worker reload errors. Use bounded diagnostics and existing evidence, not load benchmarks or ordinary feature implementation."
---

Inspect the installed SDK/CLI version and existing application evidence first.
`rappin doctor --offline` checks local setup; a bounded live `rappin doctor` checks
its own authentication/reachability when needed. Inspect `authenticated`,
`reachable`, `active_base_url` and `last_request`, not only its exit status.
`rappin capabilities` identifies compiled features. Never print credentials or
configuration contents. Doctor cannot inspect another running process's metrics.

Choose the relevant section of [troubleshooting](references/troubleshooting.md):

- [Slow replies](references/troubleshooting.md#slow-replies) and
  [missing events](references/troubleshooting.md#missing-events-or-reconnects).
- [Startup/intake availability](references/troubleshooting.md#startup-and-intake-availability).
- [Message/handler completion](references/troubleshooting.md#handler-failures-and-stopped-intake).
- [Authorization](references/troubleshooting.md#authentication-and-permissions),
  [writes](references/troubleshooting.md#writes-and-rate-limits), or
  [worker builds/watchers](references/troubleshooting.md#worker-builds-and-watchers).

Establish whether the evidence comes from the application, SDK transport or Yapin
before assigning blame. Retain sanitized status/kind, timing boundaries, attempts,
received/committed cursors, failure phase and a minimal reproduction. Report
Unknown separately from Denied; preserve partial receipts and uncertain outcomes.
Do not turn an unknown mutation into Handled merely to keep intake running.

Use bounded reads/exact IDs and local fixtures before additional live traffic.
Honor the user's existing authorization; do not clear cursors, force acknowledgements,
change permissions, rebuild source, restart processes or resend uncertain writes
as speculative fixes. Propose a concrete correction supported by the evidence,
then validate within the authorized scope. This skill does not run load benchmarks
or assume any separate benchmarking project exists.
