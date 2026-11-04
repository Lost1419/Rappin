# Bindings, runtime commands and authorization

Use the public APIs provided by the application's selected dependency. The
catalogue/bindings require no optional Cargo features; the hierarchy and
single-attempt helpers below require Rappin 0.1.20+. Keep model selection,
prompts, inference, conversation state and persistence in the application.

## Discover and validate once

`capabilities::{catalogue, capability, filtered_catalogue, CapabilityFilter}`
provide offline operation discovery without authentication, network or CLI/Clap.
Export `Capability::tool_definition` for an application's planner, rather than
manually duplicating SDK argument schemas and permission metadata.

Use one host-owned `CapabilityFilter` to select the model's tools and to validate
its returned choice with `filter.validate(name, arguments)`. Filtering is not
authorization. Never execute a tool name outside that selection through the raw
API escape hatch. `Capability::validate` returns a `ValidatedOperation` and
reuses typed SDK validation. `preview()` is offline and makes no access claim.

Execute through the shared `Api` / `ApiExt` interface, implemented by Client,
native contexts and WorkerContext. Derive `OperationContext::server(server,
requester)` from the authenticated event/host; model-provided identities cannot
establish scope or requester authority. `assess_with_policy` is read-only;
`execute_with_policy` reassesses before execution. Assessments are not reservations.

## Register commands from metadata

For a reusable operation alias, use `OperationCommandBinding`, retaining friendly
names/aliases and saved definitions in application storage. Its `command_schema`
derives parsing, validation and help from the same metadata. `bind` produces a
`RegisteredCommand` with matching `info`; do not patch its metadata afterwards.
Bind the command into a cog using `CogBuilder::registered_command`.

Call `preflight(handle, Some(cog_name))` to check naming/ownership before installing
or replacing that cog. Publication still validates under the registry lock;
preflight does not guarantee a later registration or remote synchronization will
succeed. Preserve existing routes and saved definitions when publication fails.
Use `CommandSchema` / `dynamic_command` for runtime schemas that are not operation
aliases. Existing typed commands should retain their parser and behavior.
Workers can combine `command_schema`, `WorkerCog::dynamic_command` and `prepare`;
do not assume native alias routing is identical to worker routing.

## Establish authority separately for each principal

Treat `AuthorizationReport` as Allowed, Denied or Unknown with independent
requester/bot/context checks. An Unknown required check blocks execution too.
`OperationPolicy::check` supplies evidence for specific `ExternalCheck` questions;
it cannot waive a failed SDK scope check. Do not implement a blanket Allowed
policy to get around missing evidence.

A successful bot read and server-wide permission bits do not establish the
requester's effective channel/category access. Destination category assignment,
privacy overrides, grantable permissions and rollout eligibility can require
additional evidence. Preserve Unknown where public data does not establish the
required fact. `test_server_access(true)` is trusted host configuration, never a
model argument or an inference from a successful unrelated API read.

`RoleHierarchyPolicy::new(&api)` re-fetches scoped role/member evidence and stays
conservative about undocumented requester/tie/owner/admin semantics. Only opt into
`.owner_admin_strict()` when the application's policy calls for it: owner above
editable normal roles; nonowners cannot target administrator roles; other targets
must be strictly below the actor's highest assigned role. Requester and bot are
independent; an owner's request does not elevate the bot. Everyone/automatic/
system targets stay protected, and missing evidence stays Unknown.

For validated role deletion use `RoleHierarchyPolicy::execute_deletion`, or
`execute_role_deletion` with a composed host policy. These force fresh checks and
`Api::request_once`; custom adapters must implement that method explicitly.
Workers need `WorkerCog::single_attempt_api()` and a compatible parent. No DELETE
is automatically replayed, even with Retry-After. There is no atomic API operation
combining requester assessment and deletion: account for revocation between them.

Keep `YappinError::OperationAuthorization` reports, HTTP status/retry hints,
partial-write receipts and uncertain outcomes in application-owned receipts.
`ValidatedOperation` adds no whole-workflow retry or durable deduplication.

## Version-matched examples

The crate's `capability_bindings_guide` and `availability_and_hierarchy_guide`
contain tested examples. In a Rappin checkout, `examples/planner_bindings.rs` runs
discovery offline; `examples/role_hierarchy.rs` assesses by default and requires
an explicit flag for deletion. `examples/role_hierarchy_worker.rs` demonstrates
worker assessment without credentials. Compiling workers/build scripts executes
trusted code, even when their API workflow itself is read-only.
