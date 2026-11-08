//! Local validation shared by typed requests and agent tooling.
use crate::{Result, YappinError};
use serde_json::Value;
use std::collections::HashSet;

fn invalid(field: &str, message: &str) -> YappinError {
    YappinError::validation(field, message)
}
fn text(value: &Value, path: &str, max: usize) -> Result<usize> {
    let n = value.as_str().map_or(0, |s| s.chars().count());
    if n > max {
        return Err(invalid(path, &format!("maximum {max} characters")));
    }
    Ok(n)
}
fn embeds(value: &Value, max_embeds: usize, max_fields: usize) -> Result<()> {
    let Some(items) = value.as_array() else {
        return Ok(());
    };
    if items.len() > max_embeds {
        return Err(invalid("embeds", &format!("maximum {max_embeds} embeds")));
    }
    let mut total = 0;
    for (i, e) in items.iter().enumerate() {
        let p = format!("embeds[{i}]");
        for (key, max) in [("title", 256), ("description", 4096)] {
            total += text(&e[key], &format!("{p}.{key}"), max)?;
        }
        total += text(&e["footer"]["text"], &format!("{p}.footer.text"), 2048)?;
        total += text(&e["author"]["name"], &format!("{p}.author.name"), 256)?;
        if let Some(fields) = e["fields"].as_array() {
            if fields.len() > max_fields {
                return Err(invalid(
                    &format!("{p}.fields"),
                    &format!("maximum {max_fields} fields"),
                ));
            }
            for (j, f) in fields.iter().enumerate() {
                total += text(&f["name"], &format!("{p}.fields[{j}].name"), 256)?;
                total += text(&f["value"], &format!("{p}.fields[{j}].value"), 1024)?;
            }
        }
        if total > 6000 {
            return Err(invalid("embeds", "maximum 6000 combined characters"));
        }
    }
    Ok(())
}
fn kind(v: &Value) -> i64 {
    match v["type"].as_str() {
        Some("row") => 1,
        Some("button") => 2,
        Some("select") => 3,
        Some("text_input") => 4,
        _ => v["type"].as_i64().unwrap_or(-1),
    }
}
fn component(
    v: &Value,
    path: &str,
    parent: i64,
    ids: &mut HashSet<String>,
    count: &mut usize,
    chars: &mut usize,
) -> Result<()> {
    *count += 1;
    let k = kind(v);
    match k {
        1 => {
            let children = v["components"]
                .as_array()
                .ok_or_else(|| invalid(path, "row requires buttons"))?;
            if !(children.len() == 1 && kind(&children[0]) == 3)
                && (children.is_empty()
                    || children.len() > 5
                    || children.iter().any(|c| kind(c) != 2))
            {
                return Err(invalid(
                    path,
                    "row requires 1 to 5 buttons or one select menu",
                ));
            }
        }
        2 => {
            validate_component_emoji(&v["emoji"], path)?;
            if parent != 1 && parent != 9 {
                return Err(invalid(path, "button must be in a row or section"));
            }
            text(&v["label"], &format!("{path}.label"), 80)?;
            if v["label"].as_str().unwrap_or("").is_empty() && v["emoji"].is_null() {
                return Err(invalid(path, "button requires a label or emoji"));
            }
            let link = v["style"] == "link" || v["style"] == 5;
            if link {
                if !v["url"]
                    .as_str()
                    .is_some_and(|s| url::Url::parse(s).is_ok_and(|u| u.scheme() == "https"))
                    || !v["custom_id"].is_null()
                {
                    return Err(invalid(path, "link requires an HTTPS URL and no custom_id"));
                }
            } else {
                let id = v["custom_id"].as_str().unwrap_or("");
                if id.is_empty()
                    || id.chars().count() > 100
                    || !ids.insert(id.into())
                    || !v["url"].is_null()
                {
                    return Err(invalid(
                        path,
                        "requires a unique custom_id (1 to 100 characters) and no URL",
                    ));
                }
                if !matches!(
                    v["style"].as_str(),
                    Some("primary" | "secondary" | "success" | "danger")
                ) && !v["style"].as_i64().is_some_and(|n| (1..=4).contains(&n))
                {
                    return Err(invalid(path, "invalid button style"));
                }
            }
        }
        3 => {
            if parent != 1 {
                return Err(invalid(path, "select must be alone in a row"));
            }
            select(v, path, ids)?;
        }
        9 => {
            let children = v["components"]
                .as_array()
                .ok_or_else(|| invalid(path, "section requires text displays"))?;
            if children.is_empty() || children.len() > 3 || children.iter().any(|c| kind(c) != 10) {
                return Err(invalid(path, "section requires 1 to 3 text displays"));
            }
            if !matches!(kind(&v["accessory"]), 2 | 11) {
                return Err(invalid(
                    path,
                    "section requires a button or thumbnail accessory",
                ));
            }
        }
        10 => {
            *chars += text(&v["content"], &format!("{path}.content"), 4000)?;
        }
        11 if parent != 9 => return Err(invalid(path, "thumbnail must be a section accessory")),
        11 => (),
        12 => {
            if !v["items"]
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.len() <= 10)
            {
                return Err(invalid(path, "gallery requires 1 to 10 items"));
            }
        }
        13 => (),
        14 => {
            if !v["spacing"].is_null() && v["spacing"] != 1 && v["spacing"] != 2 {
                return Err(invalid(path, "separator spacing must be 1 or 2"));
            }
        }
        17 if parent != 0 => return Err(invalid(path, "containers cannot be nested")),
        17 => (),
        _ => return Err(invalid(path, "unknown component type")),
    }
    if let Some(children) = v["components"].as_array() {
        for (i, c) in children.iter().enumerate() {
            component(c, &format!("{path}.components[{i}]"), k, ids, count, chars)?;
        }
    }
    if !v["accessory"].is_null() {
        component(
            &v["accessory"],
            &format!("{path}.accessory"),
            k,
            ids,
            count,
            chars,
        )?;
    }
    Ok(())
}
/// Validates the bot message or callback limits without making a network request.
/// # Example
///
/// ```rust
/// use rappin::validation::validate_message;
/// validate_message(&serde_json::json!({"content":"Hello"}), true)?;
/// assert!(validate_message(&serde_json::json!({"content":"x".repeat(2001)}), true).is_err());
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub fn validate_message(value: &Value, creating: bool) -> Result<()> {
    text(&value["content"], "content", 2000)?;
    embeds(&value["embeds"], 5, 10)?;
    let v2 = value["flags"].as_u64().unwrap_or(0) & 32768 != 0;
    if v2 && (!value["content"].is_null() || !value["embeds"].is_null()) {
        return Err(invalid(
            "flags",
            "V2 layouts cannot include content or embeds",
        ));
    }
    let mut count = 0;
    let mut chars = 0;
    let mut ids = HashSet::new();
    if let Some(parts) = value["components"].as_array() {
        if !v2 && (parts.len() > 5 || parts.iter().any(|c| kind(c) != 1)) {
            return Err(invalid(
                "components",
                "classic messages support up to 5 action rows",
            ));
        }
        for (i, c) in parts.iter().enumerate() {
            component(
                c,
                &format!("components[{i}]"),
                0,
                &mut ids,
                &mut count,
                &mut chars,
            )?;
        }
    }
    if count > 40 || chars > 4000 {
        return Err(invalid(
            "components",
            "maximum 40 components and 4000 text-display characters",
        ));
    }
    if creating
        && value["content"].as_str().unwrap_or("").is_empty()
        && value["embeds"].as_array().is_none_or(|v| v.is_empty())
        && !(v2 && count > 0)
    {
        return Err(invalid(
            "message",
            "requires content, embeds, or a V2 layout",
        ));
    }
    Ok(())
}

impl crate::CreateMessage {
    /// Check typed payload bounds before an API call.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::CreateMessage::new("Hello");
    /// body.validate()?;
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        validate_message(&serde_json::to_value(self)?, true)
    }
}
impl crate::EditMessage {
    /// Check typed payload bounds before an API call.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::EditMessage::new().content("Updated");
    /// body.validate()?;
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        validate_message(&serde_json::to_value(self)?, false)
    }
}
impl crate::InteractionCallback {
    /// Check typed payload bounds before an API call.
    ///
    /// # Example
    ///
    /// ```rust
    /// let body = rappin::InteractionCallback::ephemeral("Received");
    /// body.validate()?;
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        use crate::InteractionCallbackType::*;
        if self.kind == Ack {
            return Ok(());
        }
        if self.kind == Ephemeral
            && (self.flags.is_some_and(|v| v & 32768 != 0) || self.components.is_some())
        {
            return Err(invalid(
                "components",
                "ephemeral replies do not support components",
            ));
        }
        validate_message(&serde_json::to_value(self)?, self.kind != Update)
    }
}
/// Split long output on Unicode character boundaries, preferring newlines or spaces.
/// # Example
///
/// ```rust
/// let text = "a".repeat(2500);
/// let parts = rappin::split_message(&text);
/// assert_eq!(parts.concat(), text);
/// assert!(parts.iter().all(|part| part.chars().count() <= 2000));
/// ```
pub fn split_message(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = content;
    while rest.chars().count() > 2000 {
        let end = rest.char_indices().nth(2000).map(|(i, _)| i).unwrap();
        let chunk = &rest[..end];
        let cut = chunk
            .rfind('\n')
            .or_else(|| chunk.rfind(' '))
            .filter(|i| *i > 0)
            .map_or(end, |i| i + 1);
        result.push(rest[..cut].into());
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        result.push(rest.into());
    }
    result
}

/// Channel webhooks have different limits and only accept link buttons.
/// # Example
///
/// ```rust
/// rappin::validation::validate_webhook(&serde_json::json!({"content":"Build complete"}))?;
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub fn validate_webhook(value: &Value) -> Result<()> {
    text(&value["content"], "content", 2000)?;
    text(&value["username"], "username", 80)?;
    embeds(&value["embeds"], 10, 25)?;
    if value["content"].as_str().unwrap_or("").is_empty()
        && value["embeds"].as_array().is_none_or(|a| a.is_empty())
    {
        return Err(invalid("message", "webhook requires content or embeds"));
    }
    if let Some(rows) = value["components"].as_array() {
        if rows.len() > 5 {
            return Err(invalid("components", "maximum 5 rows"));
        }
        let mut count = 0;
        let mut chars = 0;
        let mut ids = HashSet::new();
        for (i, row) in rows.iter().enumerate() {
            if kind(row) != 1 {
                return Err(invalid("components", "webhooks require link-button rows"));
            }
            if row["components"].as_array().is_some_and(|buttons| {
                buttons
                    .iter()
                    .any(|b| b["style"] != "link" && b["style"] != 5)
            }) {
                return Err(invalid("components", "webhooks only support link buttons"));
            }
            component(
                row,
                &format!("components[{i}]"),
                0,
                &mut ids,
                &mut count,
                &mut chars,
            )?;
        }
    }
    Ok(())
}

/// Validate command metadata before changing a running registry.
pub(crate) fn validate_commands(commands: &[crate::Command]) -> Result<()> {
    if commands.len() > 100 {
        return Err(YappinError::validation("commands", "maximum 100 commands"));
    }
    let mut names = std::collections::HashSet::new();
    for command in commands {
        if command.name.is_empty()
            || command.name.len() > 32
            || !command
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
            || !names.insert(&command.name)
        {
            return Err(YappinError::validation(
                "commands.name",
                "requires a unique lowercase name of 1 to 32 letters, digits, _ or -",
            ));
        }
        if command.description.chars().count() > 100 {
            return Err(YappinError::validation(
                "commands.description",
                "maximum 100 characters",
            ));
        }
    }
    Ok(())
}

fn optional_text(value: &Value, path: &str, max: usize) -> Result<()> {
    if !value.is_null() && !value.is_string() {
        return Err(invalid(path, "string required"));
    }
    text(value, path, max)?;
    Ok(())
}
fn optional_bool(value: &Value, path: &str) -> Result<()> {
    if !value.is_null() && !value.is_boolean() {
        return Err(invalid(path, "boolean required"));
    }
    Ok(())
}
fn required_text(value: &Value, path: &str, max: usize) -> Result<()> {
    if !value.as_str().is_some_and(|s| !s.is_empty()) {
        return Err(invalid(path, "nonempty string required"));
    }
    text(value, path, max)?;
    Ok(())
}
fn custom_id(value: &Value, path: &str, ids: &mut HashSet<String>) -> Result<()> {
    required_text(value, path, 100)?;
    if !ids.insert(value.as_str().unwrap().into()) {
        return Err(invalid(path, "custom_id must be unique"));
    }
    Ok(())
}
fn validate_component_emoji(value: &Value, path: &str) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    if value.is_string() {
        return required_text(value, path, 80);
    }
    if !value.is_object()
        || value["id"]
            .as_str()
            .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err(invalid(path, "emoji requires text or a custom emoji UUID"));
    }
    required_text(&value["name"], path, 80)?;
    if !value["animated"].is_null() && !value["animated"].is_boolean() {
        return Err(invalid(path, "animated must be boolean"));
    }
    Ok(())
}
fn select(value: &Value, path: &str, ids: &mut HashSet<String>) -> Result<()> {
    custom_id(&value["custom_id"], path, ids)?;
    optional_text(&value["placeholder"], path, 150)?;
    optional_bool(&value["disabled"], path)?;
    let options = value["options"]
        .as_array()
        .ok_or_else(|| invalid(path, "select requires options"))?;
    if options.is_empty() || options.len() > 25 {
        return Err(invalid(path, "select requires 1 to 25 options"));
    }
    let minimum = if value["min_values"].is_null() {
        1
    } else {
        value["min_values"]
            .as_u64()
            .ok_or_else(|| invalid(path, "invalid min_values"))?
    };
    let maximum = if value["max_values"].is_null() {
        1
    } else {
        value["max_values"]
            .as_u64()
            .ok_or_else(|| invalid(path, "invalid max_values"))?
    };
    if minimum > 25
        || maximum == 0
        || maximum > 25
        || minimum > maximum
        || maximum > options.len() as u64
    {
        return Err(invalid(path, "invalid select bounds"));
    }
    let mut values = HashSet::new();
    let mut defaults = 0;
    for option in options {
        required_text(&option["label"], path, 100)?;
        required_text(&option["value"], path, 100)?;
        optional_text(&option["description"], path, 100)?;
        optional_bool(&option["default"], path)?;
        validate_component_emoji(&option["emoji"], path)?;
        if !values.insert(option["value"].as_str().unwrap()) {
            return Err(invalid(path, "option values must be unique"));
        }
        if option["default"] == true {
            defaults += 1;
        }
    }
    if defaults > maximum {
        return Err(invalid(path, "too many default selections"));
    }
    Ok(())
}
/// Validate normalized or documented Discord-wrapper modal inputs offline.
/// # Example
///
/// ```rust
/// let modal = rappin::Modal::new("feedback", "Feedback").input(rappin::TextInput::paragraph("comment", "Comment"));
/// rappin::validation::validate_modal(&serde_json::to_value(modal)?)?;
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub fn validate_modal(value: &Value) -> Result<()> {
    required_text(&value["custom_id"], "modal.custom_id", 100)?;
    required_text(&value["title"], "modal.title", 45)?;
    let inputs = value["components"]
        .as_array()
        .ok_or_else(|| invalid("modal.components", "inputs required"))?;
    if inputs.is_empty() || inputs.len() > 5 {
        return Err(invalid("modal.components", "requires 1 to 5 inputs"));
    }
    let mut ids = HashSet::new();
    for (index, wrapped) in inputs.iter().enumerate() {
        let path = format!("modal.components[{index}]");
        let mut value = wrapped.clone();
        if kind(wrapped) == 1 {
            let row = wrapped["components"]
                .as_array()
                .filter(|r| r.len() == 1)
                .ok_or_else(|| invalid(&path, "modal row requires one input"))?;
            value = row[0].clone();
        } else if wrapped["type"] == 18 {
            value = wrapped["component"].clone();
            for field in ["label", "description"] {
                if value[field].is_null() {
                    value[field] = wrapped[field].clone();
                }
            }
        }
        required_text(&value["label"], &path, 45)?;
        optional_text(&value["description"], &path, 100)?;
        optional_bool(&value["required"], &path)?;
        match kind(&value) {
            3 => select(&value, &path, &mut ids)?,
            4 => {
                custom_id(&value["custom_id"], &path, &mut ids)?;
                if !matches!(value["style"].as_str(), Some("short" | "paragraph"))
                    && !matches!(value["style"].as_u64(), Some(1 | 2))
                {
                    return Err(invalid(&path, "invalid text input style"));
                }
                let minimum = if value["min_length"].is_null() {
                    0
                } else {
                    value["min_length"]
                        .as_u64()
                        .ok_or_else(|| invalid(&path, "invalid min_length"))?
                };
                let maximum = if value["max_length"].is_null() {
                    4000
                } else {
                    value["max_length"]
                        .as_u64()
                        .ok_or_else(|| invalid(&path, "invalid max_length"))?
                };
                if minimum > 4000 || maximum == 0 || maximum > 4000 || minimum > maximum {
                    return Err(invalid(&path, "invalid text input bounds"));
                }
                optional_text(&value["placeholder"], &path, 100)?;
                optional_text(&value["value"], &path, maximum as usize)?;
            }
            _ => return Err(invalid(&path, "modal requires text inputs or select menus")),
        }
    }
    Ok(())
}
