use super::*;
use crate::{
    ArgumentSchema, ArgumentType, CommandSchema, CommandValues, DynamicCommandSpec, MessageContext,
    RegisteredCommand,
};
use std::{future::Future, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationParameter {
    pub name: String,
    /// JSON pointer into operation arguments, e.g. /content or /body/content.
    pub target: String,
    #[serde(default = "required")]
    pub required: bool,
    #[serde(default)]
    pub rest: bool,
    #[serde(default)]
    pub description: Option<String>,
}
fn required() -> bool {
    true
}
fn empty_object() -> Value {
    json!({})
}

/// Serializable command definition. Store this in the bot's own database;
/// parsing/help/binding are reconstructed from the current capability metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationCommandBinding {
    pub operation: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default = "empty_object")]
    pub fixed_arguments: Value,
    #[serde(default)]
    pub parameters: Vec<OperationParameter>,
}
impl OperationCommandBinding {
    pub fn command_schema(&self) -> Result<CommandSchema> {
        let capability = capability(&self.operation)
            .ok_or_else(|| YappinError::validation("operation", "unknown operation"))?;
        if !matches!(
            capability.availability.library,
            LibrarySupport::ScopedApi | LibrarySupport::Offline
        ) {
            return Err(YappinError::validation("operation","operation requires CLI/global runtime state and cannot be bound to a server command"));
        }
        if !self.fixed_arguments.is_object() {
            return Err(YappinError::validation(
                "fixed_arguments",
                "expected an object",
            ));
        }
        let input = schema::operation_input_schema(capability);
        schema::validate_partial(input, &self.fixed_arguments)?;
        let mut arguments = vec![];
        let mut targets: Vec<&str> = vec![];
        for parameter in &self.parameters {
            let field = at_schema(input, &parameter.target)?;
            if self.fixed_arguments.pointer(&parameter.target).is_some()
                || targets
                    .iter()
                    .any(|target| overlaps(target, &parameter.target))
            {
                return Err(YappinError::validation(
                    "parameters",
                    "argument targets must not overlap constants or other parameters",
                ));
            }
            targets.push(&parameter.target);
            arguments.push(ArgumentSchema {
                name: parameter.name.clone(),
                description: parameter
                    .description
                    .clone()
                    .or_else(|| field["description"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| {
                        format!("{} argument {}", capability.name, parameter.target)
                    }),
                kind: ArgumentType::Schema {
                    schema: field.clone(),
                },
                required: parameter.required,
                rest: parameter.rest,
            });
        }
        // Check unconditional required fields. Cross-field/typed constraints are
        // checked by prepare() once runtime values are known.
        for path in required_paths(input, String::new()) {
            let supplied = self.fixed_arguments.pointer(&path).is_some()
                || self.parameters.iter().any(|p| {
                    p.required
                        && (p.target == path
                            || path.starts_with(&format!("{}/", p.target))
                            || p.target.starts_with(&format!("{path}/")))
                });
            if !supplied {
                return Err(YappinError::validation(
                    "parameters",
                    format!("required operation argument is unbound: {path}"),
                ));
            }
        }
        let command = CommandSchema {
            name: self.name.clone(),
            description: self.description.clone(),
            aliases: self.aliases.clone(),
            arguments,
        };
        command.validate()?;
        if self.parameters.is_empty() {
            capability.validate(&self.fixed_arguments)?;
        }
        Ok(command)
    }
    pub fn validate(&self) -> Result<()> {
        self.command_schema().map(|_| ())
    }
    pub fn prepare(&self, values: &CommandValues) -> Result<ValidatedOperation> {
        self.validate()?;
        let input = capability(&self.operation)
            .expect("validated operation")
            .input_schema();
        let mut args = self.fixed_arguments.clone();
        for key in values.keys() {
            if !self.parameters.iter().any(|p| &p.name == key) {
                return Err(YappinError::validation(key, "unknown command argument"));
            }
        }
        for parameter in &self.parameters {
            match values.get(&parameter.name) {
                Some(value) => {
                    validate_schema(at_schema(&input, &parameter.target)?, value)?;
                    set_argument(&mut args, &parameter.target, value.clone())?;
                }
                None if parameter.required => {
                    return Err(crate::CommandError::Missing(parameter.name.clone()).into())
                }
                _ => (),
            }
        }
        capability(&self.operation)
            .expect("validated operation")
            .validate(&args)
    }
    pub async fn preflight(
        &self,
        handle: &crate::BotHandle,
        replacing_cog: Option<&str>,
    ) -> Result<crate::RegistrationPlan> {
        handle
            .preflight_commands(&[self.command_schema()?.info()], replacing_cog)
            .await
    }
    pub fn bind<F, Fut>(self, handler: F) -> Result<RegisteredCommand>
    where
        F: Fn(MessageContext, ValidatedOperation) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let schema = self.command_schema()?;
        let binding = Arc::new(self);
        let handler = Arc::new(handler);
        DynamicCommandSpec::new(schema).bind(move |ctx, values| {
            let binding = binding.clone();
            let handler = handler.clone();
            async move {
                let operation = binding.prepare(&values).map_err(|error| match error {
                    YappinError::Validation { field, message } => {
                        YappinError::Command(crate::CommandError::Invalid {
                            name: field,
                            reason: message,
                        })
                    }
                    other => other,
                })?;
                handler(ctx, operation).await
            }
        })
    }
}
fn overlaps(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}
fn tokens(pointer: &str) -> Result<Vec<String>> {
    if !pointer.starts_with('/') || pointer == "/" {
        return Err(YappinError::validation(
            "target",
            "use a nonempty JSON pointer to an operation argument",
        ));
    }
    pointer[1..]
        .split('/')
        .map(|token| {
            let mut chars = token.chars();
            while let Some(c) = chars.next() {
                if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                    return Err(YappinError::validation(
                        "target",
                        "invalid JSON pointer escape",
                    ));
                }
            }
            Ok(token.replace("~1", "/").replace("~0", "~"))
        })
        .collect()
}
fn at_schema<'a>(schema: &'a Value, pointer: &str) -> Result<&'a Value> {
    let mut field = schema;
    for key in tokens(pointer)? {
        field = field["properties"]
            .get(&key)
            .or_else(|| field.get("additionalProperties").filter(|s| s.is_object()))
            .ok_or_else(|| {
                YappinError::validation(
                    "target",
                    "unknown operation argument path; array indexing is not a command binding",
                )
            })?;
    }
    Ok(field)
}
fn set_argument(args: &mut Value, pointer: &str, value: Value) -> Result<()> {
    let tokens = tokens(pointer)?;
    let mut field = args;
    for (i, key) in tokens.iter().enumerate() {
        if !field.is_object() {
            return Err(YappinError::validation(
                "target",
                "argument path crosses a non-object constant",
            ));
        }
        if i + 1 == tokens.len() {
            field[key] = value;
            return Ok(());
        }
        if field.get(key).is_none() {
            field[key] = json!({});
        }
        field = &mut field[key];
    }
    unreachable!("nonempty pointer")
}
fn required_paths(schema: &Value, parent: String) -> Vec<String> {
    let mut paths = vec![];
    if let Some(required) = schema["required"].as_array() {
        for key in required.iter().filter_map(Value::as_str) {
            let path = format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"));
            let children = required_paths(&schema["properties"][key], path.clone());
            if children.is_empty() {
                paths.push(path);
            } else {
                paths.extend(children);
            }
        }
    }
    paths
}
