use super::*;
/// Borrow the pinned offline OpenAPI contract; it need not match the latest live spec.
///
/// # Example
///
/// ```rust
/// let spec = rappin::capabilities::bundled_schema();
/// assert_eq!(spec["openapi"], "3.1.0");
/// ```
pub fn bundled_schema() -> &'static Value {
    static SPEC: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("../../docs/openapi.json")).expect("bundled OpenAPI JSON")
    });
    &SPEC
}

/// Immutable schemas belong to the canonical catalogue, not caller-owned copies
/// returned by Capability::input_schema. Initialize only operations that are used.
pub(super) fn operation_input_schema(capability: &'static Capability) -> &'static Value {
    use std::{
        collections::HashMap,
        sync::{LazyLock, OnceLock},
    };
    static SCHEMAS: LazyLock<HashMap<&'static str, OnceLock<Value>>> = LazyLock::new(|| {
        catalogue()
            .iter()
            .map(|entry| (entry.name, OnceLock::new()))
            .collect()
    });
    SCHEMAS
        .get(capability.name)
        .expect("canonical operation schema")
        .get_or_init(|| capability.input_schema())
}

pub(super) fn body_schema(name: &str) -> Option<Value> {
    let (method, path) = match name {
        "create_channel" => ("post", "/servers/{id}/channels"),
        "edit_channel" => ("patch", "/channels/{id}"),
        "create_category" => ("post", "/servers/{id}/categories"),
        "edit_category" => ("patch", "/servers/{id}/categories/{cid}"),
        "create_emoji" => ("post", "/servers/{id}/emojis"),
        "edit_emoji" => ("patch", "/servers/{id}/emojis/{eid}"),
        "create_bot_webhook" => ("post", "/channels/{id}/webhooks"),
        "post_bot_webhook" => ("post", "/webhooks/{id}"),
        "send_files" => ("post", "/channels/{id}/messages"),
        "send_message" => ("post", "/channels/{id}/messages"),
        "edit_message" => ("patch", "/channels/{id}/messages/{mid}"),
        "create_role" => ("post", "/servers/{id}/roles"),
        "edit_role" => ("patch", "/servers/{id}/roles/{rid}"),
        "set_everyone_permissions" => ("patch", "/servers/{id}/roles/everyone"),
        "set_channel_permissions" => ("put", "/channels/{id}/permissions/{role}"),
        "set_category_permissions" => ("put", "/servers/{id}/categories/{cid}/permissions/{role}"),
        "create_server_event" => ("post", "/servers/{id}/events"),
        _ => return None,
    };
    fn expand(value: &Value, depth: usize) -> Value {
        assert!(
            depth < 48,
            "recursive catalogue schema must use an explicit definition"
        );
        if let Some(reference) = value["$ref"].as_str() {
            return expand(
                bundled_schema()
                    .pointer(reference.strip_prefix('#').expect("local schema reference"))
                    .expect("bundled schema reference"),
                depth + 1,
            );
        }
        match value {
            Value::Array(a) => Value::Array(a.iter().map(|v| expand(v, depth + 1)).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, v)| (k.clone(), expand(v, depth + 1)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    let mut schema = expand(
        &bundled_schema()["paths"][path][method]["requestBody"]["content"]["application/json"]
            ["schema"],
        0,
    );
    // API 1.3 explicitly accepts emoji objects when sending, although its
    // normalized response schema describes emoji as text.
    fn sending_emojis(value: &mut Value) {
        if let Value::Object(object) = value {
            if let Some(kind) = object.get_mut("properties").and_then(|p| p.get_mut("type")) {
                let constant = kind["const"].clone();
                let alias = match (constant.as_i64(), constant.as_str()) {
                    (Some(1), _) => Some(json!("row")),
                    (Some(2), _) => Some(json!("button")),
                    (Some(3), _) => Some(json!("select")),
                    (_, Some("row")) => Some(json!(1)),
                    (_, Some("button")) => Some(json!(2)),
                    (_, Some("select")) => Some(json!(3)),
                    _ => None,
                };
                if let Some(alias) = alias {
                    *kind = json!({"anyOf":[{"const":constant},{"const":alias}]});
                }
            }
            // Component IDs are assigned upstream when omitted on writes.
            if object
                .get("properties")
                .is_some_and(|p| p.get("id").is_some() && p.get("type").is_some())
            {
                if let Some(Value::Array(required)) = object.get_mut("required") {
                    required.retain(|field| field != "id");
                }
            }
            if let Some(emoji) = object
                .get_mut("properties")
                .and_then(|p| p.get_mut("emoji"))
            {
                if emoji["type"] == "string" {
                    let text = emoji.clone();
                    *emoji = json!({"anyOf":[text,{"type":"object","properties":{"id":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":80},"animated":{"type":"boolean"}},"required":["id","name"],"additionalProperties":false}]});
                }
            }
            for child in object.values_mut() {
                sending_emojis(child);
            }
            fn component_choice(value: &Value) -> bool {
                value["properties"].get("type").is_some()
                    || ["oneOf", "anyOf"].iter().any(|key| {
                        value[*key].as_array().is_some_and(|items| {
                            !items.is_empty() && items.iter().all(component_choice)
                        })
                    })
            }
            // Classic and numeric V2 rows overlap once send aliases/default IDs
            // are admitted. Either documented form is valid, not exactly one.
            if object
                .get("oneOf")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty() && items.iter().all(component_choice))
            {
                let choices = object.remove("oneOf").expect("oneOf choices");
                object.insert("anyOf".into(), choices);
            }
        } else if let Value::Array(array) = value {
            for child in array {
                sending_emojis(child);
            }
        }
    }
    sending_emojis(&mut schema);
    schema["additionalProperties"] = false.into();
    // Fill gaps in the bundled OpenAPI using the existing typed SDK contract.
    match name {
        "edit_channel" => {
            schema["properties"]["category_id"]["format"] = "uuid".into();
            schema["properties"]["slowmode"]["minimum"] = 0.into();
        }
        "create_category" => {
            schema["properties"]["position"] =
                json!({"type":"integer","minimum":i32::MIN,"maximum":i32::MAX});
        }
        "edit_category" => schema["required"] = json!(["name"]),
        _ => (),
    }
    Some(schema)
}

/// Validate the JSON Schema subset used by this catalogue. This is not a
/// general-purpose validator for untrusted external schema dialects.
/// # Example
///
/// ```rust
/// use rappin::capabilities::validate_schema;
/// let schema = serde_json::json!({"type":"integer", "minimum":1, "maximum":100});
/// validate_schema(&schema, &serde_json::json!(20))?;
/// assert!(validate_schema(&schema, &serde_json::json!(101)).is_err());
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub fn validate_schema(schema: &Value, value: &Value) -> Result<()> {
    validate_at(schema, schema, value, "arguments", false, 0)
}
pub(super) fn validate_partial(schema: &Value, value: &Value) -> Result<()> {
    validate_at(schema, schema, value, "arguments", true, 0)
}
fn validate_at(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    partial: bool,
    depth: usize,
) -> Result<()> {
    let invalid = |why: &str| YappinError::validation(path, why);
    if depth > 64 {
        return Err(invalid("schema nesting exceeds 64"));
    }
    if schema == false {
        return Err(invalid("value is not permitted"));
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let target = reference
            .strip_prefix('#')
            .and_then(|p| root.pointer(p))
            .ok_or_else(|| invalid("unsupported schema reference"))?;
        validate_at(root, target, value, path, partial, depth + 1)?;
    }
    if let Some(expected) = schema.get("const") {
        if value != expected {
            return Err(invalid("unexpected constant"));
        }
    }
    if schema["enum"]
        .as_array()
        .is_some_and(|a| !a.contains(value))
    {
        return Err(invalid("value is not in the permitted choices"));
    }
    let matches_type = |name: &str| match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        _ => false,
    };
    if let Some(t) = schema.get("type") {
        let valid = if let Some(t) = t.as_str() {
            matches_type(t)
        } else {
            t.as_array()
                .is_some_and(|a| a.iter().any(|t| t.as_str().is_some_and(matches_type)))
        };
        if !valid {
            return Err(invalid("invalid argument type"));
        }
    }
    if let Some(n) = value.as_f64() {
        // Integer comparisons must retain u64 precision (event IDs can exceed 2^53).
        let below = schema["minimum"]
            .as_u64()
            .zip(value.as_u64())
            .map(|(min, n)| n < min)
            .unwrap_or_else(|| schema["minimum"].as_f64().is_some_and(|min| n < min));
        let above = schema["maximum"]
            .as_u64()
            .zip(value.as_u64())
            .map(|(max, n)| n > max)
            .unwrap_or_else(|| schema["maximum"].as_f64().is_some_and(|max| n > max));
        if below || above {
            return Err(invalid("argument out of bounds"));
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema["minLength"].as_u64().is_some_and(|n| length < n)
            || schema["maxLength"].as_u64().is_some_and(|n| length > n)
        {
            return Err(invalid("string length is out of bounds"));
        }
        match schema["format"].as_str() {
            Some("uuid") if uuid::Uuid::parse_str(text).is_err() => {
                return Err(invalid("invalid UUID"))
            }
            Some("date-time") if chrono::DateTime::parse_from_rfc3339(text).is_err() => {
                return Err(invalid("invalid RFC3339 timestamp"))
            }
            _ => (),
        }
    }
    if let Some(object) = value.as_object() {
        if !partial {
            if let Some(required) = schema["required"].as_array() {
                for key in required.iter().filter_map(Value::as_str) {
                    if !object.contains_key(key) {
                        return Err(YappinError::validation(
                            format!("{path}.{key}"),
                            "required argument",
                        ));
                    }
                }
            }
        }
        for (key, v) in object {
            let next = format!("{path}.{key}");
            if let Some(s) = schema["properties"].get(key) {
                validate_at(root, s, v, &next, partial, depth + 1)?;
            } else if let Some(additional) = schema.get("additionalProperties") {
                validate_at(root, additional, v, &next, partial, depth + 1)?;
            }
        }
    }
    if let Some(items) = value.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|n| items.len() < (n as usize))
            || schema["maxItems"]
                .as_u64()
                .is_some_and(|n| items.len() > (n as usize))
        {
            return Err(invalid("array length is out of bounds"));
        }
        if schema["uniqueItems"] == true
            && items
                .iter()
                .enumerate()
                .any(|(i, v)| items[..i].contains(v))
        {
            return Err(invalid("array items must be unique"));
        }
        if let Some(s) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                validate_at(root, s, item, &format!("{path}[{i}]"), partial, depth + 1)?;
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = schema[key].as_array() {
            if partial && key != "allOf" {
                continue;
            }
            let matches = branches
                .iter()
                .filter(|s| validate_at(root, s, value, path, partial, depth + 1).is_ok())
                .count();
            if (key == "allOf" && matches != branches.len())
                || (key == "anyOf" && matches == 0)
                || (key == "oneOf" && matches != 1)
            {
                return Err(invalid("arguments do not satisfy the schema alternatives"));
            }
        }
    }
    if !partial {
        if schema
            .get("not")
            .is_some_and(|s| validate_at(root, s, value, path, false, depth + 1).is_ok())
        {
            return Err(invalid("conflicting arguments"));
        }
        if let Some(condition) = schema.get("if") {
            let key = if validate_at(root, condition, value, path, false, depth + 1).is_ok() {
                "then"
            } else {
                "else"
            };
            if let Some(branch) = schema.get(key) {
                validate_at(root, branch, value, path, false, depth + 1)?;
            }
        }
    }
    Ok(())
}
/// Resolve one method/path operation from a supplied OpenAPI document.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{bundled_schema, endpoint_schema};
/// let operation = endpoint_schema(bundled_schema(), "/me", "get")?;
/// assert!(operation.is_object());
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub fn endpoint_schema(spec: &Value, path: &str, method: &str) -> Result<Value> {
    let method = method.to_ascii_lowercase();
    let item = spec["paths"]
        .get(path)
        .ok_or_else(|| YappinError::validation("path", "unknown API path template"))?;
    let mut operation = item
        .get(&method)
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| YappinError::validation("method", "unknown HTTP operation"))?;
    if let Some(parameters) = item.get("parameters") {
        operation["path_parameters"] = parameters.clone();
    }
    fn refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(o) => {
                if let Some(s) = o.get("$ref").and_then(Value::as_str) {
                    out.push(s.into());
                }
                for value in o.values() {
                    refs(value, out);
                }
            }
            Value::Array(a) => {
                for value in a {
                    refs(value, out);
                }
            }
            _ => (),
        }
    }
    let mut queue = vec![];
    refs(&operation, &mut queue);
    let mut seen = std::collections::HashSet::new();
    let mut components = json!({});
    while let Some(reference) = queue.pop() {
        if !seen.insert(reference.clone()) {
            continue;
        }
        let parts: Vec<_> = reference.split('/').collect();
        if parts.len() != 4 || parts[0] != "#" || parts[1] != "components" {
            return Err(YappinError::Config("unsupported schema reference".into()));
        }
        let value = spec
            .pointer(reference.trim_start_matches('#'))
            .ok_or_else(|| YappinError::Config("missing schema reference".into()))?;
        refs(value, &mut queue);
        components[parts[2]][parts[3]] = value.clone();
    }
    Ok(
        json!({"path":path,"method":method.to_uppercase(),"operation":operation,"components":components}),
    )
}
