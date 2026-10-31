//! Bound returned collections without advancing beyond data the caller received.
use super::*;
#[derive(Clone, Default)]
pub struct Limits {
    pub fields: Vec<String>,
    pub max_items: Option<usize>,
    pub max_bytes: Option<usize>,
    pub offset: Option<usize>,
}
impl Limits {
    pub fn active(&self) -> bool {
        !self.fields.is_empty()
            || self.max_items.is_some()
            || self.max_bytes.is_some()
            || self.offset.is_some()
    }
}
pub fn collection(command: &Command) -> Option<&'static str> {
    match command {
        Command::Channels {
            action: Channels::Search { .. },
        }
        | Command::Members {
            action: Members::Search { .. },
        } => Some("matches"),
        Command::Servers {
            action: Servers::Invites { .. },
        } => Some("invites"),
        Command::Servers {
            action: Servers::Roles { .. },
        } => Some("roles"),
        Command::Servers {
            action: Servers::ScheduledEvents { .. },
        } => Some("events"),
        Command::Channels {
            action: Channels::Permissions { .. },
        } => Some("overrides"),
        Command::Servers {
            action: Servers::List,
        } => Some("servers"),
        Command::Channels {
            action: Channels::List { .. },
        } => Some("channels"),
        Command::Members {
            action: Members::List { .. },
        } => Some("members"),
        Command::Messages {
            action: Messages::List { .. },
        } => Some("messages"),
        _ => None,
    }
}
pub fn validate(limits: &Limits, key: Option<&str>) -> Result<()> {
    if !limits.active() {
        return Ok(());
    }
    if key.is_none() {
        return Err(YappinError::validation(
            "output",
            "output limits apply only to collection reads",
        ));
    }
    if limits.offset.is_some() && matches!(key, Some("messages" | "members")) {
        return Err(YappinError::validation(
            "offset",
            "use API cursors for messages and members",
        ));
    }
    if limits
        .fields
        .iter()
        .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    {
        return Err(YappinError::validation(
            "fields",
            "use comma-separated top-level field names",
        ));
    }
    Ok(())
}
pub fn apply(mut data: Value, key: &str, limits: &Limits) -> Result<Value> {
    if !limits.active() {
        return Ok(data);
    }
    let items = data[key]
        .as_array_mut()
        .ok_or_else(|| YappinError::validation("output", "response is missing its collection"))?;
    let all = std::mem::take(items);
    let total = all.len();
    let offset = limits.offset.unwrap_or(0).min(total);
    let forward = data["direction"] == "after";
    let available = &all[offset..];
    let cap = limits
        .max_items
        .unwrap_or(available.len())
        .min(available.len());
    let minimum = if key == "messages" && !available.is_empty() {
        let anchor = if forward {
            &available[0]
        } else {
            &available[available.len() - 1]
        };
        if forward {
            available
                .iter()
                .take_while(|item| item["created_at"] == anchor["created_at"])
                .count()
        } else {
            available
                .iter()
                .rev()
                .take_while(|item| item["created_at"] == anchor["created_at"])
                .count()
        }
    } else {
        usize::from(!available.is_empty())
    };
    if cap < minimum {
        return Err(YappinError::validation(
            "max_items",
            "budget would split messages sharing a timestamp; increase max_items",
        ));
    }
    let make = |mut count: usize| {
        // Timestamp-only API cursors cannot continue inside a timestamp group.
        if key == "messages" && count > 0 && count < available.len() {
            if forward && available[count - 1]["created_at"] == available[count]["created_at"] {
                let boundary = available[count - 1]["created_at"].clone();
                while count > 0 && available[count - 1]["created_at"] == boundary {
                    count -= 1;
                }
            } else if !forward
                && available[available.len() - count - 1]["created_at"]
                    == available[available.len() - count]["created_at"]
            {
                let boundary = available[available.len() - count]["created_at"].clone();
                while count > 0 && available[available.len() - count]["created_at"] == boundary {
                    count -= 1;
                }
            }
        }
        let selected = if key == "messages" && !forward {
            &available[available.len() - count..]
        } else {
            &available[..count]
        };
        let mut out = data.clone();
        let cut = offset + count < total;
        let projected: Vec<Value> = selected
            .iter()
            .map(|item| {
                if limits.fields.is_empty() {
                    return item.clone();
                }
                let mut fields = serde_json::Map::new();
                for name in limits.fields.iter().map(String::as_str).chain([
                    "id",
                    "created_at",
                    "code",
                    "role_id",
                ]) {
                    if let Some(value) = item.get(name) {
                        fields.insert(name.into(), value.clone());
                    }
                }
                Value::Object(fields)
            })
            .collect();
        out[key] = json!(projected);
        out["truncated"] = cut.into();
        out["returned_count"] = count.into();
        if key == "messages" {
            let ids: std::collections::HashSet<_> = selected
                .iter()
                .filter_map(|item| item["author_id"].as_str())
                .collect();
            if let Some(authors) = out["authors"].as_array_mut() {
                authors.retain(|author| author["id"].as_str().is_some_and(|id| ids.contains(id)));
            }
            if cut {
                out["next_cursor"] = selected
                    .iter()
                    .filter_map(|item| item["created_at"].as_str())
                    .reduce(|a, b| {
                        if (forward && b > a) || (!forward && b < a) {
                            b
                        } else {
                            a
                        }
                    })
                    .map(Value::from)
                    .unwrap_or(Value::Null);
                out["may_have_more"] = true.into();
            }
        } else if key == "members" && cut {
            out["next_cursor"] = selected
                .last()
                .map(|item| item["id"].clone())
                .unwrap_or(Value::Null);
            out["may_have_more"] = true.into();
        } else if !matches!(key, "members" | "messages") {
            out["next_offset"] = if cut {
                json!(offset + count)
            } else {
                Value::Null
            };
            out["continuation_kind"] = "client_offset".into();
        }
        out
    };
    let fits = |value: &Value| -> bool {
        limits.max_bytes.is_none_or(|max| {
            serde_json::to_vec(&json!({"ok":true,"data":value}))
                .is_ok_and(|bytes| bytes.len() <= max)
        })
    };
    let candidate = make(cap);
    if fits(&candidate) {
        return Ok(candidate);
    }
    if cap == 0 || !fits(&make(minimum)) {
        return Err(YappinError::validation(
            "max_bytes",
            "budget cannot fit an item/timestamp group and continuation metadata; increase max_bytes",
        ));
    }
    let (mut low, mut high) = (minimum, cap);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(&make(mid)) {
            low = mid
        } else {
            high = mid - 1
        };
    }
    Ok(make(low))
}
