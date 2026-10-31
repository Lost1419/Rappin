//! Runtime-owned schemas using the same routing and checks as typed commands.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArgumentType {
    String {
        #[serde(default)]
        min_length: usize,
        max_length: Option<usize>,
    },
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Boolean,
    UserId,
    ChannelId,
    Duration {
        min_seconds: Option<u64>,
        max_seconds: Option<u64>,
    },
    Choice {
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArgumentSchema {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub kind: ArgumentType,
    #[serde(default = "required")]
    pub required: bool,
    /// Consume remaining tokens as one string; permitted only on the last string argument.
    #[serde(default)]
    pub rest: bool,
}
fn required() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSchema {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub arguments: Vec<ArgumentSchema>,
}

pub type CommandValues = std::collections::BTreeMap<String, Value>;

impl CommandSchema {
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
    pub fn validate(&self) -> Result<()> {
        self.info().validate()?;
        let definition = |reason: &str| YappinError::from(CommandError::Definition(reason.into()));
        let mut optional = false;
        for (i, arg) in self.arguments.iter().enumerate() {
            if optional && arg.required {
                return Err(definition(
                    "required arguments must precede optional arguments",
                ));
            }
            optional |= !arg.required;
            if arg.rest
                && (i + 1 != self.arguments.len()
                    || !matches!(arg.kind, ArgumentType::String { .. }))
            {
                return Err(definition(
                    "rest is permitted only on the final string argument",
                ));
            }
            let valid = match &arg.kind {
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
                YappinError::from(CommandError::Invalid {
                    name: arg.name.clone(),
                    reason: format!("expected {:?}", arg.kind),
                })
            };
            let value = match &arg.kind {
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

pub struct DynamicCommandSpec {
    schema: CommandSchema,
    checks: Vec<Arc<dyn CommandCheck>>,
}
impl DynamicCommandSpec {
    pub fn new(schema: CommandSchema) -> Self {
        Self {
            schema,
            checks: vec![],
        }
    }
    pub fn check(mut self, check: impl CommandCheck) -> Self {
        self.checks.push(Arc::new(check));
        self
    }
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
