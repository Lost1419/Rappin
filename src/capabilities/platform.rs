use super::*;
use crate::{Api, UploadFile};
use base64::Engine;
use reqwest::Method;
pub(super) fn is_platform_operation(name: &str) -> bool {
    matches!(
        name,
        "resolve_id"
            | "get_member_permissions"
            | "get_channel_member_permissions"
            | "get_role"
            | "reorder_roles"
            | "get_channel_overrides"
            | "set_channel_override"
            | "delete_channel_override"
            | "get_category_overrides"
            | "set_category_override"
            | "delete_category_override"
            | "send_sticker"
            | "get_message_emojis"
            | "get_api_version"
            | "get_api_changelog"
            | "list_emojis"
            | "create_emoji"
            | "edit_emoji"
            | "delete_emoji"
            | "list_bot_webhooks"
            | "list_channel_webhooks"
            | "create_bot_webhook"
            | "delete_bot_webhook"
            | "post_bot_webhook"
            | "send_files"
    )
}

/// A validated single HTTP request, not an authorization grant. Raw Client/Api
/// callers own authorization; server-bound callers should execute the binding.
#[derive(Debug, Clone)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::capabilities::capability;
/// let operation = capability("get_api_version").unwrap().validate(&serde_json::json!({}))?;
/// let request = operation.api_request()?;
/// assert_eq!(request.path, "/version");
/// assert!(request.public);
/// # Ok::<(), rappin::Error>(())
/// ```
pub struct OperationRequest {
    /// HTTP method for this validated request.
    pub method: Method,
    /// Origin-relative API path.
    pub path: String,
    /// Optional canonical JSON body.
    pub body: Option<Value>,
    /// Validated in-memory upload buffers.
    pub files: Vec<UploadFile>,
    /// Whether dispatch uses the unauthenticated public metadata path.
    pub public: bool,
}
impl OperationRequest {
    pub(super) async fn dispatch<A: Api + ?Sized>(&self, api: &A) -> Result<Value> {
        if self.public {
            api.public_request(&self.path).await
        } else if !self.files.is_empty() {
            api.upload(
                &self.path,
                self.body.clone().unwrap_or_else(|| json!({})),
                &self.files,
            )
            .await
        } else {
            api.request(self.method.clone(), &self.path, self.body.clone())
                .await
        }
    }
}
pub(super) fn files(value: &Value) -> Result<Vec<UploadFile>> {
    let mut files = Vec::new();
    for file in value
        .as_array()
        .ok_or_else(|| Error::validation("files", "requires a file array"))?
    {
        let name = file["name"]
            .as_str()
            .ok_or_else(|| Error::validation("files.name", "filename required"))?;
        let encoded = file["data"]
            .as_str()
            .ok_or_else(|| Error::validation("files.data", "base64 required"))?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| Error::validation("files.data", "invalid base64"))?;
        files.push(UploadFile::new(name, data));
    }
    crate::validate_uploads(&files)?;
    Ok(files)
}
impl ValidatedOperation {
    /// Inspect the validated single-request representation for supported operations; this does not assess authority or execute it.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::capability;
    /// let operation = capability("get_api_version").unwrap().validate(&serde_json::json!({}))?;
    /// let request = operation.api_request()?;
    /// assert_eq!(request.path, "/version");
    /// assert!(request.public);
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn api_request(&self) -> Result<OperationRequest> {
        let id = |key: &str| self.args[key].as_str().unwrap_or_default();
        let (method, path, public) = match self.name() {
            "resolve_id" => (Method::GET, format!("/resolve/{}", id("id")), false),
            "get_member_permissions" => (
                Method::GET,
                format!("/servers/{}/members/{}/permissions", id("id"), id("user")),
                false,
            ),
            "get_channel_member_permissions" => (
                Method::GET,
                format!("/channels/{}/permissions/{}", id("channel"), id("user")),
                false,
            ),
            "get_role" => (
                Method::GET,
                format!("/servers/{}/roles/{}", id("id"), id("role")),
                false,
            ),
            "reorder_roles" => (Method::PATCH, format!("/servers/{}/roles", id("id")), false),
            "get_channel_overrides" => (
                Method::GET,
                format!("/channels/{}/overrides", id("channel")),
                false,
            ),
            "set_channel_override" => (
                Method::PUT,
                format!("/channels/{}/overrides/{}", id("channel"), id("target")),
                false,
            ),
            "delete_channel_override" => (
                Method::DELETE,
                format!("/channels/{}/overrides/{}", id("channel"), id("target")),
                false,
            ),
            "get_category_overrides" => (
                Method::GET,
                format!("/categories/{}/overrides", id("category")),
                false,
            ),
            "set_category_override" => (
                Method::PUT,
                format!("/categories/{}/overrides/{}", id("category"), id("target")),
                false,
            ),
            "delete_category_override" => (
                Method::DELETE,
                format!("/categories/{}/overrides/{}", id("category"), id("target")),
                false,
            ),
            "send_sticker" => (
                Method::POST,
                format!("/channels/{}/messages", id("channel")),
                false,
            ),
            "get_api_version" => (Method::GET, "/version".into(), true),
            "get_api_changelog" => (
                Method::GET,
                format!(
                    "/changelog?limit={}",
                    self.args["limit"].as_u64().unwrap_or(10)
                ),
                true,
            ),
            "get_message_emojis" => (
                Method::GET,
                format!(
                    "/channels/{}/messages/{}/emojis",
                    id("channel"),
                    id("message")
                ),
                false,
            ),
            "list_emojis" => (Method::GET, format!("/servers/{}/emojis", id("id")), false),
            "create_emoji" => (Method::POST, format!("/servers/{}/emojis", id("id")), false),
            "edit_emoji" => (
                Method::PATCH,
                format!("/servers/{}/emojis/{}", id("id"), id("emoji")),
                false,
            ),
            "delete_emoji" => (
                Method::DELETE,
                format!("/servers/{}/emojis/{}", id("id"), id("emoji")),
                false,
            ),
            "list_channel_webhooks" => (
                Method::GET,
                format!("/channels/{}/webhooks", id("channel")),
                false,
            ),
            "list_bot_webhooks" => (Method::GET, "/webhooks".into(), false),
            "create_bot_webhook" => (
                Method::POST,
                format!("/channels/{}/webhooks", id("channel")),
                false,
            ),
            "delete_bot_webhook" => (
                Method::DELETE,
                format!("/webhooks/{}", id("webhook")),
                false,
            ),
            "post_bot_webhook" => (Method::POST, format!("/webhooks/{}", id("webhook")), false),
            "send_files" => (
                Method::POST,
                format!("/channels/{}/messages", id("channel")),
                false,
            ),
            _ => {
                return Err(Error::validation(
                    "operation",
                    "no single platform request for this operation",
                ))
            }
        };
        Ok(OperationRequest {
            method,
            path,
            public,
            body: if self.name() == "send_sticker" {
                Some(json!({"sticker_id":self.args["sticker"]}))
            } else {
                self.args.get("body").cloned()
            },
            files: if self.name() == "send_files" {
                files(&self.args["files"])?
            } else {
                vec![]
            },
        })
    }
}

impl ValidatedOperation {
    pub(super) async fn platform_scopes<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
        channel: Option<uuid::Uuid>,
        report: &mut AuthorizationReport,
    ) -> bool {
        let lookup = if matches!(self.name(), "edit_emoji" | "delete_emoji") {
            Some((
                format!("/servers/{}/emojis", context.server_id()),
                "emojis",
                "emoji",
            ))
        } else if matches!(self.name(), "delete_bot_webhook" | "post_bot_webhook") {
            Some((
                format!("/channels/{}/webhooks", channel.expect("channel scope")),
                "webhooks",
                "webhook",
            ))
        } else {
            None
        };
        let Some((path, key, target)) = lookup else {
            return true;
        };
        let result = api.request(Method::GET, &path, None).await;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                report.failure("resource_scope", Principal::Context, error);
                return false;
            }
        };
        let wanted = self.uuid(target);
        let row = value[key].as_array().and_then(|rows| {
            rows.iter().find(|row| {
                row["id"]
                    .as_str()
                    .and_then(|s| s.parse::<uuid::Uuid>().ok())
                    == Some(wanted)
            })
        });
        let Some(row) = row else {
            report.record(
                "resource_scope",
                Principal::Context,
                CheckResolution::unknown("target was not established in the scoped API list"),
                None,
            );
            return false;
        };
        if row["server_id"]
            .as_str()
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            != Some(context.server_id())
            || channel.is_some_and(|id| {
                row["channel_id"]
                    .as_str()
                    .and_then(|s| s.parse::<uuid::Uuid>().ok())
                    != Some(id)
            })
        {
            report.record(
                "resource_scope",
                Principal::Context,
                CheckResolution::denied("target belongs to a different server/channel"),
                None,
            );
            return false;
        }
        if key == "webhooks" {
            let me = match api.request(Method::GET, "/me", None).await {
                Ok(me) => me,
                Err(error) => {
                    report.failure("webhook_owner", Principal::Bot, error);
                    return false;
                }
            };
            let bot = me["id"].as_str().and_then(|s| s.parse::<uuid::Uuid>().ok());
            let owner = row["bot_id"]
                .as_str()
                .and_then(|s| s.parse::<uuid::Uuid>().ok());
            if bot.is_none() || owner.is_none() {
                report.record(
                    "webhook_owner",
                    Principal::Bot,
                    CheckResolution::unknown("webhook bot ownership was not established"),
                    None,
                );
                return false;
            }
            report.bot_id = bot;
            if bot != owner {
                report.record(
                    "webhook_owner",
                    Principal::Bot,
                    CheckResolution::denied("webhook belongs to another bot or a person"),
                    None,
                );
                return false;
            }
        }
        report.record(
            "resource_scope",
            Principal::Context,
            CheckResolution::allowed("target identity and server/channel were established"),
            None,
        );
        true
    }
}
