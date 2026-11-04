use super::*;
use crate::{
    CreateCategoryRequest, CreateChannelRequest, CreateMessage, CreateRole, CreateServerEvent,
    EditCategoryRequest, EditChannelRequest, EditRole, PermissionOverride, Permissions,
};

pub(super) fn validate(descriptor: &Capability, args: &Value) -> Result<ValidatedOperation> {
    validate_inner(descriptor, args).map_err(|error| match error {
        YappinError::Json(_) => {
            YappinError::validation("arguments", "payload does not match the typed SDK request")
        }
        other => other,
    })
}
fn validate_inner(descriptor: &Capability, args: &Value) -> Result<ValidatedOperation> {
    let entry = capability(descriptor.name)
        .ok_or_else(|| YappinError::validation("operation", "unknown operation"))?;
    if serde_json::to_vec(args)?.len() > 131072 {
        return Err(YappinError::validation(
            "arguments",
            "maximum inline arguments are 128 KiB",
        ));
    }
    let mut args = args.clone();
    if matches!(
        entry.name,
        "send_message" | "edit_message" | "send_files" | "post_bot_webhook"
    ) {
        if let Some(body) = args.get_mut("body") {
            crate::utils::strip_proxy_metadata(body);
        }
    }
    validate_schema(schema::operation_input_schema(entry), &args).map_err(|error| match error {
        YappinError::Validation { field, message } => {
            YappinError::validation(field.strip_prefix("arguments.").unwrap_or(&field), message)
        }
        other => other,
    })?;
    let mut writes = vec![];
    match entry.name {
        "send_message" => {
            if args.get("body").is_some() {
                let mut message: crate::UiMessageBody =
                    serde_json::from_value(args["body"].clone())?;
                if let Some(reply) = args["reply_to"].as_str() {
                    message.reply_to = Some(
                        reply
                            .parse()
                            .map_err(|_| YappinError::validation("reply_to", "invalid UUID"))?,
                    );
                }
                message.validate(true)?;
                writes.push(serde_json::to_value(message)?);
            } else {
                let mut message =
                    CreateMessage::new(args["content"].as_str().expect("validated content"));
                if let Some(reply) = args["reply_to"].as_str() {
                    message.reply_to = Some(
                        reply
                            .parse()
                            .map_err(|_| YappinError::validation("reply_to", "invalid UUID"))?,
                    );
                }
                let messages = if args["split"] == true {
                    crate::split_message(message.content.as_deref().unwrap_or(""))
                        .into_iter()
                        .map(|text| {
                            let mut part = CreateMessage::new(text);
                            part.reply_to = message.reply_to;
                            part
                        })
                        .collect()
                } else {
                    vec![message]
                };
                if messages.is_empty() {
                    return Err(YappinError::validation(
                        "content",
                        "message cannot be empty",
                    ));
                }
                for message in messages {
                    message.validate()?;
                    writes.push(serde_json::to_value(message)?);
                }
            }
        }
        "edit_message" => {
            let body: crate::UiMessageBody = serde_json::from_value(args["body"].clone())?;
            body.validate(false)?;
            args["body"] = serde_json::to_value(body)?;
        }
        "create_channel" => {
            let body: CreateChannelRequest = serde_json::from_value(args["body"].clone())?;
            args["body"] = serde_json::to_value(body)?;
        }
        "edit_channel" => {
            let body: EditChannelRequest = serde_json::from_value(args["body"].clone())?;
            body.validate()?;
            args["body"] = serde_json::to_value(body)?;
        }
        "create_category" => {
            let body: CreateCategoryRequest = serde_json::from_value(args["body"].clone())?;
            args["body"] = serde_json::to_value(body)?;
        }
        "edit_category" => {
            let body: EditCategoryRequest = serde_json::from_value(args["body"].clone())?;
            args["body"] = serde_json::to_value(body)?;
        }
        "create_role" => {
            let body: CreateRole = serde_json::from_value(args["body"].clone())?;
            args["body"] = serde_json::to_value(body)?;
        }
        "edit_role" => {
            let body: EditRole = serde_json::from_value(args["body"].clone())?;
            args["body"] = serde_json::to_value(body)?;
        }
        "set_everyone_permissions" => {
            let body: Permissions = serde_json::from_value(args["body"]["permissions"].clone())?;
            args["body"] = json!({"permissions":body});
        }
        "set_channel_permissions" | "set_category_permissions" => {
            let body: PermissionOverride = serde_json::from_value(args["body"].clone())?;
            args["body"] = serde_json::to_value(body)?;
        }
        "create_server_event" => {
            let body: CreateServerEvent = serde_json::from_value(args["body"].clone())?;
            body.validate()?;
            args["body"] = serde_json::to_value(body)?;
        }
        "send_files" => {
            platform::files(&args["files"])?;
            let body: CreateMessage =
                serde_json::from_value(args.get("body").cloned().unwrap_or_else(|| json!({})))?;
            crate::validation::validate_message(&serde_json::to_value(&body)?, false)?;
            args["body"] = serde_json::to_value(body)?;
            writes.push(args["body"].clone());
        }
        "create_emoji" => {
            let body: crate::CreateEmoji = serde_json::from_value(args["body"].clone())?;
            body.validate()?;
            args["body"] = serde_json::to_value(body)?;
        }
        "edit_emoji" => {
            crate::validate_emoji_name(args["body"]["name"].as_str().unwrap_or_default())?;
        }
        "create_bot_webhook" => {
            let body: crate::CreateBotWebhook = serde_json::from_value(args["body"].clone())?;
            body.validate()?;
            args["body"] = serde_json::to_value(body)?;
        }
        "post_bot_webhook" => {
            let body: crate::webhook::WebhookMessage =
                serde_json::from_value(args["body"].clone())?;
            crate::validation::validate_webhook(&serde_json::to_value(&body)?)?;
            args["body"] = serde_json::to_value(body)?;
        }
        "get_schema" => {
            if args.get("name").is_some()
                && (args.get("path").is_some() || args.get("method").is_some())
            {
                return Err(YappinError::validation(
                    "schema",
                    "choose a name or a method/path, not both",
                ));
            }
            if let Some(name) = args["name"].as_str() {
                if bundled_schema()["components"]["schemas"]
                    .get(name)
                    .is_none()
                {
                    return Err(YappinError::validation("name", "unknown component schema"));
                }
            } else {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| YappinError::validation("path", "required with method"))?;
                let method = args["method"]
                    .as_str()
                    .ok_or_else(|| YappinError::validation("method", "required with path"))?;
                endpoint_schema(bundled_schema(), path, method)?;
            }
        }
        _ => (),
    }
    if let Some(target) = args["target"].as_str() {
        target
            .parse::<crate::PermissionTarget>()
            .map_err(|_| YappinError::validation("target", "expected a role UUID or everyone"))?;
    }
    let limits = output::Limits {
        fields: args["fields"]
            .as_str()
            .map(|s| s.split(',').map(str::to_owned).collect())
            .unwrap_or_default(),
        max_items: args["max_items"].as_u64().map(|n| n as usize),
        max_bytes: args["max_bytes"].as_u64().map(|n| n as usize),
        offset: args["offset"].as_u64().map(|n| n as usize),
    };
    output::validate(&limits, collection_key(entry.name))?;
    Ok(ValidatedOperation {
        entry,
        args,
        writes,
        limits,
    })
}
pub(super) fn collection_key(name: &str) -> Option<&'static str> {
    match name {
        "list_servers" => Some("servers"),
        "list_channels" => Some("channels"),
        "list_members" => Some("members"),
        "read_messages" => Some("messages"),
        "search_channels" | "search_members" => Some("matches"),
        "list_emojis" | "get_message_emojis" => Some("emojis"),
        "list_channel_webhooks" | "list_bot_webhooks" => Some("webhooks"),
        "list_roles" => Some("roles"),
        "list_invites" => Some("invites"),
        "list_server_events" => Some("events"),
        "get_channel_permissions" => Some("overrides"),
        _ => None,
    }
}
