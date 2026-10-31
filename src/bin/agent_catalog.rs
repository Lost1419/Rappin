use super::*;
use clap::CommandFactory;
pub use rappin::capabilities::{endpoint_schema, tools};
pub fn is_write(name: &str) -> bool {
    rappin::capabilities::capability(name).is_some_and(|c| c.write)
}
pub fn prepare(
    name: &str,
    args: &Value,
    ctx: &Context,
    cursor: Option<&std::path::Path>,
) -> Result<(Args, Context)> {
    let entry = rappin::capabilities::catalogue()
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| YappinError::validation("tool", "unknown tool"))?;
    entry.validate(args)?;
    let mut argv = vec!["rappin".to_owned()];
    argv.extend(entry.cli.iter().map(|s| (*s).to_owned()));
    for f in &entry.arguments {
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
