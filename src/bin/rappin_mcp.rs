//! MCP adapter intentionally reuses the CLI command parser and execution contract.
use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: Vec<&str>,
    write: bool,
) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":!write,"destructiveHint":false,"openWorldHint":true}})
}
fn tools() -> Vec<Value> {
    vec![
        tool(
            "list_servers",
            "List servers the bot can see",
            json!({}),
            vec![],
            false,
        ),
        tool(
            "list_channels",
            "List visible channels, optionally in one server",
            json!({"server":{"type":"string","format":"uuid"}}),
            vec![],
            false,
        ),
        tool(
            "resolve_channel",
            "Resolve a channel name; ambiguity returns an error",
            json!({"name":{"type":"string"},"server":{"type":"string","format":"uuid"}}),
            vec!["name"],
            false,
        ),
        tool(
            "read_messages",
            "Read bounded history. Cursors are RFC3339 timestamps",
            json!({"channel":{"type":"string","format":"uuid"},"limit":{"type":"integer","minimum":1,"maximum":100},"before":{"type":"string","format":"date-time"},"after":{"type":"string","format":"date-time"}}),
            vec!["channel"],
            false,
        ),
        tool(
            "send_message",
            "Send a message only when requested; use dry_run to preview",
            json!({"channel":{"type":"string","format":"uuid"},"content":{"type":"string","maxLength":2000},"reply_to":{"type":"string","format":"uuid"},"dry_run":{"type":"boolean"}}),
            vec!["channel", "content"],
            true,
        ),
        tool(
            "poll_events",
            "Read events without acknowledging them; after=0 starts from now",
            json!({"after":{"type":"integer","minimum":0},"wait":{"type":"integer","minimum":0,"maximum":25},"cursor_file":{"type":"string"}}),
            vec![],
            false,
        ),
        tool(
            "ack_events",
            "Persist a watermark after handling every delivered event up to it",
            json!({"id":{"type":"integer","minimum":0},"cursor_file":{"type":"string"},"dry_run":{"type":"boolean"}}),
            vec!["id", "cursor_file"],
            true,
        ),
    ]
}
fn argv(name: &str, a: &Value) -> Result<Vec<String>> {
    let string = |key: &str| {
        a[key]
            .as_str()
            .map(String::from)
            .ok_or_else(|| YappinError::validation(key, "required string"))
    };
    let mut args: Vec<String> = match name {
        "list_servers" => vec!["servers".into(), "list".into()],
        "list_channels" => vec!["channels".into(), "list".into()],
        "resolve_channel" => vec!["channels".into(), "resolve".into(), string("name")?],
        "read_messages" => vec!["messages".into(), "list".into(), string("channel")?],
        "send_message" => vec![
            "messages".into(),
            "send".into(),
            string("channel")?,
            "--content".into(),
            string("content")?,
        ],
        "poll_events" => vec!["events".into(), "poll".into()],
        "ack_events" => vec![
            "events".into(),
            "ack".into(),
            a["id"]
                .as_u64()
                .ok_or_else(|| YappinError::validation("id", "required unsigned integer"))?
                .to_string(),
        ],
        _ => return Err(YappinError::validation("tool", "unknown tool")),
    };
    for key in match name {
        "list_channels" | "resolve_channel" => vec!["server"],
        "read_messages" => vec!["limit", "before", "after"],
        "send_message" => vec!["reply_to"],
        "poll_events" => vec!["after", "wait", "cursor_file"],
        "ack_events" => vec!["cursor_file"],
        _ => vec![],
    } {
        if let Some(value) = a.get(key) {
            let value = value
                .as_str()
                .map(String::from)
                .or_else(|| value.as_u64().map(|v| v.to_string()))
                .ok_or_else(|| {
                    YappinError::validation(key, "expected string or unsigned integer")
                })?;
            args.push(format!("--{}", key.replace('_', "-")));
            args.push(value);
        }
    }
    if name == "ack_events" && a["cursor_file"].as_str().is_none() {
        return Err(YappinError::validation("cursor_file", "required string"));
    }
    Ok(args)
}
pub async fn serve(ctx: &Context) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                let response = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}});
                stdout.write_all(format!("{response}\n").as_bytes()).await?;
                continue;
            }
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let result: std::result::Result<Value, (i32, &str)> = match request["method"].as_str() {
            Some("initialize") => Ok(
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"rappin","version":env!("CARGO_PKG_VERSION")}}),
            ),
            Some("ping") => Ok(json!({})),
            Some("tools/list") => Ok(json!({"tools":tools()})),
            Some("tools/call") => {
                let a = &request["params"]["arguments"];
                let name = request["params"]["name"].as_str().unwrap_or("");
                let output: Result<Value> = async {
                    let mut args = vec!["rappin".to_owned()];
                    args.extend(argv(name, a)?);
                    let parsed = Args::try_parse_from(args).map_err(|_| {
                        YappinError::validation(
                            "arguments",
                            "invalid tool arguments; inspect tools/list",
                        )
                    })?;
                    let mut context = ctx.clone();
                    context.dry |= a["dry_run"] == true;
                    Box::pin(run(parsed, &context)).await
                }
                .await;
                let (failed, data) = match output {
                    Ok(data) => (false, json!({"ok":true,"data":data})),
                    Err(e) => (true, json!({"ok":false,"error":e.to_json()})),
                };
                let data = redact(data, ctx.token.as_deref());
                Ok(json!({"isError":failed,"content":[{"type":"text","text":data.to_string()}]}))
            }
            _ => Err((-32601, "Method not found")),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
            }
        };
        stdout.write_all(format!("{response}\n").as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}
