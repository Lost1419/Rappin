//! Composable JSON interface to the Yapin Bot API.
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use rappin::{Client, CreateMessage, EditMessage, EventCursor, Result, YappinError};
use reqwest::Method;
use serde_json::{json, Value};
use std::{io::Read, path::PathBuf};
use uuid::Uuid;
mod agent_catalog;
mod agent_events;
mod agent_output;
#[path = "rappin_mcp.rs"]
mod mcp;
mod server_features;

#[derive(Parser)]
#[command(
    name = "rappin",
    version,
    about = "Yapin API for agents: discover, resolve, read, write, and resume events"
)]
struct Args {
    /// Stable JSON envelope. JSON is also the default output format.
    #[arg(long, global = true)]
    json: bool,
    /// Preview writes without authentication or network access.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Override YAPPER_API_BASE / config base_url.
    #[arg(long, global = true)]
    base_url: Option<String>,
    /// TOML config; default ~/.config/rappin/config.toml.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Select top-level item fields on collection reads; IDs/cursors are retained.
    #[arg(long, global = true, value_delimiter = ',')]
    fields: Vec<String>,
    #[arg(long, global = true, value_parser=clap::value_parser!(u32).range(1..=10000))]
    max_items: Option<u32>,
    /// Maximum compact JSON envelope bytes on collection reads.
    #[arg(long, global = true, value_parser=clap::value_parser!(u32).range(256..=4194304))]
    max_bytes: Option<u32>,
    /// Local list offset for servers/channels; API cursor reads use before/after.
    #[arg(long, global = true, value_parser=clap::value_parser!(u32).range(0..=1000000))]
    offset: Option<u32>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Serve agent tools over MCP stdio (newline-delimited JSON-RPC).
    Mcp {
        /// Operator-selected event state file; tool calls cannot choose filesystem paths.
        #[arg(long)]
        cursor_file: Option<PathBuf>,
    },
    /// Check configuration and, unless --offline, authentication and reachability.
    Doctor {
        #[arg(long)]
        offline: bool,
    },
    /// Print the bundled OpenAPI spec, or one named component schema.
    Schema {
        name: Option<String>,
        #[arg(long, requires = "method", conflicts_with = "name")]
        path: Option<String>,
        #[arg(long, requires = "path", conflicts_with = "name")]
        method: Option<String>,
    },
    /// Offline inventory of CLI commands, MCP tools, feature flags and contracts.
    Capabilities,
    /// Shared named operations used by both CLI and MCP.
    Tools {
        #[command(subcommand)]
        action: Tools,
    },
    /// List or fetch servers.
    Servers {
        #[command(subcommand)]
        action: Servers,
    },
    /// Discover, resolve, inspect, create, edit, or delete channels.
    Channels {
        #[command(subcommand)]
        action: Channels,
    },
    /// Read and resolve server members within bounded pages.
    Members {
        #[command(subcommand)]
        action: Members,
    },
    /// Read, send, edit, delete, pin, or react to messages.
    Messages {
        #[command(subcommand)]
        action: Messages,
    },
    /// Poll events and explicitly commit handled IDs.
    Events {
        #[command(subcommand)]
        action: Events,
    },
    /// Open a DM or send a message to a user.
    Dm {
        #[command(subcommand)]
        action: Dm,
    },
    /// One exact API call. JSON bodies come from a file or '-' for stdin.
    Request {
        method: String,
        path: String,
        #[arg(long)]
        body_file: Option<PathBuf>,
    },
    /// Play a local audio file using the optional encrypted voice bridge.
    #[cfg(feature = "voice")]
    Voice {
        server: Uuid,
        channel: Uuid,
        file: PathBuf,
        #[arg(long)]
        sdk: PathBuf,
        #[arg(long)]
        node_modules: PathBuf,
    },
}
#[derive(Subcommand)]
enum Tools {
    List,
    Call {
        name: String,
        #[arg(long)]
        arguments_file: PathBuf,
        #[arg(long)]
        cursor_file: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
enum Servers {
    List,
    Get {
        id: Uuid,
    },
    /// Live counts (test servers only).
    Counts {
        id: Uuid,
    },
    /// Invite list (Manage Server; test servers only).
    Invites {
        id: Uuid,
    },
    Roles {
        id: Uuid,
    },
    RoleCreate {
        id: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
    RoleEdit {
        id: Uuid,
        role: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
    RoleDelete {
        id: Uuid,
        role: Uuid,
    },
    EveryonePermissions {
        id: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
    CategoryPermissionsSet {
        id: Uuid,
        category: Uuid,
        target: rappin::PermissionTarget,
        #[arg(long)]
        body_file: PathBuf,
    },
    CategoryPermissionsClear {
        id: Uuid,
        category: Uuid,
        target: rappin::PermissionTarget,
    },
    ScheduledEvents {
        id: Uuid,
    },
    EventCreate {
        id: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
}
#[derive(Subcommand)]
enum Channels {
    /// Return matches and whether the visible scope was completely searched.
    Search {
        name: String,
        #[arg(long)]
        server: Option<Uuid>,
    },
    Permissions {
        id: Uuid,
    },
    PermissionsSet {
        id: Uuid,
        target: rappin::PermissionTarget,
        #[arg(long)]
        body_file: PathBuf,
    },
    PermissionsClear {
        id: Uuid,
        target: rappin::PermissionTarget,
    },
    List {
        #[arg(long)]
        server: Option<Uuid>,
    },
    Resolve {
        name: String,
        #[arg(long)]
        server: Option<Uuid>,
    },
    Get {
        id: Uuid,
    },
    Create {
        server: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
    Edit {
        id: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
    Delete {
        id: Uuid,
    },
}
#[derive(Subcommand)]
enum Members {
    /// Return candidates without claiming uniqueness after an incomplete scan.
    Search {
        server: Uuid,
        name: String,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=100))]
        max_pages: u32,
    },
    List {
        server: Uuid,
        #[arg(long,default_value_t=200,value_parser=clap::value_parser!(u32).range(1..=500))]
        limit: u32,
        #[arg(long)]
        after: Option<Uuid>,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=100))]
        max_pages: u32,
    },
    Resolve {
        server: Uuid,
        name: String,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=100))]
        max_pages: u32,
    },
    Get {
        server: Uuid,
        user: Uuid,
    },
}
#[derive(Subcommand)]
enum Messages {
    List {
        channel: Uuid,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        #[arg(long, conflicts_with = "after")]
        before: Option<DateTime<Utc>>,
        #[arg(long, conflicts_with = "before")]
        after: Option<DateTime<Utc>>,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u32).range(1..=100))]
        max_pages: u32,
    },
    Get {
        channel: Uuid,
        message: Uuid,
    },
    Send {
        channel: Uuid,
        #[arg(long, conflicts_with = "body_file", allow_hyphen_values = true)]
        content: Option<String>,
        #[arg(long)]
        body_file: Option<PathBuf>,
        #[arg(long)]
        reply_to: Option<Uuid>,
        #[arg(long, conflicts_with = "body_file")]
        split: bool,
    },
    Edit {
        channel: Uuid,
        message: Uuid,
        #[arg(long)]
        body_file: PathBuf,
    },
    Delete {
        channel: Uuid,
        message: Uuid,
    },
    Pin {
        channel: Uuid,
        message: Uuid,
    },
    Unpin {
        channel: Uuid,
        message: Uuid,
    },
    React {
        channel: Uuid,
        message: Uuid,
        emoji: String,
    },
    Unreact {
        channel: Uuid,
        message: Uuid,
        emoji: String,
    },
}
#[derive(Subcommand)]
enum Events {
    Poll {
        #[arg(long)]
        after: Option<u64>,
        #[arg(long)]
        cursor_file: Option<PathBuf>,
        #[arg(long,default_value_t=0,value_parser=clap::value_parser!(u32).range(0..=25))]
        wait: u32,
        #[arg(long,value_parser=clap::value_parser!(u32).range(0..=86400))]
        max_age: Option<u32>,
    },
    /// Save an event watermark only after processing the returned batch. Single owner per file.
    Ack {
        id: u64,
        #[arg(long)]
        cursor_file: PathBuf,
        #[arg(long)]
        batch_id: Option<String>,
        /// Operator recovery only; bypass delivery checks. Never exposed through MCP.
        #[arg(long, conflicts_with = "batch_id")]
        force: bool,
    },
}
#[derive(Subcommand)]
enum Dm {
    Open {
        user: Uuid,
    },
    Send {
        user: Uuid,
        #[arg(long, allow_hyphen_values = true)]
        content: String,
    },
}
#[derive(Default, serde::Deserialize)]
struct Config {
    token: Option<String>,
    base_url: Option<String>,
}
#[derive(Clone)]
struct Context {
    client: Option<Client>,
    dry: bool,
    base: String,
    source: &'static str,
    config_path: Option<PathBuf>,
    token: Option<String>,
    inline_body: Option<Value>,
    events: std::sync::Arc<tokio::sync::Mutex<()>>,
}
impl Context {
    fn body(&self, path: &PathBuf) -> Result<Value> {
        self.inline_body
            .clone()
            .map(Ok)
            .unwrap_or_else(|| input(path))
    }
    fn client(&self) -> Result<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| YappinError::Config("set YAPPER_BOT_TOKEN or config token".into()))
    }
    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains('#') {
            return Err(YappinError::validation(
                "path",
                "use a relative API path starting with one slash",
            ));
        }
        if self.dry && method != Method::GET && method != Method::HEAD {
            return Ok(json!({"dry_run":true,"method":method.as_str(),"path":path,"body":body}));
        }
        self.client()?
            .request_json(method, path, body.as_ref())
            .await
    }
}
fn input(path: &PathBuf) -> Result<Value> {
    let bytes = if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().take(131073).read_to_end(&mut bytes)?;
        bytes
    } else {
        std::fs::read(path)?
    };
    if bytes.len() > 131072 {
        return Err(YappinError::validation(
            "body_file",
            "maximum JSON body is 128 KiB",
        ));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn config(args: &Args) -> Result<Context> {
    let path = args.config.clone().or_else(|| {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/rappin/config.toml"))
    });
    let config = match &path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => toml::from_str::<Config>(&s)
                .map_err(|_| YappinError::Config("invalid TOML config".into()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && args.config.is_none() => {
                Config::default()
            }
            Err(e) => return Err(e.into()),
        },
        None => Config::default(),
    };
    let env_token = std::env::var("YAPPER_BOT_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let source = if env_token.is_some() {
        "env"
    } else if config.token.is_some() {
        "config"
    } else {
        "missing"
    };
    let token = env_token.or(config.token).filter(|s| !s.trim().is_empty());
    let base = args
        .base_url
        .clone()
        .or_else(|| std::env::var("YAPPER_API_BASE").ok())
        .or(config.base_url)
        .unwrap_or_else(|| rappin::DEFAULT_API_BASE.into());
    let url =
        url::Url::parse(&base).map_err(|_| YappinError::Config("invalid API base URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(YappinError::Config(
            "base URL must be HTTP(S), without credentials, query, or fragment".into(),
        ));
    }
    let client = token
        .as_ref()
        .map(|t| Client::builder(t).base_url(&base).build());
    Ok(Context {
        client,
        dry: args.dry_run,
        base,
        source,
        config_path: path,
        token,
        inline_body: None,
        events: Default::default(),
    })
}
async fn run(args: Args, ctx: &Context) -> Result<Value> {
    let key = agent_output::collection(&args.command);
    let limits = agent_output::Limits {
        fields: args.fields.clone(),
        max_items: args.max_items.map(|n| n as usize),
        max_bytes: args.max_bytes.map(|n| n as usize),
        offset: args.offset.map(|n| n as usize),
    };
    agent_output::validate(&limits, key)?;
    let result = run_inner(args, ctx).await?;
    match key {
        Some(key) => agent_output::apply(redact(result, ctx.token.as_deref()), key, &limits),
        None => Ok(result),
    }
}
async fn run_inner(args: Args, ctx: &Context) -> Result<Value> {
    match args.command {
        Command::Mcp { .. } => Err(YappinError::Config("start MCP with rappin mcp".into())),
        Command::Doctor { offline } => {
            let mut v = json!({"version":env!("CARGO_PKG_VERSION"),"base_url":ctx.base,"auth_source":ctx.source,"token_available":ctx.token.is_some(),"config_path":ctx.config_path,"offline":offline,"reachable":null,"authenticated":null,"setup":if ctx.token.is_none(){Some("set YAPPER_BOT_TOKEN")}else{None}});
            if !offline && ctx.client.is_some() {
                match ctx.client()?.get_me().await {
                    Ok(me) => {
                        v["reachable"] = true.into();
                        v["authenticated"] = true.into();
                        v["bot_id"] = me.id.to_string().into();
                    }
                    Err(e) => {
                        v["reachable"] = e.status_code().is_some().into();
                        v["authenticated"] = false.into();
                        v["error"] = e.to_json();
                    }
                }
            }
            if let Some(client) = &ctx.client {
                v["active_base_url"] = client.active_base_url().into();
                v["last_request"]=client.last_request().map(|r|json!({"method":r.method,"route":r.route,"status":r.status,"duration_ms":r.duration.as_secs_f64()*1000.0,"rate_limit_wait_ms":r.rate_limit_wait.as_secs_f64()*1000.0,"attempts":r.attempts})).unwrap_or(Value::Null);
            }
            Ok(v)
        }
        Command::Capabilities => Ok(agent_catalog::capabilities()),
        Command::Tools { action } => match action {
            Tools::List => Ok(json!({"tools":agent_catalog::tools()})),
            Tools::Call {
                name,
                arguments_file,
                cursor_file,
            } => {
                let (args, context) = agent_catalog::prepare(
                    &name,
                    &input(&arguments_file)?,
                    ctx,
                    cursor_file.as_deref(),
                )?;
                Box::pin(run(args, &context)).await
            }
        },
        Command::Schema { name, path, method } => {
            let mut spec: Value = serde_json::from_str(include_str!("../../docs/openapi.json"))?;
            let guide: Value = serde_json::from_str(include_str!("../../docs/guide-api.json"))?;
            for (path, item) in guide["paths"].as_object().expect("bundled guide paths") {
                for (method, operation) in item.as_object().expect("bundled guide operation") {
                    spec["paths"][path][method] = operation.clone();
                }
            }
            if let (Some(path), Some(method)) = (path, method) {
                return agent_catalog::endpoint_schema(&spec, &path, &method);
            }
            match name {
                Some(n) => spec["components"]["schemas"]
                    .get(&n)
                    .cloned()
                    .ok_or_else(|| YappinError::validation("schema", "unknown component schema")),
                None => Ok(spec),
            }
        }
        Command::Servers { action } => match action {
            Servers::List => ctx.request(Method::GET, "/servers", None).await,
            Servers::Get { id } => {
                ctx.request(Method::GET, &format!("/servers/{id}"), None)
                    .await
            }
            action => server_features::server_action(action, ctx).await,
        },
        Command::Channels { action } => match action {
            Channels::Search { name, server } => {
                let result = ctx.client()?.search_channels(server, &name).await?;
                let status = result.status();
                let mut value = serde_json::to_value(result)?;
                value["status"] = serde_json::to_value(status)?;
                Ok(value)
            }
            Channels::Permissions { id } => {
                ctx.request(Method::GET, &format!("/channels/{id}/permissions"), None)
                    .await
            }
            Channels::PermissionsSet {
                id,
                target,
                body_file,
            } => {
                let body: rappin::PermissionOverride =
                    serde_json::from_value(ctx.body(&body_file)?)?;
                ctx.request(
                    Method::PUT,
                    &format!("/channels/{id}/permissions/{target}"),
                    Some(serde_json::to_value(body)?),
                )
                .await
            }
            Channels::PermissionsClear { id, target } => {
                ctx.request(
                    Method::DELETE,
                    &format!("/channels/{id}/permissions/{target}"),
                    None,
                )
                .await
            }
            Channels::List { server } => {
                ctx.request(
                    Method::GET,
                    &server.map_or("/channels".into(), |id| format!("/servers/{id}/channels")),
                    None,
                )
                .await
            }
            Channels::Resolve { name, server } => Ok(serde_json::to_value(
                ctx.client()?.resolve_channel(server, &name).await?,
            )?),
            Channels::Get { id } => {
                ctx.request(Method::GET, &format!("/channels/{id}"), None)
                    .await
            }
            Channels::Create { server, body_file } => {
                let body: rappin::CreateChannelRequest =
                    serde_json::from_value(ctx.body(&body_file)?)?;
                ctx.request(
                    Method::POST,
                    &format!("/servers/{server}/channels"),
                    Some(serde_json::to_value(body)?),
                )
                .await
            }
            Channels::Edit { id, body_file } => {
                let body: rappin::EditChannelRequest =
                    serde_json::from_value(ctx.body(&body_file)?)?;
                ctx.request(
                    Method::PATCH,
                    &format!("/channels/{id}"),
                    Some(serde_json::to_value(body)?),
                )
                .await
            }
            Channels::Delete { id } => {
                ctx.request(Method::DELETE, &format!("/channels/{id}"), None)
                    .await
            }
        },
        Command::Members { action } => match action {
            Members::Search {
                server,
                name,
                max_pages,
            } => {
                let result = ctx
                    .client()?
                    .search_members(server, &name, max_pages as usize)
                    .await?;
                let status = result.status();
                let mut value = serde_json::to_value(result)?;
                value["status"] = serde_json::to_value(status)?;
                Ok(value)
            }
            Members::Get { server, user } => {
                ctx.request(
                    Method::GET,
                    &format!("/servers/{server}/members/{user}"),
                    None,
                )
                .await
            }
            Members::Resolve {
                server,
                name,
                max_pages,
            } => Ok(serde_json::to_value(
                ctx.client()?
                    .resolve_member(server, &name, max_pages as usize)
                    .await?,
            )?),
            Members::List {
                server,
                limit,
                after,
                max_pages,
            } => {
                let mut cursor = after;
                let mut all = vec![];
                let mut pages = 0;
                let mut more = false;
                for _ in 0..max_pages {
                    let page = ctx
                        .client()?
                        .get_members(server, Some(limit), cursor)
                        .await?;
                    pages += 1;
                    more = page.len() == limit as usize;
                    let next = page.last().map(|m| m.user.id);
                    all.extend(page);
                    if !more || next == cursor {
                        more = false;
                        break;
                    }
                    cursor = next;
                }
                Ok(
                    json!({"members":all,"next_cursor":if more{cursor}else{None},"page_count":pages,"may_have_more":more}),
                )
            }
        },
        Command::Messages { action } => match action {
            Messages::List {
                channel,
                limit,
                before,
                after,
                max_pages,
            } => {
                let mut cursor = before;
                let mut forward = after;
                let mut all = vec![];
                let mut authors = std::collections::BTreeMap::new();
                let mut seen = std::collections::HashSet::new();
                let mut pages = 0;
                let mut more = false;
                let mut next = None;
                for _ in 0..max_pages {
                    let page = ctx
                        .client()?
                        .get_messages(channel, Some(limit), cursor, forward)
                        .await?;
                    pages += 1;
                    more = page.messages.len() == limit as usize;
                    next = if after.is_some() {
                        page.messages.iter().map(|m| m.created_at).max()
                    } else {
                        page.messages.iter().map(|m| m.created_at).min()
                    };
                    for m in page.messages {
                        if seen.insert(m.id) {
                            all.push(m);
                        }
                    }
                    for user in page.authors {
                        authors.insert(user.id, user);
                    }
                    if !more
                        || next.is_none()
                        || (after.is_some() && next == forward)
                        || (after.is_none() && next == cursor)
                    {
                        more = false;
                        break;
                    }
                    if after.is_some() {
                        forward = next;
                    } else {
                        cursor = next;
                    }
                }
                all.sort_by_key(|m| m.created_at);
                Ok(
                    json!({"messages":all,"authors":authors.into_values().collect::<Vec<_>>(),"next_cursor":if more{next}else{None},"direction":if after.is_some(){"after"}else{"before"},"page_count":pages,"may_have_more":more}),
                )
            }
            Messages::Get { channel, message } => {
                ctx.request(
                    Method::GET,
                    &format!("/channels/{channel}/messages/{message}"),
                    None,
                )
                .await
            }
            Messages::Send {
                channel,
                content,
                body_file,
                reply_to,
                split,
            } => {
                let mut message = match body_file {
                    Some(p) => serde_json::from_value::<CreateMessage>(ctx.body(&p)?)?,
                    None => CreateMessage::new(content.ok_or_else(|| {
                        YappinError::validation("content", "pass --content or --body-file")
                    })?),
                };
                if reply_to.is_some() {
                    message.reply_to = reply_to;
                }
                let messages = if split {
                    rappin::split_message(message.content.as_deref().unwrap_or(""))
                        .into_iter()
                        .map(|s| CreateMessage::new(s).with_reply_to_opt(reply_to))
                        .collect()
                } else {
                    vec![message]
                };
                if messages.is_empty() {
                    return Err(YappinError::validation(
                        "content",
                        "message cannot be empty",
                    ));
                }
                for message in &messages {
                    message.validate()?;
                }
                let mut sent = vec![];
                for (step, message) in messages.into_iter().enumerate() {
                    match ctx
                        .request(
                            Method::POST,
                            &format!("/channels/{channel}/messages"),
                            Some(serde_json::to_value(message)?),
                        )
                        .await
                    {
                        Ok(value) => sent.push(value),
                        Err(source) => {
                            if !split {
                                return Err(source);
                            }
                            return Err(YappinError::PartialWrite {
                                completed: sent,
                                failed_step: step + 1,
                                source: Box::new(source),
                            });
                        }
                    }
                }
                Ok(json!({"messages":sent}))
            }
            Messages::Edit {
                channel,
                message,
                body_file,
            } => {
                let body: EditMessage = serde_json::from_value(ctx.body(&body_file)?)?;
                body.validate()?;
                ctx.request(
                    Method::PATCH,
                    &format!("/channels/{channel}/messages/{message}"),
                    Some(serde_json::to_value(body)?),
                )
                .await
            }
            Messages::Delete { channel, message } => {
                ctx.request(
                    Method::DELETE,
                    &format!("/channels/{channel}/messages/{message}"),
                    None,
                )
                .await
            }
            Messages::Pin { channel, message } => {
                ctx.request(
                    Method::PUT,
                    &format!("/channels/{channel}/messages/{message}/pin"),
                    None,
                )
                .await
            }
            Messages::Unpin { channel, message } => {
                ctx.request(
                    Method::DELETE,
                    &format!("/channels/{channel}/messages/{message}/pin"),
                    None,
                )
                .await
            }
            Messages::React {
                channel,
                message,
                emoji,
            } => {
                let e = url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect::<String>();
                ctx.request(
                    Method::PUT,
                    &format!("/channels/{channel}/messages/{message}/reactions/{e}"),
                    None,
                )
                .await
            }
            Messages::Unreact {
                channel,
                message,
                emoji,
            } => {
                let e = url::form_urlencoded::byte_serialize(emoji.as_bytes()).collect::<String>();
                ctx.request(
                    Method::DELETE,
                    &format!("/channels/{channel}/messages/{message}/reactions/{e}"),
                    None,
                )
                .await
            }
        },
        Command::Events { action } => match action {
            Events::Poll {
                after,
                cursor_file,
                wait,
                max_age,
            } => agent_events::poll(ctx, after, cursor_file.as_deref(), wait, max_age).await,
            Events::Ack {
                id,
                cursor_file,
                batch_id,
                force,
            } => agent_events::acknowledge(ctx, id, &cursor_file, batch_id.as_deref(), force).await,
        },
        Command::Dm { action } => match action {
            Dm::Open { user } => {
                ctx.request(Method::POST, &format!("/users/{user}/dm"), None)
                    .await
            }
            Dm::Send { user, content } => {
                let message = CreateMessage::new(content);
                message.validate()?;
                if ctx.dry {
                    return Ok(
                        json!({"dry_run":true,"user_id":user,"steps":["open DM","send message"],"body":message}),
                    );
                }
                let ch = ctx.client()?.create_dm(user).await?;
                Ok(serde_json::to_value(
                    ctx.client()?.send_message(ch, &message).await?,
                )?)
            }
        },
        Command::Request {
            method,
            path,
            body_file,
        } => {
            let method = Method::from_bytes(method.to_uppercase().as_bytes())
                .map_err(|_| YappinError::validation("method", "invalid HTTP method"))?;
            ctx.request(
                method,
                &path,
                body_file.as_ref().map(|path| ctx.body(path)).transpose()?,
            )
            .await
        }
        #[cfg(feature = "voice")]
        Command::Voice {
            server,
            channel,
            file,
            sdk,
            node_modules,
        } => {
            if ctx.dry {
                return Ok(json!({"dry_run":true,"server":server,"channel":channel,"file":file}));
            }
            ctx.client()?
                .play_audio(
                    server,
                    channel,
                    file,
                    &rappin::voice::VoiceOptions::new(sdk, node_modules),
                )
                .await?;
            Ok(json!({"played":true}))
        }
    }
}
fn read_only(command: &Command) -> bool {
    match command {
        Command::Doctor { .. }
        | Command::Capabilities
        | Command::Schema { .. }
        | Command::Servers {
            action:
                Servers::List
                | Servers::Get { .. }
                | Servers::Counts { .. }
                | Servers::Invites { .. }
                | Servers::Roles { .. }
                | Servers::ScheduledEvents { .. },
        }
        | Command::Members { .. }
        | Command::Channels {
            action:
                Channels::List { .. }
                | Channels::Get { .. }
                | Channels::Resolve { .. }
                | Channels::Search { .. }
                | Channels::Permissions { .. },
        }
        | Command::Messages {
            action: Messages::List { .. } | Messages::Get { .. },
        }
        | Command::Events {
            action: Events::Poll { .. },
        } => true,
        Command::Request { method, .. } => {
            method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD")
        }
        Command::Tools {
            action: Tools::List,
        } => true,
        Command::Tools {
            action: Tools::Call { name, .. },
        } => !agent_catalog::is_write(name),
        _ => false,
    }
}
fn error_json(error: &YappinError, read_only: bool) -> Value {
    let mut value = error.to_json();
    if read_only && value["retry_safety"] == "unknown_write_outcome" {
        value["retry_safety"] = "safe_read".into();
        value["outcome"] = "not_a_write".into();
    }
    value
}
// Keep optional reply construction local to the CLI.
trait OptionalReply {
    fn with_reply_to_opt(self, id: Option<Uuid>) -> Self;
}
impl OptionalReply for CreateMessage {
    fn with_reply_to_opt(mut self, id: Option<Uuid>) -> Self {
        self.reply_to = id;
        self
    }
}
#[tokio::main]
async fn main() {
    let parsed = Args::try_parse();
    let args = match parsed {
        Ok(a) => a,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{e}");
                return;
            }
            println!(
                "{}",
                json!({"ok":false,"error":{"kind":"arguments","message":e.to_string()}})
            );
            std::process::exit(2);
        }
    };
    let context = config(&args);
    if let Command::Mcp { cursor_file } = &args.command {
        match context {
            Ok(ctx) => {
                if let Err(e) = mcp::serve(&ctx, cursor_file.as_deref()).await {
                    eprintln!("MCP server stopped: {e}");
                    std::process::exit(1);
                }
            }
            Err(_) => {
                eprintln!("could not load configuration");
                std::process::exit(2);
            }
        }
        return;
    }
    let read_only = read_only(&args.command);
    let result = match &context {
        Ok(ctx) => run(args, ctx).await,
        Err(e) => Err(YappinError::Config(e.to_string())),
    };
    match result {
        Ok(data) => println!(
            "{}",
            redact(
                json!({"ok":true,"data":data}),
                context.as_ref().ok().and_then(|ctx| ctx.token.as_deref())
            )
        ),
        Err(e) => {
            let mut value = json!({"ok":false,"error":error_json(&e,read_only)});
            value = redact(
                value,
                context.as_ref().ok().and_then(|ctx| ctx.token.as_deref()),
            );
            println!("{value}");
            std::process::exit(
                if matches!(e, YappinError::Validation { .. } | YappinError::Config(_)) {
                    2
                } else {
                    1
                },
            );
        }
    }
}

fn redact(value: Value, token: Option<&str>) -> Value {
    rappin::security::redact_json(value, &token.into_iter().collect::<Vec<_>>())
}
