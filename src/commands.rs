//! Local command parsing and policy. Yapin's advertised Command remains only a name and description.
use crate::{cache::Freshness, MessageContext, Result, YappinError};
use std::{future::Future, marker::PhantomData, pin::Pin, str::FromStr, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("unterminated quoted argument or escape")]
    Quoting,
    #[error("missing argument: {0}")]
    Missing(String),
    #[error("invalid {name}: {reason}")]
    Invalid { name: String, reason: String },
    #[error("unexpected extra arguments")]
    Extra,
    #[error("command denied: {0}")]
    Denied(String),
    #[error("invalid command definition: {0}")]
    Definition(String),
}

#[derive(Debug, Clone)]
pub struct Token {
    pub value: String,
    pub start: usize,
    pub end: usize,
}
/// Shell-like grouping only: no expansion or execution. Quotes and backslash escapes are respected.
pub fn tokenize(input: &str) -> std::result::Result<Vec<Token>, CommandError> {
    let mut chars = input.char_indices().peekable();
    let mut out = vec![];
    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let mut value = String::new();
        let mut quote = None;
        let mut end = start;
        while let Some(&(i, ch)) = chars.peek() {
            if quote.is_none() && ch.is_whitespace() {
                break;
            }
            chars.next();
            end = i + ch.len_utf8();
            if ch == '\\' {
                let Some((i, next)) = chars.next() else {
                    return Err(CommandError::Quoting);
                };
                value.push(next);
                end = i + next.len_utf8();
            } else if let Some(close) = quote {
                if ch == close {
                    quote = None;
                } else {
                    value.push(ch);
                }
            } else if ch == '\'' || ch == '"' {
                quote = Some(ch);
            } else {
                value.push(ch);
            }
        }
        if quote.is_some() {
            return Err(CommandError::Quoting);
        }
        out.push(Token { value, start, end });
    }
    Ok(out)
}

/// Quote-aware argument cursor. Parsing performs no network requests.
pub struct Arguments {
    tokens: std::vec::IntoIter<Token>,
}
impl Arguments {
    pub fn new(input: &str) -> std::result::Result<Self, CommandError> {
        Ok(Self {
            tokens: tokenize(input)?.into_iter(),
        })
    }
    pub fn required<T: FromStr>(&mut self, name: &str) -> std::result::Result<T, CommandError>
    where
        T::Err: std::fmt::Display,
    {
        self.optional(name)?
            .ok_or_else(|| CommandError::Missing(name.into()))
    }
    pub fn optional<T: FromStr>(
        &mut self,
        name: &str,
    ) -> std::result::Result<Option<T>, CommandError>
    where
        T::Err: std::fmt::Display,
    {
        self.tokens
            .next()
            .map(|token| {
                token
                    .value
                    .parse()
                    .map_err(|e: T::Err| CommandError::Invalid {
                        name: name.into(),
                        reason: e.to_string(),
                    })
            })
            .transpose()
    }
    pub fn rest(&mut self) -> String {
        self.tokens
            .by_ref()
            .map(|t| t.value)
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn finish(&mut self) -> std::result::Result<(), CommandError> {
        if self.tokens.next().is_some() {
            Err(CommandError::Extra)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArgumentInfo {
    pub name: String,
    pub description: String,
    pub required: bool,
}
impl ArgumentInfo {
    pub fn required(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            required: true,
        }
    }
    pub fn optional(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            required: false,
        }
    }
}
/// Parse a command's Rust argument type; `describe` supplies manually maintained help metadata.
pub trait FromArgs: Send + 'static {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError>
    where
        Self: Sized;
    fn describe() -> Vec<ArgumentInfo> {
        vec![]
    }
}
impl FromArgs for () {
    fn parse(_: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(())
    }
}

/// A human duration such as 10m, 1h30m, 2d. Bare values mean seconds; no floating point or overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HumanDuration(pub Duration);
impl FromStr for HumanDuration {
    type Err = CommandError;
    fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
        let invalid = || CommandError::Invalid {
            name: "duration".into(),
            reason: "use a positive duration such as 10m or 1h30m".into(),
        };
        let mut total = 0u64;
        let mut number = String::new();
        for ch in input.chars() {
            if ch.is_ascii_digit() {
                number.push(ch);
                continue;
            }
            let multiplier = match ch {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                'w' => 604800,
                _ => return Err(invalid()),
            };
            let n = number.parse::<u64>().map_err(|_| invalid())?;
            number.clear();
            total = total
                .checked_add(n.checked_mul(multiplier).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
        }
        if !number.is_empty() {
            total = total
                .checked_add(number.parse::<u64>().map_err(|_| invalid())?)
                .ok_or_else(invalid)?;
        }
        if total == 0 {
            return Err(invalid());
        }
        Ok(Self(Duration::from_secs(total)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedU32<const MIN: u32, const MAX: u32>(pub u32);
impl<const MIN: u32, const MAX: u32> FromStr for BoundedU32<MIN, MAX> {
    type Err = CommandError;
    fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
        let value = input
            .parse::<u32>()
            .ok()
            .filter(|n| MIN <= MAX && *n >= MIN && *n <= MAX)
            .ok_or_else(|| CommandError::Invalid {
                name: "number".into(),
                reason: format!("requires {MIN} through {MAX}"),
            })?;
        Ok(Self(value))
    }
}

/// Pure ID parsing; fetching the corresponding resource remains an explicit async operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserId(pub Uuid);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelId(pub Uuid);
macro_rules! id_parser {
    ($ty:ident,$prefix:literal) => {
        impl FromStr for $ty {
            type Err = uuid::Error;
            fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
                let value = input
                    .strip_prefix($prefix)
                    .and_then(|s| s.strip_suffix('>'))
                    .unwrap_or(input);
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}
id_parser!(UserId, "<@");
id_parser!(ChannelId, "<#");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permission(String);
impl Permission {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn manage_messages() -> Self {
        Self::new("manage_messages")
    }
    pub fn manage_server() -> Self {
        Self::new("manage_server")
    }
    pub fn kick_members() -> Self {
        Self::new("kick_members")
    }
    pub fn ban_members() -> Self {
        Self::new("ban_members")
    }
    pub fn timeout_members() -> Self {
        Self::new("timeout_members")
    }
}
pub type CheckFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
/// An async authorization/policy check run before a typed handler.
pub trait CommandCheck: Send + Sync + 'static {
    fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a>;
}

pub struct CheckFn<F>(F);
/// Adapt an async closure into a reusable check. The closure receives a cloned context.
pub fn check_fn<F, Fut>(check: F) -> CheckFn<F>
where
    F: Fn(MessageContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    CheckFn(check)
}
impl<F, Fut> CommandCheck for CheckFn<F>
where
    F: Fn(MessageContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a> {
        Box::pin((self.0)(ctx.clone()))
    }
}
#[derive(Debug, Clone)]
pub enum Check {
    Owner,
    ServerOnly,
    Roles {
        ids: Vec<Uuid>,
        freshness: Freshness,
    },
    Permissions {
        permissions: Vec<Permission>,
        freshness: Freshness,
    },
}
impl Check {
    pub fn owner_only() -> Self {
        Self::Owner
    }
    pub fn server_only() -> Self {
        Self::ServerOnly
    }
    pub fn roles(ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self::Roles {
            ids: ids.into_iter().collect(),
            freshness: Freshness::Refresh,
        }
    }
    pub fn permissions(permissions: impl IntoIterator<Item = Permission>) -> Self {
        Self::Permissions {
            permissions: permissions.into_iter().collect(),
            freshness: Freshness::Refresh,
        }
    }
    /// Cached permissions are opt-in; Refresh is the default for authorization checks.
    pub fn freshness(mut self, policy: Freshness) -> Self {
        match &mut self {
            Self::Roles { freshness, .. } | Self::Permissions { freshness, .. } => {
                *freshness = policy
            }
            _ => (),
        };
        self
    }
}
impl CommandCheck for Check {
    fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a> {
        Box::pin(async move {
            let deny = |message: &str| YappinError::from(CommandError::Denied(message.into()));
            match self {
                Self::Owner => {
                    let me = match ctx.cache().get_me().await {
                        Some(me) => me,
                        None => ctx.client.get_me().await?,
                    };
                    if me.bot.as_ref().map(|b| b.owner_id) != Some(ctx.author_id()) {
                        return Err(deny("only the bot owner may run this command"));
                    }
                }
                Self::ServerOnly => {
                    if ctx.server_id().is_none() {
                        return Err(deny("this command requires a server"));
                    }
                }
                Self::Roles { ids, freshness } => {
                    let server = ctx
                        .server_id()
                        .ok_or_else(|| deny("this command requires a server"))?;
                    let member = ctx
                        .client
                        .fetch_member(server, ctx.author_id(), *freshness)
                        .await?;
                    if !ids.iter().any(|id| member.member.roles.contains(id)) {
                        return Err(deny("a required role is missing"));
                    }
                }
                Self::Permissions {
                    permissions,
                    freshness,
                } => {
                    let server = ctx
                        .server_id()
                        .ok_or_else(|| deny("this command requires a server"))?;
                    if ctx.client.get_server(server).await?.owner_id == ctx.author_id() {
                        return Ok(());
                    }
                    let member = ctx
                        .client
                        .fetch_member(server, ctx.author_id(), *freshness)
                        .await?;
                    let roles = ctx.client.fetch_roles(server, *freshness).await?;
                    for permission in permissions {
                        let allowed = roles
                            .iter()
                            .filter(|role| member.member.roles.contains(&role.id))
                            .any(|role| match permission.as_str() {
                                "manage_server" => role.manage_server == Some(true),
                                "manage_messages" => role.manage_messages == Some(true),
                                key => {
                                    role.perms
                                        .as_ref()
                                        .and_then(|p| p.get(key))
                                        .and_then(|p| p.as_bool())
                                        == Some(true)
                                }
                            });
                        if !allowed {
                            return Err(deny(&format!(
                                "missing permission {}",
                                permission.as_str()
                            )));
                        }
                    }
                }
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub arguments: Vec<ArgumentInfo>,
}
impl CommandInfo {
    pub fn usage(&self) -> String {
        let args = self
            .arguments
            .iter()
            .map(|arg| {
                if arg.required {
                    format!("<{}>", arg.name)
                } else {
                    format!("[{}]", arg.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "/{}{}{}",
            self.name,
            if args.is_empty() { "" } else { " " },
            args
        )
    }
    pub fn help(&self) -> String {
        let mut text = format!("{} — {}", self.usage(), self.description);
        if !self.aliases.is_empty() {
            text.push_str(&format!("\nAliases: {}", self.aliases.join(", ")));
        }
        for arg in &self.arguments {
            text.push_str(&format!("\n{}: {}", arg.name, arg.description));
        }
        text
    }
}
/// Typed parser, local aliases, help metadata, and checks for a command handler.
pub struct CommandSpec<A: FromArgs> {
    pub(crate) info: CommandInfo,
    checks: Vec<Arc<dyn CommandCheck>>,
    marker: PhantomData<fn() -> A>,
}
impl<A: FromArgs> CommandSpec<A> {
    /// Space-separated names define a local subcommand path, advertised as one root command.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            info: CommandInfo {
                name: name.into().to_lowercase(),
                description: description.into(),
                aliases: vec![],
                arguments: A::describe(),
            },
            checks: vec![],
            marker: PhantomData,
        }
    }
    pub fn alias(mut self, name: impl Into<String>) -> Self {
        self.info.aliases.push(name.into().to_lowercase());
        self
    }
    pub fn check(mut self, check: impl CommandCheck) -> Self {
        self.checks.push(Arc::new(check));
        self
    }
    pub fn info(&self) -> &CommandInfo {
        &self.info
    }
    pub fn bind<F, Fut>(self, handler: F) -> RegisteredCommand
    where
        F: Fn(MessageContext, A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let checks = Arc::new(self.checks);
        let handler = Arc::new(handler);
        RegisteredCommand {
            info: self.info,
            handler: Arc::new(move |ctx| {
                let checks = checks.clone();
                let handler = handler.clone();
                Box::pin(async move {
                    let mut args = Arguments::new(&ctx.args)?;
                    let parsed = A::parse(&mut args)?;
                    args.finish()?;
                    for check in checks.iter() {
                        check.check(&ctx).await?;
                    }
                    handler(ctx, parsed).await
                })
            }),
        }
    }
}
#[derive(Clone)]
/// Erased typed handler and metadata, suitable for returning from a Cog.
pub struct RegisteredCommand {
    pub info: CommandInfo,
    pub(crate) handler: crate::bot::CommandHandlerFn,
}
