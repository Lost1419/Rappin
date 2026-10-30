---
name: rappin-diagnostics
description: "Diagnose Yapin bot failures, latency, missing events, permissions, retries, and gateway fallback using Rappin diagnostics and bounded reads. Use for troubleshooting rather than API load benchmarking."
---

Start with `rappin doctor --offline`, then `rappin doctor` for a bounded live auth
check. Inspect `authenticated`, `reachable`, `active_base_url`, and `last_request`;
doctor's successful exit alone does not establish authentication. Use
`rappin capabilities` to identify compiled features and available tools.
Do not print tokens or configuration contents.

Choose the investigation from the symptom; read only the relevant part of
[the troubleshooting reference](references/troubleshooting.md). Establish whether
an observation came from the SDK, its transport, a handler, or the Yapin service
before assigning blame. Retain sanitized status codes, timing boundaries,
attempt counts, cursor values and a minimal reproduction.

Use bounded history and exact IDs when possible. Do not clear state, force an
acknowledgement, change permissions or resend an uncertain write just to make an
error disappear. Honor the user's existing authorization; ask only for scope or
information actually needed. Existing log evidence and local fixtures should
precede additional live traffic. This skill does not run load tests; use the
separate YapBench project when benchmarking is explicitly requested.
