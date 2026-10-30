//! One catalog drives MCP and `rappin tools`; execution still uses typed CLI commands.
use super::*;
use clap::CommandFactory;
struct Field {
    name: &'static str,
    flag: Option<&'static str>,
    schema: Value,
    required: bool,
}
struct Entry {
    name: &'static str,
    description: &'static str,
    cli: &'static [&'static str],
    fields: Vec<Field>,
    write: bool,
    destructive: bool,
    collection: bool,
}
fn field(name: &'static str, flag: Option<&'static str>, schema: Value, required: bool) -> Field {
    Field {
        name,
        flag,
        schema,
        required,
    }
}
fn id(name: &'static str, flag: Option<&'static str>, required: bool) -> Field {
    field(
        name,
        flag,
        json!({"type":"string","format":"uuid"}),
        required,
    )
}
fn text(name: &'static str, flag: Option<&'static str>, required: bool) -> Field {
    field(name, flag, json!({"type":"string"}), required)
}
fn number(name: &'static str, flag: &'static str, min: u64, max: u64) -> Field {
    field(
        name,
        Some(flag),
        json!({"type":"integer","minimum":min,"maximum":max}),
        false,
    )
}
fn body() -> Field {
    field(
        "body",
        Some("--body-file"),
        json!({"type":"object","description":"Inline JSON payload, not a file path"}),
        true,
    )
}
fn catalog() -> &'static [Entry] {
    static CATALOG: std::sync::LazyLock<Vec<Entry>> = std::sync::LazyLock::new(build_catalog);
    &CATALOG
}
fn build_catalog() -> Vec<Entry> {
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
            if matches!(entry.name, "list_servers" | "list_channels") {
                entry.fields.push(number("offset", "--offset", 0, 1000000));
            }
        }
    }
    entries
}
pub fn output_schema() -> Value {
    json!({"type":"object","properties":{"ok":{"type":"boolean"},"data":{"type":"object"},"error":{"type":"object"}},"required":["ok"],"additionalProperties":false,"oneOf":[{"properties":{"ok":{"const":true}},"required":["data"]},{"properties":{"ok":{"const":false}},"required":["error"]}]})
}
pub fn tools() -> Vec<Value> {
    catalog().iter().map(|entry| {
        let properties:serde_json::Map<String,Value>=entry.fields.iter().map(|field|(field.name.into(),field.schema.clone())).collect();
        let required:Vec<_>=entry.fields.iter().filter(|field|field.required).map(|field|field.name).collect();
        let mut schema=json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        if entry.name=="send_message" {
            schema["oneOf"]=json!([{"required":["content"]},{"required":["body"]}]);
            schema["allOf"]=json!([{"if":{"properties":{"split":{"const":true}},"required":["split"]},"then":{"required":["content"]},"else":{"properties":{"content":{"maxLength":2000}}}}]);
        } else if entry.name=="get_schema" {
            schema["oneOf"]=json!([{"required":["name"]},{"required":["path","method"]}]);
        } else if entry.name=="read_messages" {
            schema["not"]=json!({"required":["before","after"]});
        }
        json!({"name":entry.name,"description":entry.description,"inputSchema":schema,"outputSchema":output_schema(),"annotations":{"readOnlyHint":!entry.write,"destructiveHint":entry.destructive,"openWorldHint":!matches!(entry.name,"get_capabilities"|"get_schema"|"ack_events")}})
    }).collect()
}

pub fn is_write(name: &str) -> bool {
    catalog().iter().any(|e| e.name == name && e.write)
}
fn validate(entry: &Entry, args: &Value) -> Result<()> {
    let object = args
        .as_object()
        .ok_or_else(|| YappinError::validation("arguments", "expected an object"))?;
    for f in &entry.fields {
        if f.required && !object.contains_key(f.name) {
            return Err(YappinError::validation(f.name, "required argument"));
        }
    }
    for (key, value) in object {
        let f = entry.fields.iter().find(|f| f.name == key).ok_or_else(|| {
            YappinError::validation("arguments", "unknown argument; inspect tools/list")
        })?;
        let valid = match f.schema["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.as_u64().is_some(),
            Some("boolean") => value.is_boolean(),
            Some("object") => value.is_object(),
            _ => false,
        };
        if !valid {
            return Err(YappinError::validation(key, "invalid argument type"));
        }
        if let Some(n) = value.as_u64() {
            if f.schema["minimum"].as_u64().is_some_and(|v| n < v)
                || f.schema["maximum"].as_u64().is_some_and(|v| n > v)
            {
                return Err(YappinError::validation(key, "argument out of bounds"));
            }
        }
        if let Some(s) = value.as_str() {
            if f.schema["maxLength"]
                .as_u64()
                .is_some_and(|n| s.chars().count() > n as usize)
            {
                return Err(YappinError::validation(
                    key,
                    "string exceeds maximum length",
                ));
            }
            if f.schema["format"] == "uuid" && Uuid::parse_str(s).is_err() {
                return Err(YappinError::validation(key, "invalid UUID"));
            }
            if f.schema["format"] == "date-time" && DateTime::parse_from_rfc3339(s).is_err() {
                return Err(YappinError::validation(key, "invalid RFC3339 timestamp"));
            }
        }
    }
    if entry.name == "send_message" && object.contains_key("body") == object.contains_key("content")
    {
        return Err(YappinError::validation(
            "content",
            "provide exactly one of content or body",
        ));
    }
    if entry.name == "get_schema" && !object.contains_key("name") && !object.contains_key("path") {
        return Err(YappinError::validation("schema", "choose a component name or method/path; full spec is available through the CLI schema command"));
    }
    if serde_json::to_vec(args)?.len() > 131072 {
        return Err(YappinError::validation(
            "arguments",
            "maximum inline arguments are 128 KiB",
        ));
    }
    Ok(())
}
pub fn prepare(
    name: &str,
    args: &Value,
    ctx: &Context,
    cursor: Option<&std::path::Path>,
) -> Result<(Args, Context)> {
    let entry = catalog()
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| YappinError::validation("tool", "unknown tool"))?;
    validate(entry, args)?;
    let mut argv = vec!["rappin".to_owned()];
    argv.extend(entry.cli.iter().map(|s| (*s).to_owned()));
    for f in &entry.fields {
        let Some(value) = args.get(f.name) else {
            continue;
        };
        if value.is_boolean() {
            if value == true {
                argv.push(f.flag.unwrap().into());
            }
            continue;
        }
        if let Some(flag) = f.flag {
            argv.push(flag.into());
        }
        argv.push(if f.name == "body" {
            "-".into()
        } else {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        });
    }
    if matches!(name, "poll_events" | "ack_events") {
        if let Some(path) = cursor {
            argv.push("--cursor-file".into());
            argv.push(
                path.to_str()
                    .ok_or_else(|| YappinError::Config("event cursor path must be UTF-8".into()))?
                    .into(),
            );
        } else if name == "ack_events" {
            return Err(YappinError::Config(
                "start MCP with --cursor-file PATH or use the explicit events CLI to acknowledge"
                    .into(),
            ));
        }
    }
    let parsed = Args::try_parse_from(argv).map_err(|_| {
        YappinError::validation(
            "arguments",
            "invalid arguments; inspect tools/list or CLI help",
        )
    })?;
    let mut context = ctx.clone();
    context.dry |= args["dry_run"] == true;
    context.inline_body = args.get("body").cloned();
    Ok((parsed, context))
}
pub fn capabilities() -> Value {
    fn tree(command: &clap::Command) -> Value {
        json!({"name":command.get_name(),"description":command.get_about().map(|s|s.to_string()),"subcommands":command.get_subcommands().map(tree).collect::<Vec<_>>()})
    }
    json!({"version":env!("CARGO_PKG_VERSION"),"cli":tree(&Args::command()),"tools":tools(),"features":{"cli":true,"gateway":cfg!(feature="gateway"),"voice":cfg!(feature="voice"),"hot_reload":cfg!(feature="hot-reload")},"default_api_base":rappin::DEFAULT_API_BASE,"protocol_versions":["2025-06-18","2025-11-25"],"output":{"format":"json","max_bytes_scope":"compact success envelope before MCP transport encoding","continuation":"API cursor for messages/members; local offset for servers/channels"},"events":{"ack":"delivered IDs only","replay_pending":true,"mcp_cursor":"selected at process startup"}})
}
pub fn endpoint_schema(spec: &Value, path: &str, method: &str) -> Result<Value> {
    let method = method.to_ascii_lowercase();
    let item = spec["paths"]
        .get(path)
        .ok_or_else(|| YappinError::validation("path", "unknown API path template"))?;
    let mut operation = item
        .get(&method)
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| YappinError::validation("method", "unknown HTTP operation"))?;
    if let Some(parameters) = item.get("parameters") {
        operation["path_parameters"] = parameters.clone();
    }
    fn refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(o) => {
                if let Some(s) = o.get("$ref").and_then(Value::as_str) {
                    out.push(s.into());
                }
                for value in o.values() {
                    refs(value, out);
                }
            }
            Value::Array(a) => {
                for value in a {
                    refs(value, out);
                }
            }
            _ => (),
        }
    }
    let mut queue = vec![];
    refs(&operation, &mut queue);
    let mut seen = std::collections::HashSet::new();
    let mut components = json!({});
    while let Some(reference) = queue.pop() {
        if !seen.insert(reference.clone()) {
            continue;
        }
        let parts: Vec<_> = reference.split('/').collect();
        if parts.len() != 4 || parts[0] != "#" || parts[1] != "components" {
            return Err(YappinError::Config("unsupported schema reference".into()));
        }
        let value = spec
            .pointer(reference.trim_start_matches('#'))
            .ok_or_else(|| YappinError::Config("missing schema reference".into()))?;
        refs(value, &mut queue);
        components[parts[2]][parts[3]] = value.clone();
    }
    Ok(
        json!({"path":path,"method":method.to_uppercase(),"operation":operation,"components":components}),
    )
}
