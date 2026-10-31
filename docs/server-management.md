# Server management APIs

The [Yapin developer guide](https://www.yapin.gg/developers), checked on
31 October 2026, documents role management, permission overrides, live counts,
invites and scheduled events. **These endpoints are currently available only on
test servers.** Rappin exposes them without assuming that a bot has access.
Permission errors from Yapin remain visible to the caller.

The separate API reference and upstream OpenAPI file do not yet contain these
endpoints. `docs/openapi.json` remains the upstream specification;
`docs/guide-api.json` supplements CLI/MCP schema discovery with guide-derived
operations marked `x-rappin-source` and `x-yapin-test-servers-only`. Their response
schemas deliberately remain open objects until upstream publishes full schemas.

## Rust client

```rust,no_run
use rappin::{Client, CreateRole, EditRole, PermissionOverride, PermissionTarget,
             CreateServerEvent, EventRepeat, Result};
use uuid::Uuid;

async fn configure(client: &Client, server: Uuid, channel: Uuid) -> Result<()> {
    let mut request = CreateRole::new("Helpers");
    request.mentionable = Some(false);
    request.permissions = Some([("manage_messages".into(), true)].into());
    let role = client.create_role(server, &request).await?;
    client.edit_role(server, role.id, &EditRole {
        permissions: Some([("send_messages".into(), false)].into()),
        ..Default::default()
    }).await?;
    client.set_channel_permissions(channel, PermissionTarget::Role(role.id),
        &PermissionOverride {
            allow: vec!["view_channel".into()],
            deny: vec!["send_messages".into()],
        }).await?;

    let counts = client.get_member_counts(server).await?;
    println!("{} members, {} online", counts.member_count, counts.online_count);
    let mut event = CreateServerEvent::new("Meetup",
        "2026-12-01T18:00:00Z".parse().unwrap());
    event.channel_id = Some(channel); // must be a voice channel
    event.repeat = Some(EventRepeat::Weekly);
    event.tz = Some("America/Phoenix".into());
    client.create_server_event(server, &event).await?;
    Ok(())
}
```

- Role create/edit/delete and `set_everyone_permissions` require Manage Roles.
  Yapin enforces hierarchy, automatic-role restrictions and the permissions the
  bot can grant. PATCH changes only supplied keys; `Some(false)` is a real
  change, not an omitted field. Permission names remain extensible strings.
- Channel/category overrides require Manage Roles or Manage Channels. Use
  `PermissionTarget::Everyone` for @everyone, empty lists to clear an override,
  or `clear_channel_permissions` / `clear_category_permissions` to delete it.
  `get_channel_permissions` returns null role IDs for @everyone.
- Role and category-permission writes invalidate member/role snapshots in the
  built-in or application-owned lookup cache, including ambiguous failures.
  Rappin does not add a database or change application prefix resolution.
- `get_member_counts` returns member and online counts. Online counts update
  roughly once a minute; query every few minutes rather than on every message.
  `Server::online_count` is optional for servers without this feature.
- `get_invites` needs Manage Server. `MemberDetail` and `MemberJoinEvent` retain
  optional `invite_code` and `inviter_id`. Missing/null values do not prove that
  someone joined without an invite: attribution may be unavailable.
- `invite_create` / `invite_delete` dispatch to `EventHandler` and native cog
  hooks, typed `InviteEvent` collectors, and hot-worker listeners. Rebuild workers
  with an updated host before registering these new listener variants.
- `get_server_events` lists upcoming/live events; `create_server_event` requires
  Manage Events or Manage Server. Use a voice `channel_id` or an external
  `location`, plus optional end, announcement channel, repeat and time zone.
  Yapin enforces its limit of 25 upcoming events. Local validation checks the
  destination, nonempty title and end-after-start ordering.

## CLI and MCP

```sh
rappin servers counts "$SERVER"
rappin servers invites "$SERVER" --max-items 20
rappin servers roles "$SERVER"
rappin servers role-create "$SERVER" --body-file role.json --dry-run
rappin channels permissions "$CHANNEL"
rappin channels permissions-set "$CHANNEL" everyone --body-file override.json --dry-run
rappin servers scheduled-events "$SERVER"
rappin servers event-create "$SERVER" --body-file event.json --dry-run
rappin schema --method POST --path '/servers/{id}/events'
```

`role.json` uses `name`, optional `color`, `mentionable`, and `permissions`.
`override.json` uses `allow` and `deny` arrays. `event.json` uses the snake_case
fields of `CreateServerEvent`. `servers everyone-permissions` expects
`{"permissions":{"send_messages":false}}`, matching the HTTP body.

The shared MCP catalog exposes `get_member_counts`, `list_invites`, `list_roles`,
`create_role`, `edit_role`, `delete_role`, `set_everyone_permissions`,
`get_channel_permissions`, `set_channel_permissions`, `clear_channel_permissions`,
`set_category_permissions`, `clear_category_permissions`, `list_server_events`
and `create_server_event`. Resource UUIDs use the `id` argument; role/category
IDs use `role`/`category`, and override targets use `target` (UUID or `everyone`).
Bodies are inline JSON objects. Writes support dry runs; edits and removals have
destructive annotations. List output supports fields/items/bytes and local
offset continuation, which re-fetches the list and can shift if the server changes.

## Source compatibility

Existing JSON remains decodable. Rust struct literals of `Server`, `Role`,
`MemberDetail` and `MemberJoinEvent` need their new optional fields. Exhaustive
matches on `EventData`, hot-reload `Listener`, or `CogEvent` need the invite
variants. Existing handler/cog implementations inherit no-op invite hooks.
