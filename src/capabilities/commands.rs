use super::*;
use crate::{
    ArgumentSchema, ArgumentType, CommandSchema, CommandValues, DynamicCommandSpec, MessageContext,
    RegisteredCommand,
};
use std::{future::Future, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Persistable mapping from command arguments to validated operation metadata.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::OperationCommandBinding;
/// let binding: OperationCommandBinding = serde_json::from_value(serde_json::json!({
///     "operation":"send_message", "name":"announce", "description":"Post an announcement",
///     "fixed_arguments":{"channel":uuid::Uuid::nil()},
///     "parameters":[{"name":"text", "target":"/content", "required":true, "rest":true}]
/// }))?;
/// binding.validate()?;
/// let schema = binding.command_schema()?;
/// assert_eq!(schema.info().usage(), "/announce <text>");
/// let operation = binding.prepare(&schema.parse("Hello everyone")?)?;
/// assert_eq!(operation.arguments()["content"], "Hello everyone");
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct OperationParameter {
    /// Command argument key.
    pub name: String,
    /// JSON pointer into operation arguments, e.g. /content or /body/content.
    pub target: String,
    #[serde(default = "required")]
    /// Whether this command parameter must be supplied.
    pub required: bool,
    #[serde(default)]
    /// Consume remaining tokens as one final string argument.
    pub rest: bool,
    #[serde(default)]
    /// Optional help override; otherwise use capability schema help.
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
/// # Example
///
/// ```rust
/// use rappin::capabilities::OperationCommandBinding;
/// let binding: OperationCommandBinding = serde_json::from_value(serde_json::json!({
///     "operation":"send_message", "name":"announce", "description":"Post an announcement",
///     "fixed_arguments":{"channel":uuid::Uuid::nil()},
///     "parameters":[{"name":"text", "target":"/content", "required":true, "rest":true}]
/// }))?;
/// binding.validate()?;
/// let schema = binding.command_schema()?;
/// assert_eq!(schema.info().usage(), "/announce <text>");
/// let operation = binding.prepare(&schema.parse("Hello everyone")?)?;
/// assert_eq!(operation.arguments()["content"], "Hello everyone");
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct OperationCommandBinding {
    /// Canonical catalogue operation to validate.
    pub operation: String,
    /// Installed local command name.
    pub name: String,
    /// Command advertisement/help text.
    pub description: String,
    #[serde(default)]
    /// Additional local names selecting this binding.
    pub aliases: Vec<String>,
    #[serde(default = "empty_object")]
    /// Trusted fixed operation fields; they cannot overlap parameter targets.
    pub fixed_arguments: Value,
    #[serde(default)]
    /// Ordered command parameters mapped through JSON pointers.
    pub parameters: Vec<OperationParameter>,
}
impl OperationCommandBinding {
    /// Reconstruct parsing/help from current metadata and reject overlapping or unbound required fields.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::OperationCommandBinding;
    /// let binding: OperationCommandBinding = serde_json::from_value(serde_json::json!({
    ///     "operation":"send_message", "name":"announce", "description":"Post an announcement",
    ///     "fixed_arguments":{"channel":uuid::Uuid::nil()},
    ///     "parameters":[{"name":"text", "target":"/content", "required":true, "rest":true}]
    /// }))?;
    /// binding.validate()?;
    /// let schema = binding.command_schema()?;
    /// assert_eq!(schema.info().usage(), "/announce <text>");
    /// let operation = binding.prepare(&schema.parse("Hello everyone")?)?;
    /// assert_eq!(operation.arguments()["content"], "Hello everyone");
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn command_schema(&self) -> Result<CommandSchema> {
        let capability = capability(&self.operation)
            .ok_or_else(|| Error::validation("operation", "unknown operation"))?;
        if !matches!(
            capability.availability.library,
            LibrarySupport::ScopedApi | LibrarySupport::Offline
        ) {
            return Err(Error::validation("operation","operation requires CLI/global runtime state and cannot be bound to a server command"));
        }
        if !self.fixed_arguments.is_object() {
            return Err(Error::validation("fixed_arguments", "expected an object"));
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
                return Err(Error::validation(
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
                return Err(Error::validation(
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
    /// Validate the complete binding definition without executing a handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::OperationCommandBinding;
    /// let binding: OperationCommandBinding = serde_json::from_value(serde_json::json!({
    ///     "operation":"send_message", "name":"announce", "description":"Post an announcement",
    ///     "fixed_arguments":{"channel":uuid::Uuid::nil()},
    ///     "parameters":[{"name":"text", "target":"/content", "required":true, "rest":true}]
    /// }))?;
    /// binding.validate()?;
    /// let schema = binding.command_schema()?;
    /// assert_eq!(schema.info().usage(), "/announce <text>");
    /// let operation = binding.prepare(&schema.parse("Hello everyone")?)?;
    /// assert_eq!(operation.arguments()["content"], "Hello everyone");
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        self.command_schema().map(|_| ())
    }
    /// Combine parsed values and fixed arguments, then apply canonical typed operation validation.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::OperationCommandBinding;
    /// let binding: OperationCommandBinding = serde_json::from_value(serde_json::json!({
    ///     "operation":"send_message", "name":"announce", "description":"Post an announcement",
    ///     "fixed_arguments":{"channel":uuid::Uuid::nil()},
    ///     "parameters":[{"name":"text", "target":"/content", "required":true, "rest":true}]
    /// }))?;
    /// binding.validate()?;
    /// let schema = binding.command_schema()?;
    /// assert_eq!(schema.info().usage(), "/announce <text>");
    /// let operation = binding.prepare(&schema.parse("Hello everyone")?)?;
    /// assert_eq!(operation.arguments()["content"], "Hello everyone");
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn prepare(&self, values: &CommandValues) -> Result<ValidatedOperation> {
        self.validate()?;
        let input = capability(&self.operation)
            .expect("validated operation")
            .input_schema();
        let mut args = self.fixed_arguments.clone();
        for key in values.keys() {
            if !self.parameters.iter().any(|p| &p.name == key) {
                return Err(Error::validation(key, "unknown command argument"));
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
    /// Check registration names and ownership without reserving routes or executing actions.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{BotHandle, Result, capabilities::OperationCommandBinding};
    /// async fn assess(handle: &BotHandle, binding: &OperationCommandBinding) -> Result<()> {
    ///     let plan = binding.preflight(handle, None).await?;
    ///     println!("{:?}", plan.routes);
    ///     Ok(())
    /// }
    /// ```
    pub async fn preflight(
        &self,
        handle: &crate::BotHandle,
        replacing_cog: Option<&str>,
    ) -> Result<crate::RegistrationPlan> {
        handle
            .preflight_commands(&[self.command_schema()?.info()], replacing_cog)
            .await
    }
    /// Create a normal installed command binding; handler-owned authorization remains explicit.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::OperationCommandBinding;
    /// let binding: OperationCommandBinding = serde_json::from_value(serde_json::json!({
    ///     "operation":"send_message", "name":"announce", "description":"Post an announcement",
    ///     "fixed_arguments":{"channel":uuid::Uuid::nil()},
    ///     "parameters":[{"name":"text", "target":"/content", "required":true, "rest":true}]
    /// }))?;
    /// binding.validate()?;
    /// let schema = binding.command_schema()?;
    /// assert_eq!(schema.info().usage(), "/announce <text>");
    /// let operation = binding.prepare(&schema.parse("Hello everyone")?)?;
    /// assert_eq!(operation.arguments()["content"], "Hello everyone");
    /// let registered = binding.bind(|ctx, operation| async move {
    ///     // Assess and execute with a host-derived OperationContext and policy here.
    ///     ctx.reply(format!("Prepared {}", operation.name())).await?; Ok(())
    /// })?;
    /// # Ok::<(), rappin::Error>(())
    /// ```
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
                    Error::Validation { field, message } => {
                        Error::Command(crate::CommandError::Invalid {
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
        return Err(Error::validation(
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
                    return Err(Error::validation("target", "invalid JSON pointer escape"));
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
                Error::validation(
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
            return Err(Error::validation(
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
