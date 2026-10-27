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
            if children.is_empty() || children.len() > 5 || children.iter().any(|c| kind(c) != 2) {
                return Err(invalid(path, "row requires 1 to 5 buttons"));
            }
        }
        2 => {
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
    pub fn validate(&self) -> Result<()> {
        validate_message(&serde_json::to_value(self)?, true)
    }
}
impl crate::EditMessage {
    pub fn validate(&self) -> Result<()> {
        validate_message(&serde_json::to_value(self)?, false)
    }
}
impl crate::InteractionCallback {
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
