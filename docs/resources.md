# Resources and computed permissions

Rappin supports the public API 1.4 contract captured on 12 November 2026. The new
resource endpoints and lifecycle events require the platform's `bot_api_round2`
rollout. The developer guide limits this to test servers and eligible test bot
owners. Preserve a returned 403 and its cause when eligibility is unavailable.

## Computed permissions

`ApiExt` works on `Client`, native contexts, worker contexts and custom
`Api` implementations. The client also exposes inherent forwarding methods.

```rust,no_run
use rappin::{Api, ApiExt, Result};
use uuid::Uuid;

async fn inspect(api: &impl Api, server: Uuid, requester: Uuid, bot: Uuid, channel: Uuid) -> Result<()> {
    for actor in [requester, bot] {
        let access = api.get_member_permissions(server, actor, Some(channel)).await?;
        println!("{actor}: {:?}", access.permission("manage_messages"));
    }
    Ok(())
}
```

Each call fetches fresh data. `None` selects server-wide permissions. The SDK
verifies the returned discriminator, server, user and channel. A missing flag
returns `None` from `MemberPermissions::permission`. Owner or Administrator
produces `Some(true)` under the documented platform bypass. Channel responses
incorporate roles, everyone, category/channel overrides and channel visibility.
The result describes the assessed actor. Fetch requester and bot independently.

`ComputedPermissionsPolicy::new(api)` implements `OperationPolicy` for
`ExternalCheck::ChannelPermissions`. It supplies grant, denial or unknown from
fresh computed results. Other external questions return Unknown. Compose it with
application policies for content, category authority, hierarchy and rollout.
`ValidatedOperation::execute_with_policy` reassesses access before dispatch.
Assessment neither reserves access nor makes the following mutation idempotent.

```rust,no_run
use rappin::{Api, ComputedPermissionsPolicy, Result};
use rappin::capabilities::{OperationContext, ValidatedOperation};

async fn execute(api: &impl Api, operation: &ValidatedOperation, context: &OperationContext) -> Result<()> {
    let policy = ComputedPermissionsPolicy::new(api);
    let report = operation.assess_with_policy(api, context, &policy).await?;
    if report.allowed() {
        operation.execute_with_policy(api, context, &policy).await?;
    }
    Ok(())
}
```

The existing local `PermissionSnapshot` and application-supplied policies remain
available. Category-level effective permissions have no direct computed endpoint
in this contract. A category authority question can therefore remain Unknown.
The role-order additions do not establish every owner, administrator-peer, tie
or protected-role rule for arbitrary requester authorization.

## Resource methods

| Method | Meaning |
|---|---|
| `resolve_id(id)` | Read visible object identity and available server/channel scope |
| `get_member_permissions(server, user, channel)` | Read fresh server or channel effective access |
| `get_role(server, role)` | Read one role |
| `reorder_roles(server, ids)` | Submit highest-first role IDs |
| `reposition_roles(server, positions)` | Submit the alternate array of `RolePosition` values |
| `get_channel_overrides(channel)` | List channel role/everyone overrides |
| `set_channel_override` / `delete_channel_override` | Replace or remove a channel override |
| `get_category_overrides(category)` | List category role/everyone overrides |
| `set_category_override` / `delete_category_override` | Replace or remove a category override |
| `send_sticker(channel, body)` | Send a `StickerMessage` occupying the whole message |

Override methods use `PermissionTarget::Role(id)` or `PermissionTarget::Everyone`.
The platform rejects member-specific override writes. New reads retain explicit
target and resource identity in `ResourceOverride`. Legacy permission methods keep
their existing route behavior. GET of `/channels/{id}/permissions/{target}` takes
a user ID, while PUT and DELETE take a role ID or everyone.

The platform keeps roles at or above the bot's highest role fixed during
reordering. Duplicate role IDs fail local validation. Every mutation preserves
the existing structured transport cause and uncertain-write outcome. Reconcile
ambiguous writes before a consumer-controlled retry.

`ResolvedObject.object` preserves an extensible kind string. Hidden and unknown
IDs both produce the platform's 404. Resolver reads are limited to 120/minute.
The server-bound `resolve_id` capability requires the response to establish that
server. A global/user/DM lookup with unavailable server identity remains blocked
by the binding. The typed raw method supports host-authorized global lookup.

## Lifecycle events

`EventData::Resource(ResourceEvent)` covers channel, thread and category
`create`, `update` and `delete` notifications. `ResourceEvent.kind` carries the
event name. `data` retains the complete resource payload, including its own
`type` and future fields. Invalid identity/discriminator combinations fail
decoding and keep the event incomplete.

Implement fallible `EventHandler::on_resource_event` or `Cog::on_resource_event`.
The default event-handler method delegates to the existing `on_unknown_event`
hook. Callback errors and panics propagate through the normal checkpoint barrier.
Successful earlier events can commit while the failed event and later events stay
unprocessed. No handler replay or mutation retry is added by this callback.

Resource dispatch evicts affected built-in channel entries and invalidates server
lookup data through the configured cache backend. Category changes evict that
server's built-in channel entries. Applications retain cache storage ownership.
The platform sends these events only under its visibility rules. Channel trash
uses delete and restoration uses create. Whole-server deletion emits no channel
delete notifications. Channel create covers text/voice channels.

Workers subscribe with `WorkerCog::listen(Listener::ResourceEvent, handler)` and
match `CogEvent::ResourceEvent { event }`. This opt-in advertises protocol v9.
Older worker capabilities continue selecting their existing protocol version.
Upgrade the parent before enabling the new listener. Resource API methods use
the existing JSON proxy and preserve parent-held credentials and structured errors.

## Identity and prior contract gaps

Opt-in `UserId`, `ChannelId`, `ServerId`, `RoleId`, `MessageId`, `ThreadId`,
`CategoryId`, `EmojiId`, `StickerId`, `WebhookId` and `ScheduledEventId` types
serialize as UUID strings. Existing user/channel command parsers gain UUID
conversion, display and Serde support. Converting these labels grants no access.
`ApiObject::<T>::decode(value, expected_kind)` validates an `object` discriminator
and retains a typed view alongside it. Existing response DTO construction stays
compatible with applications already using UUID fields.

`CreateMessage.sticker_id` supports sticker creation through the rich-message
binding. Stickers require empty content, embeds, components and files. The named
`send_sticker` binding checks the sticker's server through the resolver and applies
rollout and separate actor checks. Sticker edits and mixed sticker payloads fail
local validation. Inline CLI/MCP arguments remain bounded to 128 KiB.

Override validation accepts `use_activities`. JSON and multipart emoji helpers
accept up to 2 MiB decoded. Worker uploads use the existing chunked transport and
frame bounds. The CLI's `emojis upload` command accepts the same image bound.

## CLI, MCP and permanent commands

The public catalogue includes `resolve_id`, `get_member_permissions`,
`get_channel_member_permissions`, `get_role`, `reorder_roles`,
`get_channel_overrides`, `set_channel_override`, `delete_channel_override`,
`get_category_overrides`, `set_category_override`, `delete_category_override`
and `send_sticker`. They appear in MCP discovery and execute through
`rappin platform OPERATION --arguments-file PATH`.

Library bindings enforce the authenticated host server, referenced roles and
categories, response scope and independent actor requirements. Unknown checks
block execution. CLI/MCP operator execution uses the configured bot token and its
platform authority. Model planning, command selection and durable receipts belong
to the consumer.

Use `OperationCommandBinding` to derive parsing and help from these schemas.
Registration preflight and command/cog replacement use the existing APIs.
Revalidate saved definitions against the installed catalogue before publication.

The [official API reference](https://www.yapin.gg/developers/api) and
[developer guide](https://www.yapin.gg/developers) describe platform rollout and
permissions. The SDK's public schema is an offline snapshot of that contract.
