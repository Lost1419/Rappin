//! Slash publication and option parsing with caller-owned command policy.

use crate::{Command, CommandError, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
/// Supported Yapin command option types.
///
/// ```
/// assert_eq!(serde_json::to_value(rappin::SlashOptionKind::User).unwrap(), "user");
/// ```
pub enum SlashOptionKind {
    /// Text.
    #[default]
    String,
    /// Whole number.
    Integer,
    /// Number, including decimals.
    Number,
    /// Boolean.
    Boolean,
    /// User mention or UUID.
    User,
    /// Channel mention or UUID.
    Channel,
    /// Role mention or UUID.
    Role,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// A displayed choice and the value delivered to the handler.
///
/// ```
/// let choice: rappin::SlashChoice = serde_json::from_str(r#"{"name":"Seven","value":7}"#).unwrap();
/// assert_eq!(choice.value, 7);
/// ```
pub struct SlashChoice {
    /// Choice label.
    pub name: String,
    /// String or numeric choice value.
    pub value: Value,
}

impl SlashChoice {
    /// Construct a choice. Publication validates its type against the option.
    ///
    /// ```
    /// let choice = rappin::SlashChoice::new("Seven", 7);
    /// assert_eq!(choice.value, 7);
    /// ```
    pub fn new(name: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
/// A slash command option advertised to Yapin.
///
/// ```
/// let option = rappin::SlashOption::new("days", rappin::SlashOptionKind::Integer);
/// assert_eq!(option.name, "days");
/// ```
pub struct SlashOption {
    /// Unique lowercase name.
    pub name: String,
    /// Optional help text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Value type.
    #[serde(rename = "type", default)]
    pub kind: SlashOptionKind,
    /// Whether the caller must supply the option.
    #[serde(default)]
    pub required: bool,
    /// Allowed values. Empty omits the choice restriction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<SlashChoice>,
}

impl SlashOption {
    /// Create an optional option with no choices.
    ///
    /// ```
    /// assert!(!rappin::SlashOption::new("reason", rappin::SlashOptionKind::String).required);
    /// ```
    pub fn new(name: impl Into<String>, kind: SlashOptionKind) -> Self {
        Self {
            name: name.into(),
            description: None,
            kind,
            required: false,
            choices: Vec::new(),
        }
    }
}

impl Command {
    /// Parse positional input or an entire form submission of named pairs.
    /// Authorization and handler dispatch remain with the caller.
    ///
    /// ```
    /// use rappin::{Command, SlashOption, SlashOptionKind};
    /// let command = Command::new("ban", "").options([
    ///     SlashOption::new("reason", SlashOptionKind::String),
    ///     SlashOption::new("days", SlashOptionKind::Integer),
    /// ]);
    /// let args = command.parse_options(r#"days=7 reason="spam and ads""#).unwrap();
    /// assert_eq!(args["reason"], "spam and ads");
    /// ```
    pub fn parse_options(&self, args: &str) -> Result<BTreeMap<String, Value>> {
        parse_options(&self.options, args)
    }
}

pub(crate) fn parse_options(
    options: &[SlashOption],
    args: &str,
) -> Result<BTreeMap<String, Value>> {
    validate_options(options)?;
    let tokens = crate::commands::tokenize(args)?;
    let named = !tokens.is_empty()
        && tokens.iter().all(|token| {
            token
                .value
                .split_once('=')
                .is_some_and(|(key, _)| valid_name(key))
                && args[token.start..token.end]
                    .split_once('=')
                    .is_some_and(|(key, _)| valid_name(key))
        });
    let mut values = BTreeMap::new();
    if named {
        for token in &tokens {
            let (key, raw) = token.value.split_once('=').expect("named token");
            let option = options
                .iter()
                .find(|option| option.name == key)
                .ok_or_else(|| CommandError::Invalid {
                    name: "options".into(),
                    reason: "unknown named option".into(),
                })?;
            if values
                .insert(key.into(), parse_value(option, raw)?)
                .is_some()
            {
                return Err(CommandError::Invalid {
                    name: option.name.clone(),
                    reason: "duplicate named option".into(),
                }
                .into());
            }
        }
    } else {
        let mut remaining = tokens.iter();
        for (index, option) in options.iter().enumerate() {
            let Some(token) = remaining.next() else { break };
            let value = if index + 1 == options.len() && option.kind == SlashOptionKind::String {
                std::iter::once(token)
                    .chain(remaining.by_ref())
                    .map(|part| part.value.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                token.value.clone()
            };
            values.insert(option.name.clone(), parse_value(option, &value)?);
        }
        if remaining.next().is_some() {
            return Err(CommandError::Extra.into());
        }
    }
    if let Some(option) = options
        .iter()
        .find(|option| option.required && !values.contains_key(&option.name))
    {
        return Err(CommandError::Missing(option.name.clone()).into());
    }
    Ok(values)
}

fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}

pub(crate) fn validate_options(options: &[SlashOption]) -> Result<()> {
    if options.len() > 10 {
        return Err(Error::validation("options", "maximum 10 options"));
    }
    let mut names = HashSet::new();
    let mut optional_seen = false;
    for option in options {
        if !valid_name(&option.name) || !names.insert(&option.name) {
            return Err(Error::validation(
                "options.name",
                "expected a unique lowercase option name",
            ));
        }
        if option.required && optional_seen {
            return Err(Error::validation(
                "options.required",
                "required options must precede optional options",
            ));
        }
        optional_seen |= !option.required;
        if option
            .description
            .as_ref()
            .is_some_and(|description| description.chars().count() > 100)
        {
            return Err(Error::validation(
                "options.description",
                "maximum 100 characters",
            ));
        }
        if option.choices.len() > 25
            || option.choices.iter().any(|choice| {
                choice.name.is_empty()
                    || choice.name.chars().count() > 50
                    || !match option.kind {
                        SlashOptionKind::String => choice
                            .value
                            .as_str()
                            .is_some_and(|s| !s.is_empty() && !s.chars().any(char::is_whitespace)),
                        SlashOptionKind::Integer | SlashOptionKind::Number => {
                            choice.value.as_i64().is_some() || choice.value.as_u64().is_some()
                        }
                        _ => false,
                    }
            })
        {
            return Err(Error::validation(
                "options.choices",
                "expected up to 25 choices of the declared type",
            ));
        }
    }
    Ok(())
}

fn parse_value(option: &SlashOption, raw: &str) -> Result<Value> {
    let invalid = || CommandError::Invalid {
        name: option.name.clone(),
        reason: "value does not match its declared type or choices".into(),
    };
    let value = match option.kind {
        SlashOptionKind::String => Value::String(raw.into()),
        SlashOptionKind::Integer => raw.parse::<i64>().map(Value::from).map_err(|_| invalid())?,
        SlashOptionKind::Number => {
            if let Ok(integer) = raw.parse::<i64>() {
                Value::from(integer)
            } else {
                let number = raw.parse::<f64>().map_err(|_| invalid())?;
                Value::Number(serde_json::Number::from_f64(number).ok_or_else(invalid)?)
            }
        }
        SlashOptionKind::Boolean => match raw.to_ascii_lowercase().as_str() {
            "yes" | "true" => Value::Bool(true),
            "no" | "false" => Value::Bool(false),
            _ => return Err(invalid().into()),
        },
        SlashOptionKind::User | SlashOptionKind::Channel | SlashOptionKind::Role => {
            let prefix = match option.kind {
                SlashOptionKind::User => "<@",
                SlashOptionKind::Channel => "<#",
                SlashOptionKind::Role => "<@&",
                _ => unreachable!(),
            };
            let id = raw
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix('>'))
                .unwrap_or(raw);
            let id = if option.kind == SlashOptionKind::User {
                id.strip_prefix('!').unwrap_or(id)
            } else {
                id
            };
            json!(id.parse::<uuid::Uuid>().map_err(|_| invalid())?)
        }
    };
    if !option.choices.is_empty()
        && !option.choices.iter().any(|choice| {
            choice.value == value
                || option.kind == SlashOptionKind::Number && choice.value.as_f64() == value.as_f64()
        })
    {
        return Err(invalid().into());
    }
    Ok(value)
}
