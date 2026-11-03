# Capability bindings

Discover API operations, validate calls and build runtime commands from the same
catalogue used by Rappin's CLI and MCP tools. Capability bindings combine typed
argument validation, server scope and separate requester/bot authorization through
the shared `Api` interface.

Use them for application-defined commands, administrative workflows or AI tool
calling. Applications choose which operations to expose and retain control of
their storage, cache backend, command prefixes and model integration.

## Public APIs and feature requirements

All APIs below are in **`rappin::capabilities`**, available with **default
features disabled**. No CLI, Clap, configuration files, authentication or network
access is needed for discovery, schema derivation or validation. Executing API
operations needs an `Api` implementation. `Client`, native contexts and `Arc<T>`
already implement it; `WorkerContext` requires the existing `hot-reload` feature.

| API | Contract |
|---|---|
| `catalogue() -> &'static [Capability]` | Shared 56-operation inventory used by CLI/MCP |
| `capability(name) -> Option<&'static Capability>` | Exact operation lookup |
| `filtered_catalogue(&CapabilityFilter)` | Select by names, domains, write/read, library support and test-server restriction |
| `CapabilityFilter::validate(name, &Value)` | Enforce the selected inventory and validate a call |
| `Capability::{input_schema, tool_definition}` | Offline, self-contained argument schemas and model/tool metadata |
| `Capability::validate(&Value) -> Result<ValidatedOperation>` | Offline argument and typed SDK validation |
| `ValidatedOperation::{name, arguments, capability, preview}` | Inspect canonical arguments; previews make no authorization claim |
| `ValidatedOperation::{assess, assess_with_policy}` | Read-only authorization assessment |
| `ValidatedOperation::{execute, execute_with_policy}` | Reassess, then execute only if every required check is allowed |
| `OperationContext::server(server_id, requester_id)` | Host-owned scope and actor; never derive these identities from model output |
| `OperationContext::{freshness, test_server_access}` | Explicit cache freshness and trusted availability facts |
| `OperationPolicy::check(&AccessQuestion)` | Resolve facts the SDK cannot establish; default `NoAdditionalAccess` returns unknown |
| `AuthorizationReport::{decision, allowed}` | Aggregate allowed/denied/unknown while retaining separate principal checks |
| `OperationCommandBinding::{validate, command_schema, prepare, preflight, bind}` | Store a command definition, rebuild matching help/parsing, preflight ownership, bind a handler |

Supporting public types are `CapabilityDomain`, `OperationScope`,
`PermissionRequirements`, `PermissionRule`, `Availability`, `LibrarySupport`,
`OperationArgument`, `OperationParameter`, `AccessDecision`, `Principal`,
`ExternalCheck`, `AccessQuestion`, `CheckResolution`, `AuthorizationCheck`,
`AuthorizationReport` and `OperationResult`. `bundled_schema`, `endpoint_schema`,
`tools` and `output_schema` expose the same embedded discovery data used by CLI.
`validate_schema` validates the subset used by the catalogue; it is not a
general-purpose validator for arbitrary external schema dialects.

The `Capability` descriptor includes the operation name, description, argument
definitions, permission rules, scope, availability and CLI mapping. These are
metadata, not authority: validation uses the registered canonical definition,
even if an application edits a cloned descriptor. Filtering alone does not
authorize execution. Use the host-selected filter's `validate` method when a
model is restricted to a subset, then enforce execution authorization as well.

## Discover and execute an operation

```rust,no_run
use rappin::{Api, Result, capabilities::*};
use serde_json::{json, Value};
use uuid::Uuid;

async fn run_plan(api: &(impl Api + ?Sized), server: Uuid, requester: Uuid) -> Result<()> {
    let selection = CapabilityFilter {
        names: vec!["list_roles".into()],
        library_only: true,
        include_test_servers: false,
        ..Default::default()
    };
    let model_metadata: Vec<Value> = filtered_catalogue(&selection)
        .into_iter().map(Capability::tool_definition).collect();
    // Supply these definitions to your planner or application UI, then validate
    // its selected operation against the same inventory before execution.
    let _ = model_metadata;
    let call = selection.validate("list_roles", &json!({"id": server}))?;
    let result = call.execute(api, &OperationContext::server(server, requester)).await?;
    println!("{}", result.data);
    Ok(())
}
```

See [`examples/planner_bindings.rs`](https://github.com/Lost1419/Rappin/blob/main/examples/planner_bindings.rs) for an
offline executable and a permanent-command installation helper. Run
`cargo run --example planner_bindings` to print capabilities without credentials.
The library can be used as `rappin-rs = { version = "0.1.17", default-features = false }`.

## Channel and category management

The following operations are available through the library, CLI and MCP without
an additional Cargo feature in the library:

| Operation | Arguments | Changes |
|---|---|---|
| `create_channel` | `server`, `body` | Name, `kind: "text"` or `"voice"`, optional `category_id` |
| `edit_channel` | `id`, `body` | Name, category, slowmode, read-only and privacy settings |
| `delete_channel` | `id` | Delete the specified channel |
| `create_category` | `id` (server), `body` | Name and optional signed 32-bit position |
| `edit_category` | `id` (server), `category`, `body` | Rename the specified category |
| `delete_category` | `id` (server), `category` | Delete the specified category |

Bodies use the SDK's `CreateChannelRequest`, `EditChannelRequest`,
`CreateCategoryRequest` and `EditCategoryRequest` contracts. Slowmode is an integer
from 0 through 21600 seconds; 0 disables it. For channel edits, omitting
`category_id` keeps the current category, a UUID moves the channel, and JSON
`null` removes the category. `false` remains a supplied boolean value.

Both principals need Manage Channels. Existing channel scope and effective
channel permission checks apply to edits/deletes. Category references and privacy
role lists must belong to the host server. A verified category ID alone does not
establish access: `ExternalCheck::CategoryPermissions` asks the host policy about
each principal when assigning a destination category or editing/deleting one.
Privacy and role-list changes also require `OverrideAuthority` evidence.
Unestablished checks remain unknown and block execution. These six operations
are not marked test-server-only; permission override operations have their own
availability restrictions.

The CLI uses `channels create/edit/delete` and
`servers category-create/category-edit/category-delete`. Supply JSON bodies with
`--body-file`; named `tools call` and MCP operations accept an inline `body`.
Dry runs validate and preview without claiming authorization or sending writes.

## Authorization and scope

Requester and bot are independent principals. The SDK obtains the bot ID through
`Api`, evaluates each actor with the shared permission evaluator, and never
substitutes the bot's privileges for the requester's. Server owners,
Administrator, @everyone, incomplete snapshots and custom cache backends follow
the existing evaluator's rules. `Freshness::Refresh` is the default.

Server arguments must match the host context. Optional-server operations are
narrowed to that server; library `list_servers` returns only the host server.
Channel IDs are checked through the bot API, including the returned identity and
server ID. DMs or missing scope metadata do not establish server access. Referenced
roles, categories, reply messages and scheduled-event channels receive scope
checks too. An application policy cannot override a failed SDK scope check.
Member reads through these bindings omit invite attribution, which bot privileges
alone must not disclose to an ordinary requester. The standalone typed SDK still
retains those fields for trusted bot workflows.

Server permission bits are not proof of a requester's channel visibility or
effective posting rights. The SDK records successful bot channel reads, but asks
the host policy for other effective channel permissions, content rules, role
hierarchy, grant/override authority and event-destination access. A channel being
in the correct server does not establish either principal's access to it.
Test-server-only endpoints need a trusted
host availability fact (`test_server_access`) or policy evidence. Missing facts
remain **unknown**, not implicitly allowed. Policies receive the exact principal,
actor, server, channel, required permission names and canonical arguments.
They must derive their answers from trusted application/API data, not planner
claims. `CheckResolution::{allowed, denied, unknown}` include a human reason.

An assessment is diagnostic, not a reservation. `execute` checks again before
performing an action; it does not accept a caller-supplied "approved" report.
Yapin still enforces concurrent changes and server rules. Permission-policy
errors retain sanitized structured causes in the report. The error variant
`YappinError::OperationAuthorization { report }` distinguishes denied from
unknown; `to_json()` exposes `decision` and `authorization.checks` with separate
requester/bot/context entries. No mutation has run in that case.

`preview()` and `dry_run:true` are offline. Their `OperationResult.authorization`
is `None`, so a successful preview must not be presented as proof of access.
Successful execution returns `Some(AuthorizationReport)` along with sanitized
`data`, the operation name and `dry_run:false`. Collection limits/cursors use the
same implementation as CLI/MCP.

Global event polling, CLI event receipts and CLI doctor state cannot be supplied
by the generic server-scoped `Api` interface. Their catalogue availability is
explicit; execution reports an unknown runtime requirement and permanent
bindings reject them. Keep the bot's event loop outside the AI capability list.

The binding executor adds no retry loop. Existing Client retries remain limited
to server-advised requests known not to have applied; uncertain writes are not
automatically replayed. Split sends preserve completed results and the failed
step through `PartialWrite`. Do not retry a complete feature/workflow after an
unknown outcome or partial write.
Event delivery remains at-least-once. Applications should persist invocation/outcome
receipts in their own storage and reconcile uncertain writes before re-executing
a command after restart; the binding layer does not add durable deduplication.

## Permanent commands

Store an `OperationCommandBinding` as ordinary serializable bot data:

```json
{
  "operation": "send_message",
  "name": "announce",
  "description": "Post an announcement",
  "aliases": ["ann"],
  "fixed_arguments": {"channel": "00000000-0000-0000-0000-000000000004"},
  "parameters": [{"name":"text", "target":"/content", "required":true, "rest":true}]
}
```

`command_schema()` derives the argument types, constraints and help from operation
metadata. Parameters target JSON pointers such as `/content` or `/body/name`.
They cannot shadow fixed values or overlap other parameter targets. Bind a whole
object/array as a quoted JSON argument when needed; array indexing is not a
parameter-target syntax. `ArgumentType::Schema` preserves catalogue validation,
including unsigned IDs, booleans and JSON payloads. Typed `FromArgs` commands
continue to use their original parser.

`binding.bind(handler)` returns a `RegisteredCommand` whose `info` already
matches parsing and aliases. The handler receives a fully validated operation;
it still explicitly executes it with a host-derived context and policy. Cross-field
payload rules are checked after runtime arguments are supplied, before the
handler executes. `binding.preflight(handle, Some(cog_name))` checks definitions
and naming/ownership conflicts without network access. Register the result with
`CogBuilder::registered_command`, then use the existing cog installation/reload
path. Workers can use `command_schema()` with `WorkerCog::dynamic_command` and
`prepare()`; existing single-name worker routing constraints remain unchanged.

Preflight is advisory and final publication revalidates under the registry lock.
An invalid candidate or failed slash-command publication retains old local
routes/help. Applications manage their own database transactions and reconcile
ambiguous remote publication outcomes before trying again.

## Application integration

- **Saved command definitions:** store the operation name, fixed arguments and
  parameter mapping in your chosen database. Retain ownership, per-server
  enablement, quotas and application ACLs. Reconstruct `OperationCommandBinding`,
  preflight, then `bind` and install through the native cog API. Mark a definition
  active after installation succeeds and keep the prior active definition if
  publication fails. Parsing and help come from the binding; existing prefix
  resolvers and dispatch policies continue to apply.
- **Shared API helpers:** accept `&(impl Api + ?Sized)` to support native and worker
  contexts. Validate through the selected filter and execute with an
  `OperationContext` derived from the authenticated event. Implement
  `OperationPolicy` using trusted access data for unresolved checks. Preserve
  structured errors and retry outcomes, report unknown checks, and keep requester
  and bot authorization separate.
- **AI tools:** serialize only the selected `tool_definition()` values.
  Filter by enabled features/domains, read/write needs, library support and trusted
  availability. Require an operation name plus JSON arguments; keep identity,
  scope and policy inputs out of model control. Validate against the same filter
  on receipt. Model selection, prompting, inference, conversation state and storage
  remain application choices.
