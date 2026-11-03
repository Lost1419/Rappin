use super::*;
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capability {
    pub name: &'static str,
    pub description: &'static str,
    pub cli: &'static [&'static str],
    pub arguments: Vec<OperationArgument>,
    pub write: bool,
    pub destructive: bool,
    pub collection: bool,
    pub scope: OperationScope,
    pub permissions: PermissionRequirements,
    pub availability: Availability,
    pub domain: CapabilityDomain,
}
impl Capability {
    fn from_entry(mut entry: Entry) -> Self {
        let (scope, permissions, availability, domain) = metadata(entry.name);
        if let Some(field) = entry.fields.iter_mut().find(|f| f.name == "body") {
            if let Some(schema) = schema::body_schema(entry.name) {
                field.schema = schema;
            }
        }
        Self {
            name: entry.name,
            description: entry.description,
            cli: entry.cli,
            arguments: entry.fields,
            write: entry.write,
            destructive: entry.destructive,
            collection: entry.collection,
            scope,
            permissions,
            availability,
            domain,
        }
    }
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct OperationArgument {
    pub name: &'static str,
    pub flag: Option<&'static str>,
    pub schema: Value,
    pub required: bool,
}
struct Entry {
    name: &'static str,
    description: &'static str,
    cli: &'static [&'static str],
    fields: Vec<OperationArgument>,
    write: bool,
    destructive: bool,
    collection: bool,
}
fn field(
    name: &'static str,
    flag: Option<&'static str>,
    schema: Value,
    required: bool,
) -> OperationArgument {
    OperationArgument {
        name,
        flag,
        schema,
        required,
    }
}
fn id(name: &'static str, flag: Option<&'static str>, required: bool) -> OperationArgument {
    field(
        name,
        flag,
        json!({"type":"string","format":"uuid"}),
        required,
    )
}
fn text(name: &'static str, flag: Option<&'static str>, required: bool) -> OperationArgument {
    field(name, flag, json!({"type":"string"}), required)
}
fn number(name: &'static str, flag: &'static str, min: u64, max: u64) -> OperationArgument {
    field(
        name,
        Some(flag),
        json!({"type":"integer","minimum":min,"maximum":max}),
        false,
    )
}
fn body() -> OperationArgument {
    field(
        "body",
        Some("--body-file"),
        json!({"type":"object","description":"Inline JSON payload, not a file path"}),
        true,
    )
}
pub(super) fn entries() -> &'static [Capability] {
    static CATALOG: std::sync::LazyLock<Vec<Capability>> = std::sync::LazyLock::new(build_catalog);
    &CATALOG
}
fn build_catalog() -> Vec<Capability> {
    let mut entries=vec![
        Entry{name:"list_servers",description:"List servers visible to the bot",cli:&["servers","list"],fields:vec![],write:false,destructive:false,collection:true},
        Entry{name:"get_server",description:"Inspect one server by ID",cli:&["servers","get"],fields:vec![id("id",None,true)],write:false,destructive:false,collection:false},
        Entry{name:"list_channels",description:"List visible channels, optionally within a server",cli:&["channels","list"],fields:vec![id("server",Some("--server"),false)],write:false,destructive:false,collection:true},
        Entry{name:"resolve_channel",description:"Resolve a channel name; ambiguity is an error",cli:&["channels","resolve"],fields:vec![text("name",None,true),id("server",Some("--server"),false)],write:false,destructive:false,collection:false},
        Entry{name:"get_channel",description:"Inspect a channel by ID",cli:&["channels","get"],fields:vec![id("id",None,true)],write:false,destructive:false,collection:false},
        Entry{name:"list_members",description:"Read bounded member pages; cursor is a user UUID",cli:&["members","list"],fields:vec![id("server",None,true),number("limit","--limit",1,500),id("after",Some("--after"),false),number("max_pages","--max-pages",1,100)],write:false,destructive:false,collection:true},
        Entry{name:"resolve_member",description:"Resolve a member name within bounded pages",cli:&["members","resolve"],fields:vec![id("server",None,true),text("name",None,true),number("max_pages","--max-pages",1,100)],write:false,destructive:false,collection:false},
        Entry{name:"get_member",description:"Fetch current membership and roles",cli:&["members","get"],fields:vec![id("server",None,true),id("user",None,true)],write:false,destructive:false,collection:false},
        Entry{name:"read_messages",description:"Read bounded history; cursors are RFC3339 timestamps",cli:&["messages","list"],fields:vec![id("channel",None,true),number("limit","--limit",1,100),field("before",Some("--before"),json!({"type":"string","format":"date-time"}),false),field("after",Some("--after"),json!({"type":"string","format":"date-time"}),false),number("max_pages","--max-pages",1,100)],write:false,destructive:false,collection:true},
        Entry{name:"get_message",description:"Read one message and its author context",cli:&["messages","get"],fields:vec![id("channel",None,true),id("message",None,true)],write:false,destructive:false,collection:false},
        Entry{name:"send_message",description:"Send requested text or a rich-message body; dry_run previews without sending",cli:&["messages","send"],fields:vec![id("channel",None,true),field("content",Some("--content"),json!({"type":"string","description":"Up to 2000 characters unless split=true"}),false),field("body",Some("--body-file"),json!({"type":"object","description":"CreateMessage JSON with embeds/components/layout flags"}),false),id("reply_to",Some("--reply-to"),false)],write:true,destructive:false,collection:false},
        Entry{name:"edit_message",description:"Edit a bot message with an inline JSON body",cli:&["messages","edit"],fields:vec![id("channel",None,true),id("message",None,true),body()],write:true,destructive:true,collection:false},
        Entry{name:"delete_message",description:"Delete the specified message when requested",cli:&["messages","delete"],fields:vec![id("channel",None,true),id("message",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"add_reaction",description:"Add the bot's reaction to a message",cli:&["messages","react"],fields:vec![id("channel",None,true),id("message",None,true),text("emoji",None,true)],write:true,destructive:false,collection:false},
        Entry{name:"remove_reaction",description:"Remove the bot's reaction from a message",cli:&["messages","unreact"],fields:vec![id("channel",None,true),id("message",None,true),text("emoji",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"pin_message",description:"Pin a message when authorized",cli:&["messages","pin"],fields:vec![id("channel",None,true),id("message",None,true)],write:true,destructive:false,collection:false},
        Entry{name:"unpin_message",description:"Unpin a message when authorized",cli:&["messages","unpin"],fields:vec![id("channel",None,true),id("message",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"poll_events",description:"Deliver or replay a pending batch without acknowledging it",cli:&["events","poll"],fields:vec![number("after","--after",0,u64::MAX),number("wait","--wait",0,25),number("max_age","--max-age",0,86400)],write:false,destructive:false,collection:false},
        Entry{name:"ack_events",description:"Commit only a delivered event ID or batch watermark after processing it",cli:&["events","ack"],fields:vec![field("id",None,json!({"type":"integer","minimum":0}),true),text("batch_id",Some("--batch-id"),false)],write:true,destructive:true,collection:false},
        Entry{name:"doctor",description:"Diagnose auth, active API host and this request's timing; offline avoids networking",cli:&["doctor"],fields:vec![field("offline",Some("--offline"),json!({"type":"boolean"}),false)],write:false,destructive:false,collection:false},
        Entry{name:"get_capabilities",description:"Inspect command/tool capabilities and compiled optional features offline",cli:&["capabilities"],fields:vec![],write:false,destructive:false,collection:false},
        Entry{name:"get_schema",description:"Inspect one component or one method/path, with referenced definitions",cli:&["schema"],fields:vec![text("name",None,false),text("path",Some("--path"),false),text("method",Some("--method"),false)],write:false,destructive:false,collection:false},
    ];
    let target = || {
        field(
            "target",
            None,
            json!({"type":"string","description":"Role UUID or the literal everyone","anyOf":[{"const":"everyone"},{"format":"uuid"}]}),
            true,
        )
    };
    entries.extend([
        Entry{name:"create_channel",description:"Create a text or voice channel in a server; category access requires host evidence",cli:&["channels","create"],fields:vec![id("server",None,true),body()],write:true,destructive:false,collection:false},
        Entry{name:"edit_channel",description:"Rename, move or configure a channel; null category_id removes its category",cli:&["channels","edit"],fields:vec![id("id",None,true),body()],write:true,destructive:true,collection:false},
        Entry{name:"delete_channel",description:"Delete a channel when both principals can manage it",cli:&["channels","delete"],fields:vec![id("id",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"create_category",description:"Create a category in a server",cli:&["servers","category-create"],fields:vec![id("id",None,true),body()],write:true,destructive:false,collection:false},
        Entry{name:"edit_category",description:"Rename a category within its verified server scope",cli:&["servers","category-edit"],fields:vec![id("id",None,true),id("category",None,true),body()],write:true,destructive:true,collection:false},
        Entry{name:"delete_category",description:"Delete a category within its verified server scope",cli:&["servers","category-delete"],fields:vec![id("id",None,true),id("category",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"search_channels",description:"Return channel candidates with complete/unique/ambiguous search metadata",cli:&["channels","search"],fields:vec![text("name",None,true),id("server",Some("--server"),false)],write:false,destructive:false,collection:true},
        Entry{name:"search_members",description:"Return bounded member candidates; a full final page is incomplete, never confirmed unique",cli:&["members","search"],fields:vec![id("server",None,true),text("name",None,true),number("max_pages","--max-pages",1,100)],write:false,destructive:false,collection:true},
        Entry {
            name: "get_member_counts",
            description: "Live counts, refreshed roughly once a minute; test servers only",
            cli: &["servers", "counts"],
            fields: vec![id("id", None, true)],
            write: false,
            destructive: false,
            collection: false,
        },
        Entry {
            name: "list_invites",
            description: "List invites; Manage Server and test-server access required",
            cli: &["servers", "invites"],
            fields: vec![id("id", None, true)],
            write: false,
            destructive: false,
            collection: true,
        },
        Entry {
            name: "list_roles",
            description: "List server roles",
            cli: &["servers", "roles"],
            fields: vec![id("id", None, true)],
            write: false,
            destructive: false,
            collection: true,
        },
        Entry {
            name: "create_role",
            description:
                "Create a role; Manage Roles, hierarchy checks and test-server access apply",
            cli: &["servers", "role-create"],
            fields: vec![id("id", None, true), body()],
            write: true,
            destructive: false,
            collection: false,
        },
        Entry {
            name: "edit_role",
            description:
                "Patch role fields; only supplied permission keys change; test servers only",
            cli: &["servers", "role-edit"],
            fields: vec![id("id", None, true), id("role", None, true), body()],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "delete_role",
            description: "Delete a specified role when authorized; test servers only",
            cli: &["servers", "role-delete"],
            fields: vec![id("id", None, true), id("role", None, true)],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "set_everyone_permissions",
            description:
                "Patch everyone permissions using a permissions dictionary; test servers only",
            cli: &["servers", "everyone-permissions"],
            fields: vec![id("id", None, true), body()],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "get_channel_permissions",
            description: "List role overrides; null role_id means everyone; test servers only",
            cli: &["channels", "permissions"],
            fields: vec![id("id", None, true)],
            write: false,
            destructive: false,
            collection: true,
        },
        Entry {
            name: "set_channel_permissions",
            description: "Replace allow/deny lists for a role or everyone; test servers only",
            cli: &["channels", "permissions-set"],
            fields: vec![id("id", None, true), target(), body()],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "clear_channel_permissions",
            description: "Remove a specified channel override; test servers only",
            cli: &["channels", "permissions-clear"],
            fields: vec![id("id", None, true), target()],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "set_category_permissions",
            description: "Replace category allow/deny lists; test servers only",
            cli: &["servers", "category-permissions-set"],
            fields: vec![
                id("id", None, true),
                id("category", None, true),
                target(),
                body(),
            ],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "clear_category_permissions",
            description: "Remove a specified category override; test servers only",
            cli: &["servers", "category-permissions-clear"],
            fields: vec![id("id", None, true), id("category", None, true), target()],
            write: true,
            destructive: true,
            collection: false,
        },
        Entry {
            name: "list_server_events",
            description: "List upcoming/live scheduled server events; test servers only",
            cli: &["servers", "scheduled-events"],
            fields: vec![id("id", None, true)],
            write: false,
            destructive: false,
            collection: true,
        },
        Entry {
            name: "create_server_event",
            description:
                "Create a scheduled event; Manage Events or Manage Server; test servers only",
            cli: &["servers", "event-create"],
            fields: vec![id("id", None, true), body()],
            write: true,
            destructive: false,
            collection: false,
        },
    ]);
    entries.extend([
        Entry{name:"get_api_version",description:"Read public API build and SDK versions",cli:&["platform","get_api_version"],fields:vec![],write:false,destructive:false,collection:false},
        Entry{name:"get_api_changelog",description:"Read public API changelog",cli:&["platform","get_api_changelog"],fields:vec![number("limit","--limit",1,50)],write:false,destructive:false,collection:false},
        Entry{name:"list_emojis",description:"List server custom emojis and capacity",cli:&["platform","list_emojis"],fields:vec![id("id",None,true)],write:false,destructive:false,collection:true},
        Entry{name:"create_emoji",description:"Create a checked emoji; pending moderation is not a failure",cli:&["platform","create_emoji"],fields:vec![id("id",None,true),body()],write:true,destructive:false,collection:false},
        Entry{name:"edit_emoji",description:"Rename a server emoji",cli:&["platform","edit_emoji"],fields:vec![id("id",None,true),id("emoji",None,true),body()],write:true,destructive:true,collection:false},
        Entry{name:"delete_emoji",description:"Delete a server emoji",cli:&["platform","delete_emoji"],fields:vec![id("id",None,true),id("emoji",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"list_channel_webhooks",description:"List channel webhooks without tokens",cli:&["platform","list_channel_webhooks"],fields:vec![id("channel",None,true)],write:false,destructive:false,collection:true},
        Entry{name:"list_bot_webhooks",description:"List bot-owned webhooks; library bindings filter to their server",cli:&["platform","list_bot_webhooks"],fields:vec![],write:false,destructive:false,collection:true},
        Entry{name:"create_bot_webhook",description:"Create a bot-owned webhook in a channel",cli:&["platform","create_bot_webhook"],fields:vec![id("channel",None,true),body()],write:true,destructive:false,collection:false},
        Entry{name:"delete_bot_webhook",description:"Delete a bot-owned webhook in its verified channel",cli:&["platform","delete_bot_webhook"],fields:vec![id("channel",None,true),id("webhook",None,true)],write:true,destructive:true,collection:false},
        Entry{name:"post_bot_webhook",description:"Post with bot-owned webhook identity and allowed mentions",cli:&["platform","post_bot_webhook"],fields:vec![id("channel",None,true),id("webhook",None,true),body()],write:true,destructive:false,collection:false},
        Entry{name:"send_files",description:"Send inline base64 attachments; preserves partial-write receipts; max 128 KiB inline arguments",cli:&["platform","send_files"],fields:vec![id("channel",None,true),field("body",None,json!({"type":"object"}),false),field("files",None,json!({"type":"array","minItems":1,"maxItems":4,"items":{"type":"object","required":["name","data"],"additionalProperties":false,"properties":{"name":{"type":"string","maxLength":255},"data":{"type":"string","contentEncoding":"base64","maxLength":13981016}}}}),true)],write:true,destructive:false,collection:false},
    ]);
    for entry in &mut entries {
        if entry.name == "send_message" {
            entry.fields.push(field("split",Some("--split"),json!({"type":"boolean","description":"Split plain text into separate messages; errors report completed steps"}),false));
        }
        if entry.write {
            entry.fields.push(field(
                "dry_run",
                Some("--dry-run"),
                json!({"type":"boolean"}),
                false,
            ));
        }
        if entry.collection {
            entry.fields.extend([field("fields",Some("--fields"),json!({"type":"string","description":"Comma-separated item fields; IDs and timestamps are retained"}),false),number("max_items","--max-items",1,10000),number("max_bytes","--max-bytes",256,4194304)]);
            if !matches!(entry.name, "read_messages" | "list_members") {
                entry.fields.push(number("offset", "--offset", 0, 1000000));
            }
        }
    }
    entries.into_iter().map(Capability::from_entry).collect()
}
