//! Runtime-owned schemas using the same routing and checks as typed commands.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Runtime argument validation rules; parsing remains pure and quote-aware.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
/// let schema = CommandSchema {
///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
///     arguments: vec![ArgumentSchema {
///         name: "sides".into(), description: "Die sides".into(),
///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
///     }],
/// };
/// schema.validate()?;
/// assert_eq!(schema.parse("20")?["sides"], 20);
/// assert!(schema.parse("101").is_err());
/// assert_eq!(schema.info().usage(), "/roll <sides>");
/// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
/// });
/// # Ok::<(), rappin::Error>(())
/// ```
pub enum ArgumentType {
    /// A catalogue argument schema. Strings remain strings; other values use a
    /// quoted JSON token. Validation uses the catalogue's supported schema subset.
    Schema {
        /// Supported JSON Schema subset from the public capability catalogue.
        schema: Value,
    },
    /// Decoded string with Unicode character bounds.
    String {
        #[serde(default)]
        /// Inclusive minimum Unicode character count.
        min_length: usize,
        /// Optional inclusive maximum Unicode character count.
        max_length: Option<usize>,
    },
    /// Signed 64-bit integer with inclusive bounds.
    Integer {
        /// Optional inclusive integer minimum.
        min: Option<i64>,
        /// Optional inclusive integer maximum.
        max: Option<i64>,
    },
    /// Finite floating-point number with inclusive bounds.
    Number {
        /// Optional inclusive finite-number minimum.
        min: Option<f64>,
        /// Optional inclusive finite-number maximum.
        max: Option<f64>,
    },
    /// Boolean token accepted by the runtime parser.
    Boolean,
    /// User UUID or mention; does not establish access or existence.
    UserId,
    /// Channel UUID or mention; scope and authorization remain separate.
    ChannelId,
    /// Positive human duration converted to seconds.
    Duration {
        /// Optional inclusive minimum duration in seconds.
        min_seconds: Option<u64>,
        /// Optional inclusive positive maximum duration in seconds.
        max_seconds: Option<u64>,
    },
    /// One of a nonempty set of exact literal strings.
    Choice {
        /// Nonempty unique accepted literal strings.
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One ordered runtime command argument.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
/// let schema = CommandSchema {
///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
///     arguments: vec![ArgumentSchema {
///         name: "sides".into(), description: "Die sides".into(),
///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
///     }],
/// };
/// schema.validate()?;
/// assert_eq!(schema.parse("20")?["sides"], 20);
/// assert!(schema.parse("101").is_err());
/// assert_eq!(schema.info().usage(), "/roll <sides>");
/// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
/// });
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct ArgumentSchema {
    /// Unique argument key used in parsed values and help.
    pub name: String,
    #[serde(default)]
    /// Human-readable argument help.
    pub description: String,
    /// Validation/parsing rule for this argument.
    pub kind: ArgumentType,
    #[serde(default = "required")]
    /// Whether omission is rejected; required arguments precede optional ones.
    pub required: bool,
    /// Consume remaining tokens as one string; permitted only on the last string argument.
    #[serde(default)]
    pub rest: bool,
}
fn required() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Serializable command definition with shared parsing, validation and help.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
/// let schema = CommandSchema {
///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
///     arguments: vec![ArgumentSchema {
///         name: "sides".into(), description: "Die sides".into(),
///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
///     }],
/// };
/// schema.validate()?;
/// assert_eq!(schema.parse("20")?["sides"], 20);
/// assert!(schema.parse("101").is_err());
/// assert_eq!(schema.info().usage(), "/roll <sides>");
/// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
/// });
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct CommandSchema {
    /// Canonical local command path.
    pub name: String,
    /// Description advertised to the platform and shown in help.
    pub description: String,
    #[serde(default)]
    /// Additional paths selecting the same installed binding.
    pub aliases: Vec<String>,
    #[serde(default)]
    /// Ordered argument definitions used for parsing and help.
    pub arguments: Vec<ArgumentSchema>,
}

/// Ordered map of parsed named argument values; omitted optional arguments are absent.
pub type CommandValues = std::collections::BTreeMap<String, Value>;

impl CommandSchema {
    /// Generate help/registration metadata from the same runtime schema.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
    /// let schema = CommandSchema {
    ///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
    ///     arguments: vec![ArgumentSchema {
    ///         name: "sides".into(), description: "Die sides".into(),
    ///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
    ///     }],
    /// };
    /// schema.validate()?;
    /// assert_eq!(schema.parse("20")?["sides"], 20);
    /// assert!(schema.parse("101").is_err());
    /// assert_eq!(schema.info().usage(), "/roll <sides>");
    /// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
    ///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
    /// });
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn info(&self) -> CommandInfo {
        CommandInfo {
            name: self.name.clone(),
            description: self.description.clone(),
            aliases: self.aliases.clone(),
            arguments: self
                .arguments
                .iter()
                .map(|a| ArgumentInfo {
                    name: a.name.clone(),
                    description: a.description.clone(),
                    required: a.required,
                })
                .collect(),
        }
    }
    /// Validate names, argument ordering, rest placement and bounds without executing a handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
    /// let schema = CommandSchema {
    ///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
    ///     arguments: vec![ArgumentSchema {
    ///         name: "sides".into(), description: "Die sides".into(),
    ///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
    ///     }],
    /// };
    /// schema.validate()?;
    /// assert_eq!(schema.parse("20")?["sides"], 20);
    /// assert!(schema.parse("101").is_err());
    /// assert_eq!(schema.info().usage(), "/roll <sides>");
    /// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
    ///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
    /// });
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        self.info().validate()?;
        let definition = |reason: &str| Error::from(CommandError::Definition(reason.into()));
        let mut optional = false;
        for (i, arg) in self.arguments.iter().enumerate() {
            if optional && arg.required {
                return Err(definition(
                    "required arguments must precede optional arguments",
                ));
            }
            optional |= !arg.required;
            let string_kind = matches!(arg.kind, ArgumentType::String { .. })
                || matches!(&arg.kind,ArgumentType::Schema{schema} if schema["type"]=="string");
            if arg.rest && (i + 1 != self.arguments.len() || !string_kind) {
                return Err(definition(
                    "rest is permitted only on the final string argument",
                ));
            }
            let valid = match &arg.kind {
                ArgumentType::Schema { schema } => schema.is_object() || schema.is_boolean(),
                ArgumentType::String {
                    min_length,
                    max_length,
                } => max_length.is_none_or(|max| max >= *min_length),
                ArgumentType::Integer { min, max } => {
                    min.zip(*max).is_none_or(|(min, max)| min <= max)
                }
                ArgumentType::Number { min, max } => {
                    min.is_none_or(f64::is_finite)
                        && max.is_none_or(f64::is_finite)
                        && min.zip(*max).is_none_or(|(min, max)| min <= max)
                }
                ArgumentType::Duration {
                    min_seconds,
                    max_seconds,
                } => {
                    min_seconds
                        .zip(*max_seconds)
                        .is_none_or(|(min, max)| min <= max)
                        && *max_seconds != Some(0)
                }
                ArgumentType::Choice { values } => {
                    !values.is_empty()
                        && values.iter().all(|s| !s.is_empty())
                        && values
                            .iter()
                            .collect::<std::collections::HashSet<_>>()
                            .len()
                            == values.len()
                }
                _ => true,
            };
            if !valid {
                return Err(definition("invalid argument bounds or choices"));
            }
        }
        Ok(())
    }
    /// Parsing is pure, quote-aware and validates the schema before any handler executes.
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
    /// let schema = CommandSchema {
    ///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
    ///     arguments: vec![ArgumentSchema {
    ///         name: "sides".into(), description: "Die sides".into(),
    ///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
    ///     }],
    /// };
    /// schema.validate()?;
    /// assert_eq!(schema.parse("20")?["sides"], 20);
    /// assert!(schema.parse("101").is_err());
    /// assert_eq!(schema.info().usage(), "/roll <sides>");
    /// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
    ///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
    /// });
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn parse(&self, input: &str) -> Result<CommandValues> {
        self.validate()?;
        let tokens = tokenize(input)?;
        let mut cursor = 0;
        let mut values = CommandValues::new();
        for arg in &self.arguments {
            let Some(token) = tokens.get(cursor) else {
                if arg.required {
                    return Err(CommandError::Missing(arg.name.clone()).into());
                }
                continue;
            };
            let raw = if arg.rest {
                tokens[cursor..]
                    .iter()
                    .map(|t| t.value.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                token.value.clone()
            };
            cursor = if arg.rest { tokens.len() } else { cursor + 1 };
            let invalid = || {
                Error::from(CommandError::Invalid {
                    name: arg.name.clone(),
                    reason: format!("expected {:?}", arg.kind),
                })
            };
            let value = match &arg.kind {
                ArgumentType::Schema { schema } => {
                    let string_type = schema["type"] == "string"
                        || schema["type"]
                            .as_array()
                            .is_some_and(|a| a.iter().any(|t| t == "string") && raw != "null")
                        || schema["enum"]
                            .as_array()
                            .is_some_and(|a| a.iter().all(Value::is_string));
                    let value = if string_type {
                        Value::String(raw)
                    } else {
                        serde_json::from_str(&raw).map_err(|_| invalid())?
                    };
                    crate::capabilities::validate_schema(schema, &value).map_err(|_| {
                        CommandError::Invalid {
                            name: arg.name.clone(),
                            reason: "value does not match the operation argument schema".into(),
                        }
                    })?;
                    value
                }
                ArgumentType::String {
                    min_length,
                    max_length,
                } => {
                    let count = raw.chars().count();
                    if count < *min_length || max_length.is_some_and(|max| count > max) {
                        return Err(invalid());
                    }
                    Value::String(raw)
                }
                ArgumentType::Integer { min, max } => {
                    let n = raw.parse::<i64>().map_err(|_| invalid())?;
                    if min.is_some_and(|min| n < min) || max.is_some_and(|max| n > max) {
                        return Err(invalid());
                    }
                    n.into()
                }
                ArgumentType::Number { min, max } => {
                    let n = raw.parse::<f64>().map_err(|_| invalid())?;
                    if !n.is_finite()
                        || min.is_some_and(|min| n < min)
                        || max.is_some_and(|max| n > max)
                    {
                        return Err(invalid());
                    }
                    serde_json::Number::from_f64(n).ok_or_else(invalid)?.into()
                }
                ArgumentType::Boolean => match raw.as_str() {
                    "true" => true.into(),
                    "false" => false.into(),
                    _ => return Err(invalid()),
                },
                ArgumentType::UserId => raw
                    .parse::<UserId>()
                    .map_err(|_| invalid())?
                    .0
                    .to_string()
                    .into(),
                ArgumentType::ChannelId => raw
                    .parse::<ChannelId>()
                    .map_err(|_| invalid())?
                    .0
                    .to_string()
                    .into(),
                ArgumentType::Duration {
                    min_seconds,
                    max_seconds,
                } => {
                    let n = raw
                        .parse::<HumanDuration>()
                        .map_err(|_| invalid())?
                        .0
                        .as_secs();
                    if min_seconds.is_some_and(|min| n < min)
                        || max_seconds.is_some_and(|max| n > max)
                    {
                        return Err(invalid());
                    }
                    n.into()
                }
                ArgumentType::Choice { values } => {
                    if !values.contains(&raw) {
                        return Err(invalid());
                    }
                    raw.into()
                }
            };
            values.insert(arg.name.clone(), value);
        }
        if cursor != tokens.len() {
            return Err(CommandError::Extra.into());
        }
        Ok(values)
    }
}

/// Runtime schema and checks bound to the normal installed command pipeline.
///
/// # Example
///
/// ```rust
/// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
/// let schema = CommandSchema {
///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
///     arguments: vec![ArgumentSchema {
///         name: "sides".into(), description: "Die sides".into(),
///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
///     }],
/// };
/// schema.validate()?;
/// assert_eq!(schema.parse("20")?["sides"], 20);
/// assert!(schema.parse("101").is_err());
/// assert_eq!(schema.info().usage(), "/roll <sides>");
/// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
/// });
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct DynamicCommandSpec {
    schema: CommandSchema,
    checks: Vec<Arc<dyn CommandCheck>>,
}
impl DynamicCommandSpec {
    /// Create a runtime binding; registration validates its definition.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
    /// let schema = CommandSchema {
    ///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
    ///     arguments: vec![ArgumentSchema {
    ///         name: "sides".into(), description: "Die sides".into(),
    ///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
    ///     }],
    /// };
    /// schema.validate()?;
    /// assert_eq!(schema.parse("20")?["sides"], 20);
    /// assert!(schema.parse("101").is_err());
    /// assert_eq!(schema.info().usage(), "/roll <sides>");
    /// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
    ///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
    /// });
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn new(schema: CommandSchema) -> Self {
        Self {
            schema,
            checks: vec![],
        }
    }
    /// Append a normal command check, re-evaluated at execution.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
    /// let schema = CommandSchema {
    ///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
    ///     arguments: vec![ArgumentSchema {
    ///         name: "sides".into(), description: "Die sides".into(),
    ///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
    ///     }],
    /// };
    /// schema.validate()?;
    /// assert_eq!(schema.parse("20")?["sides"], 20);
    /// assert!(schema.parse("101").is_err());
    /// assert_eq!(schema.info().usage(), "/roll <sides>");
    /// let command = DynamicCommandSpec::new(schema).check(rappin::commands::Check::server_only()).bind(|ctx, values| async move {
    ///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
    /// });
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn check(mut self, check: impl CommandCheck) -> Self {
        self.checks.push(Arc::new(check));
        self
    }
    /// Bind parsed named JSON values to a normal native handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::commands::{ArgumentSchema, ArgumentType, CommandSchema, DynamicCommandSpec};
    /// let schema = CommandSchema {
    ///     name: "roll".into(), description: "Roll a die".into(), aliases: vec!["dice".into()],
    ///     arguments: vec![ArgumentSchema {
    ///         name: "sides".into(), description: "Die sides".into(),
    ///         kind: ArgumentType::Integer { min: Some(1), max: Some(100) }, required: true, rest: false,
    ///     }],
    /// };
    /// schema.validate()?;
    /// assert_eq!(schema.parse("20")?["sides"], 20);
    /// assert!(schema.parse("101").is_err());
    /// assert_eq!(schema.info().usage(), "/roll <sides>");
    /// let command = DynamicCommandSpec::new(schema).bind(|ctx, values| async move {
    ///     ctx.reply(format!("d{}", values["sides"])).await?; Ok(())
    /// });
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn bind<F, Fut>(self, handler: F) -> Result<RegisteredCommand>
    where
        F: Fn(MessageContext, CommandValues) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.schema.validate()?;
        let info = self.schema.info();
        let schema = Arc::new(self.schema);
        let checks = Arc::new(self.checks);
        let handler = Arc::new(handler);
        Ok(RegisteredCommand {
            info,
            preparation: Some(CommandPreparation {
                worker_checks: false,
                schema: Some(schema.clone()),
                checks: checks.clone(),
            }),
            handler: Arc::new(move |ctx| {
                let schema = schema.clone();
                let checks = checks.clone();
                let handler = handler.clone();
                Box::pin(async move {
                    let values = schema.parse(&ctx.args)?;
                    for check in checks.iter() {
                        check.check(&ctx).await?;
                    }
                    handler(ctx, values).await
                })
            }),
        })
    }
}
