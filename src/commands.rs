//! Local command parsing and policy. Yapin's advertised Command remains only a name and description.
use crate::{cache::Freshness, MessageContext, Result, YappinError};
use std::{future::Future, marker::PhantomData, pin::Pin, str::FromStr, sync::Arc, time::Duration};
use uuid::Uuid;
mod dynamic;
pub use dynamic::{ArgumentSchema, ArgumentType, CommandSchema, CommandValues, DynamicCommandSpec};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
/// Typed argument/policy rejections and invalid command definitions. Expected rejection completes only after a successful reply or explicit failure disposition.
///
/// # Example
///
/// ```rust
/// use rappin::commands::CommandError;
/// let error = CommandError::Missing("recipient".into());
/// assert_eq!(error.kind(), "missing");
/// assert!(error.is_rejection());
/// assert!(!CommandError::Definition("duplicate name".into()).is_rejection());
/// ```
pub enum CommandError {
    #[error("unterminated quoted argument or escape")]
    /// Input ends inside a quote or after an incomplete escape.
    Quoting,
    #[error("missing argument: {0}")]
    /// A required named argument was omitted.
    Missing(#[doc = "Missing argument name."] String),
    #[error("invalid {name}: {reason}")]
    /// An argument could not be parsed or violated its bounds.
    Invalid {
        #[doc = "Name of the invalid argument."]
        name: String,
        #[doc = "Parsing or validation explanation."]
        reason: String,
    },
    #[error("unexpected extra arguments")]
    /// Unconsumed tokens remain after the declared arguments.
    Extra,
    #[error("command denied: {0}")]
    /// A command policy rejected the caller.
    Denied(#[doc = "Reason access was rejected."] String),
    #[error("invalid command definition: {0}")]
    /// Command metadata is invalid; classified as an unexpected configuration failure.
    Definition(#[doc = "Invalid definition detail."] String),
}
impl CommandError {
    /// Stable snake-case classification for structured reporting.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::CommandError;
    /// let error = CommandError::Missing("recipient".into());
    /// assert_eq!(error.kind(), "missing");
    /// assert!(error.is_rejection());
    /// assert!(!CommandError::Definition("duplicate name".into()).is_rejection());
    /// ```
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Quoting => "quoting",
            Self::Missing(_) => "missing",
            Self::Invalid { .. } => "invalid",
            Self::Extra => "extra",
            Self::Denied(_) => "denied",
            Self::Definition(_) => "definition",
        }
    }
    /// Whether this is an expected user/policy rejection rather than an invalid definition.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::CommandError;
    /// let error = CommandError::Missing("recipient".into());
    /// assert_eq!(error.kind(), "missing");
    /// assert!(error.is_rejection());
    /// assert!(!CommandError::Definition("duplicate name".into()).is_rejection());
    /// ```
    pub fn is_rejection(&self) -> bool {
        !matches!(self, Self::Definition(_))
    }
}

#[derive(Debug, Clone)]
/// A decoded argument and its UTF-8 byte range in the original input.
///
/// # Example
///
/// ```rust
/// use rappin::commands::tokenize;
/// let tokens = tokenize(r#"20 "two words""#)?;
/// assert_eq!(tokens[1].value, "two words");
/// assert_eq!((tokens[1].start, tokens[1].end), (3, 14));
/// # Ok::<(), rappin::commands::CommandError>(())
/// ```
pub struct Token {
    /// Decoded token with grouping quotes removed and backslash escapes resolved.
    pub value: String,
    /// Inclusive UTF-8 byte offset in the original input.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the original input.
    pub end: usize,
}
/// Shell-like grouping only: no expansion or execution. Quotes and backslash escapes are respected.
/// # Example
///
/// ```rust
/// use rappin::commands::tokenize;
/// let tokens = tokenize(r#"20 "two words""#)?;
/// assert_eq!(tokens[1].value, "two words");
/// assert_eq!((tokens[1].start, tokens[1].end), (3, 14));
/// # Ok::<(), rappin::commands::CommandError>(())
/// ```
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
/// # Example
///
/// ```rust
/// use rappin::commands::Arguments;
/// let mut args = Arguments::new(r#"20 "two words""#)?;
/// let sides: u32 = args.required("sides")?;
/// let label: Option<String> = args.optional("label")?;
/// assert_eq!(sides, 20);
/// assert_eq!(label.as_deref(), Some("two words"));
/// args.finish()?;
/// # Ok::<(), rappin::commands::CommandError>(())
/// ```
pub struct Arguments {
    tokens: std::vec::IntoIter<Token>,
}
impl Arguments {
    /// Tokenize input into a quote-aware cursor without shell execution.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Arguments;
    /// let mut args = Arguments::new(r#"20 "two words""#)?;
    /// let sides: u32 = args.required("sides")?;
    /// let label: Option<String> = args.optional("label")?;
    /// assert_eq!(sides, 20);
    /// assert_eq!(label.as_deref(), Some("two words"));
    /// args.finish()?;
    /// # Ok::<(), rappin::commands::CommandError>(())
    /// ```
    pub fn new(input: &str) -> std::result::Result<Self, CommandError> {
        Ok(Self {
            tokens: tokenize(input)?.into_iter(),
        })
    }
    /// Consume and parse one required token; report its argument name on missing/invalid input.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Arguments;
    /// let mut args = Arguments::new(r#"20 "two words""#)?;
    /// let sides: u32 = args.required("sides")?;
    /// let label: Option<String> = args.optional("label")?;
    /// assert_eq!(sides, 20);
    /// assert_eq!(label.as_deref(), Some("two words"));
    /// args.finish()?;
    /// # Ok::<(), rappin::commands::CommandError>(())
    /// ```
    pub fn required<T: FromStr>(&mut self, name: &str) -> std::result::Result<T, CommandError>
    where
        T::Err: std::fmt::Display,
    {
        self.optional(name)?
            .ok_or_else(|| CommandError::Missing(name.into()))
    }
    /// Consume and parse one token if present; invalid input remains an error.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Arguments;
    /// let mut args = Arguments::new(r#"20 "two words""#)?;
    /// let sides: u32 = args.required("sides")?;
    /// let label: Option<String> = args.optional("label")?;
    /// assert_eq!(sides, 20);
    /// assert_eq!(label.as_deref(), Some("two words"));
    /// args.finish()?;
    /// # Ok::<(), rappin::commands::CommandError>(())
    /// ```
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
    /// Consume all remaining decoded tokens and join them with single spaces.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Arguments;
    /// let mut args = Arguments::new(r#"hello "two words""#)?;
    /// assert_eq!(args.rest(), "hello two words");
    /// args.finish()?;
    /// # Ok::<(), rappin::commands::CommandError>(())
    /// ```
    pub fn rest(&mut self) -> String {
        self.tokens
            .by_ref()
            .map(|t| t.value)
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// Reject any unconsumed token after argument parsing.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Arguments;
    /// let mut args = Arguments::new(r#"20 "two words""#)?;
    /// let sides: u32 = args.required("sides")?;
    /// let label: Option<String> = args.optional("label")?;
    /// assert_eq!(sides, 20);
    /// assert_eq!(label.as_deref(), Some("two words"));
    /// args.finish()?;
    /// # Ok::<(), rappin::commands::CommandError>(())
    /// ```
    pub fn finish(&mut self) -> std::result::Result<(), CommandError> {
        if self.tokens.next().is_some() {
            Err(CommandError::Extra)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Local help and registration metadata; metadata alone does not enforce permission checks.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentInfo, CommandSpec};
/// let required = ArgumentInfo::required("recipient", "User ID");
/// let optional = ArgumentInfo::optional("reason", "Audit reason");
/// assert!(required.required);
/// assert!(!optional.required);
/// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
/// info.validate()?;
/// assert_eq!(info.usage(), "/status");
/// assert!(info.help().contains("Show status"));
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub struct ArgumentInfo {
    /// Unique local argument name without whitespace.
    pub name: String,
    /// Human-readable help for this argument.
    pub description: String,
    /// Whether help renders a required rather than optional placeholder.
    pub required: bool,
}
impl ArgumentInfo {
    /// Describe a required argument for generated help.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, CommandSpec};
    /// let required = ArgumentInfo::required("recipient", "User ID");
    /// let optional = ArgumentInfo::optional("reason", "Audit reason");
    /// assert!(required.required);
    /// assert!(!optional.required);
    /// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
    /// info.validate()?;
    /// assert_eq!(info.usage(), "/status");
    /// assert!(info.help().contains("Show status"));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn required(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            required: true,
        }
    }
    /// Describe an optional argument for generated help.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, CommandSpec};
    /// let required = ArgumentInfo::required("recipient", "User ID");
    /// let optional = ArgumentInfo::optional("reason", "Audit reason");
    /// assert!(required.required);
    /// assert!(!optional.required);
    /// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
    /// info.validate()?;
    /// assert_eq!(info.usage(), "/status");
    /// assert!(info.help().contains("Show status"));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn optional(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            required: false,
        }
    }
}
/// Parse a command's Rust argument type; `describe` supplies manually maintained help metadata.
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentInfo, Arguments, CommandError, FromArgs};
/// struct Roll { sides: u32 }
/// impl FromArgs for Roll {
///     fn parse(args: &mut Arguments) -> Result<Self, CommandError> {
///         Ok(Self { sides: args.required("sides")? })
///     }
///     fn describe() -> Vec<ArgumentInfo> { vec![ArgumentInfo::required("sides", "Number of sides")] }
/// }
/// let mut input = Arguments::new("20")?;
/// assert_eq!(Roll::parse(&mut input)?.sides, 20);
/// input.finish()?;
/// # Ok::<(), CommandError>(())
/// ```
pub trait FromArgs: Send + 'static {
    /// Parse the declared Rust type from the cursor; dispatch rejects remaining tokens afterward.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, Arguments, CommandError, FromArgs};
    /// struct Roll { sides: u32 }
    /// impl FromArgs for Roll {
    ///     fn parse(args: &mut Arguments) -> Result<Self, CommandError> {
    ///         Ok(Self { sides: args.required("sides")? })
    ///     }
    ///     fn describe() -> Vec<ArgumentInfo> { vec![ArgumentInfo::required("sides", "Number of sides")] }
    /// }
    /// let mut input = Arguments::new("20")?;
    /// assert_eq!(Roll::parse(&mut input)?.sides, 20);
    /// input.finish()?;
    /// # Ok::<(), CommandError>(())
    /// ```
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError>
    where
        Self: Sized;
    /// Describe ordered arguments for generated usage/help; keep this metadata aligned with parse.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, Arguments, CommandError, FromArgs};
    /// struct Roll { sides: u32 }
    /// impl FromArgs for Roll {
    ///     fn parse(args: &mut Arguments) -> Result<Self, CommandError> {
    ///         Ok(Self { sides: args.required("sides")? })
    ///     }
    ///     fn describe() -> Vec<ArgumentInfo> { vec![ArgumentInfo::required("sides", "Number of sides")] }
    /// }
    /// let mut input = Arguments::new("20")?;
    /// assert_eq!(Roll::parse(&mut input)?.sides, 20);
    /// input.finish()?;
    /// # Ok::<(), CommandError>(())
    /// ```
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
/// # Example
///
/// ```rust
/// use rappin::commands::HumanDuration;
/// let delay: HumanDuration = "1h30m".parse()?;
/// assert_eq!(delay.0.as_secs(), 5400);
/// # Ok::<(), rappin::commands::CommandError>(())
/// ```
pub struct HumanDuration(#[doc = "Parsed positive duration."] pub Duration);
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
/// Integer parser with inclusive compile-time minimum and maximum.
///
/// # Example
///
/// ```rust
/// use rappin::commands::BoundedU32;
/// let sides: BoundedU32<1, 100> = "20".parse()?;
/// assert_eq!(sides.0, 20);
/// assert!("101".parse::<BoundedU32<1, 100>>().is_err());
/// # Ok::<(), rappin::commands::CommandError>(())
/// ```
pub struct BoundedU32<const MIN: u32, const MAX: u32>(
    #[doc = "Parsed integer within the inclusive bounds."] pub u32,
);
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
/// # Example
///
/// ```rust
/// use rappin::commands::UserId;
/// let id: UserId = "<@00000000-0000-0000-0000-000000000000>".parse()?;
/// assert!(id.0.is_nil());
/// # Ok::<(), uuid::Error>(())
/// ```
pub struct UserId(
    #[doc = "Parsed user UUID; existence and permissions are not established."] pub Uuid,
);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Channel UUID parser accepting a raw UUID or channel mention.
///
/// # Example
///
/// ```rust
/// use rappin::commands::ChannelId;
/// let id: ChannelId = "<#00000000-0000-0000-0000-000000000000>".parse()?;
/// assert!(id.0.is_nil());
/// # Ok::<(), uuid::Error>(())
/// ```
pub struct ChannelId(
    #[doc = "Parsed channel UUID; scope and access require separate checks."] pub Uuid,
);
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
/// Server permission name. Construction labels a requirement; it does not grant access.
///
/// # Example
///
/// ```rust
/// use rappin::commands::Permission;
/// let permission = Permission::new("manage_channels");
/// assert_eq!(permission.as_str(), "manage_channels");
/// ```
pub struct Permission(String);
impl Permission {
    /// Name a permission requirement without evaluating access.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// let permission = Permission::new("manage_channels");
    /// assert_eq!(permission.as_str(), "manage_channels");
    /// ```
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    /// Borrow the permission name.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// let permission = Permission::new("manage_channels");
    /// assert_eq!(permission.as_str(), "manage_channels");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Name the manage_messages server permission; evaluation remains a separate step.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// assert_eq!(Permission::manage_messages().as_str(), "manage_messages");
    /// ```
    pub fn manage_messages() -> Self {
        Self::new("manage_messages")
    }
    /// Name the manage_server server permission; evaluation remains a separate step.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// assert_eq!(Permission::manage_server().as_str(), "manage_server");
    /// ```
    pub fn manage_server() -> Self {
        Self::new("manage_server")
    }
    /// Name the kick_members server permission; evaluation remains a separate step.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// assert_eq!(Permission::kick_members().as_str(), "kick_members");
    /// ```
    pub fn kick_members() -> Self {
        Self::new("kick_members")
    }
    /// Name the ban_members server permission; evaluation remains a separate step.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// assert_eq!(Permission::ban_members().as_str(), "ban_members");
    /// ```
    pub fn ban_members() -> Self {
        Self::new("ban_members")
    }
    /// Name the timeout_members server permission; evaluation remains a separate step.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Permission;
    /// assert_eq!(Permission::timeout_members().as_str(), "timeout_members");
    /// ```
    pub fn timeout_members() -> Self {
        Self::new("timeout_members")
    }
}
/// Boxed Send future for command policy evaluation.
pub type CheckFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
/// An async authorization/policy check run before a typed handler.
/// # Example
///
/// ```rust
/// use rappin::{MessageContext, commands::{CommandCheck, CheckFuture, CommandError}};
/// struct ServerOnly;
/// impl CommandCheck for ServerOnly {
///     fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a> {
///         Box::pin(async move {
///             if ctx.server_id().is_none() { return Err(CommandError::Denied("Server required".into()).into()); }
///             Ok(())
///         })
///     }
///     fn prepare<'a>(&'a self, ctx: &'a MessageContext) -> Option<CheckFuture<'a>> { Some(self.check(ctx)) }
/// }
/// ```
pub trait CommandCheck: Send + Sync + 'static {
    /// Evaluate current authority before handler execution; execution runs it even after successful preparation.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{MessageContext, commands::{CommandCheck, CheckFuture, CommandError}};
    /// struct ServerOnly;
    /// impl CommandCheck for ServerOnly {
    ///     fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a> {
    ///         Box::pin(async move {
    ///             if ctx.server_id().is_none() { return Err(CommandError::Denied("Server required".into()).into()); }
    ///             Ok(())
    ///         })
    ///     }
    ///     fn prepare<'a>(&'a self, ctx: &'a MessageContext) -> Option<CheckFuture<'a>> { Some(self.check(ctx)) }
    /// }
    /// ```
    fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a>;
    /// Optional read-only assessment. Must not reply, mutate application state,
    /// reserve access, or consume cooldowns. Execution always calls `check` again.
    /// The default reports this check as deferred.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{MessageContext, commands::{CommandCheck, CheckFuture, CommandError}};
    /// struct ServerOnly;
    /// impl CommandCheck for ServerOnly {
    ///     fn check<'a>(&'a self, ctx: &'a MessageContext) -> CheckFuture<'a> {
    ///         Box::pin(async move {
    ///             if ctx.server_id().is_none() { return Err(CommandError::Denied("Server required".into()).into()); }
    ///             Ok(())
    ///         })
    ///     }
    ///     fn prepare<'a>(&'a self, ctx: &'a MessageContext) -> Option<CheckFuture<'a>> { Some(self.check(ctx)) }
    /// }
    /// ```
    fn prepare<'a>(&'a self, _ctx: &'a MessageContext) -> Option<CheckFuture<'a>> {
        None
    }
}

/// Closure-backed command check created by check_fn; arbitrary checks are deferred during preparation.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{check_fn, CommandSpec, CommandError};
/// let check = check_fn(|ctx| async move {
///     if ctx.server_id().is_none() { return Err(CommandError::Denied("Server required".into()).into()); }
///     Ok(())
/// });
/// let spec = CommandSpec::<()>::new("status", "Show status").check(check);
/// ```
pub struct CheckFn<F>(F);
/// Adapt an async closure into a reusable check. The closure receives a cloned context.
/// # Example
///
/// ```rust
/// use rappin::commands::{check_fn, CommandSpec, CommandError};
/// let check = check_fn(|ctx| async move {
///     if ctx.server_id().is_none() { return Err(CommandError::Denied("Server required".into()).into()); }
///     Ok(())
/// });
/// let spec = CommandSpec::<()>::new("status", "Show status").check(check);
/// ```
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
/// Reusable built-in command checks; requester server permissions do not establish resource-specific overrides.
///
/// # Example
///
/// ```rust
/// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
/// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
///     .alias("clean")
///     .check(Check::server_only())
///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
/// assert_eq!(command.info.name, "cleanup");
/// ```
pub enum Check {
    /// Allow only the bot owner from the bot profile.
    Owner,
    /// Require a server message context.
    ServerOnly,
    /// Require at least one listed assigned role.
    Roles {
        /// Any of these role IDs satisfies this check.
        ids: Vec<Uuid>,
        /// Whether cached membership evidence may be used.
        freshness: Freshness,
    },
    /// Require every listed server-wide permission.
    Permissions {
        /// All listed server-wide permissions are required.
        permissions: Vec<Permission>,
        /// Whether cached role/membership evidence may be used.
        freshness: Freshness,
    },
}
impl Check {
    /// Require the authenticated bot profile's owner, not the server owner.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{Check, CommandSpec};
    /// let command = CommandSpec::<()>::new("admin", "Bot administration").check(Check::owner_only());
    /// ```
    pub fn owner_only() -> Self {
        Self::Owner
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
    pub fn server_only() -> Self {
        Self::ServerOnly
    }
    /// Require membership in at least one selected role; defaults to fresh evidence.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::Check;
    /// let role = uuid::Uuid::nil();
    /// let required = Check::roles([role]);
    /// ```
    pub fn roles(ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self::Roles {
            ids: ids.into_iter().collect(),
            freshness: Freshness::Refresh,
        }
    }
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
    pub fn permissions(permissions: impl IntoIterator<Item = Permission>) -> Self {
        Self::Permissions {
            permissions: permissions.into_iter().collect(),
            freshness: Freshness::Refresh,
        }
    }
    /// Cached permissions are opt-in; Refresh is the default for authorization checks.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
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
    fn prepare<'a>(&'a self, ctx: &'a MessageContext) -> Option<CheckFuture<'a>> {
        Some(self.check(ctx))
    }
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
                    crate::ApiExt::evaluate_permissions(
                        ctx.client.as_ref(),
                        server,
                        ctx.author_id(),
                        *freshness,
                    )
                    .await?
                    .require(permissions)?;
                }
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Local help and registration metadata; metadata alone does not enforce permission checks.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentInfo, CommandSpec};
/// let required = ArgumentInfo::required("recipient", "User ID");
/// let optional = ArgumentInfo::optional("reason", "Audit reason");
/// assert!(required.required);
/// assert!(!optional.required);
/// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
/// info.validate()?;
/// assert_eq!(info.usage(), "/status");
/// assert!(info.help().contains("Show status"));
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub struct CommandInfo {
    /// Canonical lowercase command path; spaces denote local groups.
    pub name: String,
    /// Description advertised with the command and shown in help.
    pub description: String,
    /// Additional local command paths resolving to the same binding.
    pub aliases: Vec<String>,
    /// Ordered local parsing/help metadata.
    pub arguments: Vec<ArgumentInfo>,
}
impl CommandInfo {
    /// Definition-only validation; does not call handlers or perform network I/O.
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, CommandSpec};
    /// let required = ArgumentInfo::required("recipient", "User ID");
    /// let optional = ArgumentInfo::optional("reason", "Audit reason");
    /// assert!(required.required);
    /// assert!(!optional.required);
    /// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
    /// info.validate()?;
    /// assert_eq!(info.usage(), "/status");
    /// assert!(info.help().contains("Show status"));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for name in std::iter::once(&self.name).chain(&self.aliases) {
            if !names.insert(name) || name.split(' ').count() > 8 {
                return Err(CommandError::Definition(
                    "command or alias already registered, or subcommand depth exceeds eight".into(),
                )
                .into());
            }
            for part in name.split(' ') {
                crate::validation::validate_commands(&[crate::Command::new(
                    part,
                    &self.description,
                )])?;
            }
        }
        let mut names = std::collections::HashSet::new();
        for arg in &self.arguments {
            if arg.name.is_empty()
                || arg.name.chars().any(char::is_whitespace)
                || !names.insert(&arg.name)
            {
                return Err(CommandError::Definition(
                    "arguments need unique nonempty names without whitespace".into(),
                )
                .into());
            }
        }
        Ok(())
    }
    /// Render the slash-style command path and required/optional argument placeholders.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, CommandSpec};
    /// let required = ArgumentInfo::required("recipient", "User ID");
    /// let optional = ArgumentInfo::optional("reason", "Audit reason");
    /// assert!(required.required);
    /// assert!(!optional.required);
    /// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
    /// info.validate()?;
    /// assert_eq!(info.usage(), "/status");
    /// assert!(info.help().contains("Show status"));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
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
    /// Render usage, description, aliases and argument descriptions.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentInfo, CommandSpec};
    /// let required = ArgumentInfo::required("recipient", "User ID");
    /// let optional = ArgumentInfo::optional("reason", "Audit reason");
    /// assert!(required.required);
    /// assert!(!optional.required);
    /// let info = CommandSpec::<()>::new("status", "Show status").info().clone();
    /// info.validate()?;
    /// assert_eq!(info.usage(), "/status");
    /// assert!(info.help().contains("Show status"));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
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
/// # Example
///
/// ```rust
/// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
/// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
///     .alias("clean")
///     .check(Check::server_only())
///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
/// assert_eq!(command.info.name, "cleanup");
/// ```
pub struct CommandSpec<A: FromArgs> {
    pub(crate) info: CommandInfo,
    checks: Vec<Arc<dyn CommandCheck>>,
    marker: PhantomData<fn() -> A>,
}
impl<A: FromArgs> CommandSpec<A> {
    /// Space-separated names define a local subcommand path, advertised as one root command.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
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
    /// Add a local alias for the same command binding.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
    pub fn alias(mut self, name: impl Into<String>) -> Self {
        self.info.aliases.push(name.into().to_lowercase());
        self
    }
    /// Append a check executed before the typed handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
    pub fn check(mut self, check: impl CommandCheck) -> Self {
        self.checks.push(Arc::new(check));
        self
    }
    /// Borrow the generated registration/help metadata.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
    pub fn info(&self) -> &CommandInfo {
        &self.info
    }
    /// Bind the typed handler; execution parses arguments and runs registered checks.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
    /// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
    ///     .alias("clean")
    ///     .check(Check::server_only())
    ///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
    ///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
    /// assert_eq!(command.info.name, "cleanup");
    /// ```
    pub fn bind<F, Fut>(self, handler: F) -> RegisteredCommand
    where
        F: Fn(MessageContext, A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let checks = Arc::new(self.checks);
        let handler = Arc::new(handler);
        RegisteredCommand {
            info: self.info,
            preparation: Some(CommandPreparation {
                schema: None,
                checks: checks.clone(),
            }),
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
/// # Example
///
/// ```rust
/// use rappin::{Freshness, commands::{Check, CommandSpec, Permission}};
/// let command = CommandSpec::<()>::new("cleanup", "Clean up messages")
///     .alias("clean")
///     .check(Check::server_only())
///     .check(Check::permissions([Permission::manage_messages()]).freshness(Freshness::Refresh))
///     .bind(|ctx, ()| async move { ctx.reply("Authorized").await?; Ok(()) });
/// assert_eq!(command.info.name, "cleanup");
/// ```
pub struct RegisteredCommand {
    /// Registration metadata for the bound handler; prefer CommandSpec or DynamicCommandSpec to keep parsing and help aligned.
    pub info: CommandInfo,
    pub(crate) handler: crate::bot::CommandHandlerFn,
    pub(crate) preparation: Option<CommandPreparation>,
}
#[derive(Clone)]
pub(crate) struct CommandPreparation {
    pub schema: Option<Arc<CommandSchema>>,
    pub checks: Arc<Vec<Arc<dyn CommandCheck>>>,
}
