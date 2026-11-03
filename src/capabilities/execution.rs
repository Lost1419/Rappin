use super::*;
use crate::{Api, ApiExt, Member, MemberDetail, MessagesResponse};
use uuid::Uuid;

/// Constructed only by catalogue validation, never by deserializing model data.
#[derive(Debug, Clone)]
pub struct ValidatedOperation {
    pub(super) entry: &'static Capability,
    pub(super) args: Value,
    pub(super) writes: Vec<Value>,
    pub(super) limits: output::Limits,
}
#[derive(Debug, Clone, Serialize)]
pub struct OperationResult {
    pub operation: String,
    pub data: Value,
    pub dry_run: bool,
    /// None for offline previews; a preview never asserts authorization.
    pub authorization: Option<AuthorizationReport>,
}
impl ValidatedOperation {
    pub fn name(&self) -> &'static str {
        self.entry.name
    }
    pub fn arguments(&self) -> &Value {
        &self.args
    }
    pub fn capability(&self) -> &'static Capability {
        self.entry
    }
    pub(super) fn uuid(&self, key: &str) -> Uuid {
        self.args[key]
            .as_str()
            .expect("validated UUID string")
            .parse()
            .expect("validated UUID")
    }
    pub fn preview(&self) -> OperationResult {
        let mut arguments = self.args.clone();
        if self.name() == "create_emoji" {
            arguments["body"]["image"] = "[image omitted]".into();
        }
        if let Some(files) = arguments["files"].as_array_mut() {
            for file in files {
                file["data"] = "[file data omitted]".into();
            }
        }
        OperationResult {
            operation: self.name().into(),
            data: json!({"arguments":arguments,"write_steps":if self.entry.write {self.writes.len().max(1)}else{0}}),
            dry_run: true,
            authorization: None,
        }
    }
    pub async fn execute<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
    ) -> Result<OperationResult> {
        self.execute_with_policy(api, context, &NoAdditionalAccess)
            .await
    }
    pub async fn execute_with_policy<A: Api + ?Sized>(
        &self,
        api: &A,
        context: &OperationContext,
        policy: &dyn OperationPolicy,
    ) -> Result<OperationResult> {
        if self.args["dry_run"] == true {
            let scope = self.local_scope(context);
            if !scope.allowed() {
                return Err(YappinError::OperationAuthorization {
                    report: Box::new(scope),
                });
            }
            return Ok(self.preview());
        }
        let report = self.assess_with_policy(api, context, policy).await?;
        if !report.allowed() {
            return Err(YappinError::OperationAuthorization {
                report: Box::new(report),
            });
        }
        let data = self.run(api, context).await?;
        self.verify_result_scope(&data, context)?;
        let data = api.sanitize(data);
        let data = match validation::collection_key(self.name()) {
            Some(key) => output::apply(data, key, &self.limits)?,
            None => data,
        };
        Ok(OperationResult {
            operation: self.name().into(),
            data,
            dry_run: false,
            authorization: Some(report),
        })
    }
    fn verify_result_scope(&self, data: &Value, context: &OperationContext) -> Result<()> {
        let uuid = |value: &Value| value.as_str().and_then(|s| s.parse::<Uuid>().ok());
        let valid = match self.name() {
            "list_emojis" => data["emojis"].as_array().is_some_and(|rows| {
                rows.iter()
                    .all(|row| uuid(&row["server_id"]) == Some(context.server_id()))
            }),
            "list_channel_webhooks" => data["webhooks"].as_array().is_some_and(|rows| {
                rows.iter().all(|row| {
                    uuid(&row["server_id"]) == Some(context.server_id())
                        && uuid(&row["channel_id"]) == Some(self.uuid("channel"))
                })
            }),
            "create_bot_webhook" => {
                uuid(&data["webhook"]["channel_id"]) == Some(self.uuid("channel"))
                    && uuid(&data["webhook"]["server_id"]) == Some(context.server_id())
            }
            "post_bot_webhook" => {
                uuid(&data["message"]["channel_id"]) == Some(self.uuid("channel"))
            }
            "send_files" => data["messages"].as_array().is_some_and(|rows| {
                rows.iter()
                    .all(|row| uuid(&row["channel_id"]) == Some(self.uuid("channel")))
            }),
            "get_member" => uuid(&data["member"]["id"]) == Some(self.uuid("user")),
            "get_server" => uuid(&data["server"]["id"]) == Some(context.server_id()),
            "get_channel" => {
                uuid(&data["channel"]["id"]) == Some(self.uuid("id"))
                    && uuid(&data["channel"]["server_id"]) == Some(context.server_id())
            }
            "get_message" => {
                uuid(&data["message"]["id"]) == Some(self.uuid("message"))
                    && uuid(&data["message"]["channel_id"]) == Some(self.uuid("channel"))
            }
            "list_channels" => data["channels"].as_array().is_some_and(|channels| {
                channels.iter().all(|c| {
                    uuid(&c["server_id"]).is_none_or(|server| server == context.server_id())
                })
            }),
            _ => true,
        };
        if valid {
            Ok(())
        } else if self.entry.write {
            Err(YappinError::Remote(crate::RemoteError{kind:"operation_result_scope".into(),message:"API write result identity did not match the bound resource scope; reconcile before retrying".into(),status:None,retry_after:None,details:[("outcome".into(),"unknown".into()),("retry_safety".into(),"do_not_replay_workflow".into())].into()}))
        } else {
            Err(YappinError::validation(
                "scope",
                "API result identity does not match the bound resource scope",
            ))
        }
    }
    async fn run<A: Api + ?Sized>(&self, api: &A, context: &OperationContext) -> Result<Value> {
        let server = context.server_id();
        let text = |key: &str| self.args[key].as_str().unwrap_or("");
        let get =
            |path: String| async move { api.request("GET".parse().unwrap(), &path, None).await };
        let request = |method: &'static str, path: String, body: Option<Value>| async move {
            api.request(method.parse().unwrap(), &path, body).await
        };
        let id = || self.uuid("id");
        let message_path = || {
            format!(
                "/channels/{}/messages/{}",
                self.uuid("channel"),
                self.uuid("message")
            )
        };
        if platform::is_platform_operation(self.name()) {
            let request = self.api_request()?;
            let mut data = request.dispatch(api).await?;
            if self.name() == "send_files" {
                let messages = serde_json::from_value::<crate::UploadedMessages>(data)?
                    .into_result_for(request.files.len())?;
                return Ok(json!({"messages":messages}));
            }
            if self.name() == "list_bot_webhooks" {
                if let Some(webhooks) = data["webhooks"].as_array_mut() {
                    webhooks.retain(|row| {
                        row["server_id"]
                            .as_str()
                            .and_then(|s| s.parse::<Uuid>().ok())
                            == Some(server)
                    });
                } else {
                    return Err(YappinError::validation(
                        "webhooks",
                        "API did not return a webhook list",
                    ));
                }
            }
            return Ok(data);
        }
        match self.name() {
            "get_capabilities" => Ok(
                json!({"version":env!("CARGO_PKG_VERSION"),"tools":tools(),"library":true,"features":{"cli":cfg!(feature="cli"),"gateway":cfg!(feature="gateway"),"voice":cfg!(feature="voice"),"hot_reload":cfg!(feature="hot-reload")}}),
            ),
            "get_schema" => {
                if let Some(name) = self.args["name"].as_str() {
                    Ok(bundled_schema()["components"]["schemas"][name].clone())
                } else {
                    endpoint_schema(bundled_schema(), text("path"), text("method"))
                }
            }
            // Server-scoped bindings never disclose the bot's other servers.
            "list_servers" => {
                let value = get(format!("/servers/{server}")).await?;
                Ok(json!({"servers":[value["server"]]}))
            }
            "get_server" => get(format!("/servers/{server}")).await,
            "list_channels" => get(format!("/servers/{server}/channels")).await,
            "create_channel" => {
                request(
                    "POST",
                    format!("/servers/{server}/channels"),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "edit_channel" => {
                request(
                    "PATCH",
                    format!("/channels/{}", id()),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "delete_channel" => request("DELETE", format!("/channels/{}", id()), None).await,
            "create_category" => {
                request(
                    "POST",
                    format!("/servers/{server}/categories"),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "edit_category" => {
                request(
                    "PATCH",
                    format!("/servers/{server}/categories/{}", self.uuid("category")),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "delete_category" => {
                request(
                    "DELETE",
                    format!("/servers/{server}/categories/{}", self.uuid("category")),
                    None,
                )
                .await
            }
            "get_channel" => get(format!("/channels/{}", id())).await,
            "list_roles" => get(format!("/servers/{server}/roles")).await,
            "get_member_counts" => get(format!("/servers/{server}/counts")).await,
            "list_invites" => get(format!("/servers/{server}/invites")).await,
            "list_server_events" => get(format!("/servers/{server}/events")).await,
            "get_member" => {
                let mut value =
                    get(format!("/servers/{server}/members/{}", self.uuid("user"))).await?;
                // Bot-only invite attribution is not implied by ordinary requester
                // membership. The explicit invite operation requires Manage Server.
                if let Some(member) = value["member"].as_object_mut() {
                    member.remove("invite_code");
                    member.remove("inviter_id");
                }
                Ok(value)
            }
            "list_members" => {
                let limit = self.args["limit"].as_u64().unwrap_or(200) as u32;
                let mut cursor = self.args["after"]
                    .as_str()
                    .map(|s| s.parse::<Uuid>().expect("validated cursor"));
                let mut members = vec![];
                let mut pages = 0;
                let mut more = false;
                for _ in 0..self.args["max_pages"].as_u64().unwrap_or(1) {
                    let batch = member_page(api, server, limit, cursor).await?;
                    pages += 1;
                    more = batch.len() == limit as usize;
                    let next = batch.last().map(|m| m.user.id);
                    members.extend(batch);
                    if !more || next == cursor {
                        more = false;
                        break;
                    }
                    cursor = next;
                }
                Ok(
                    json!({"members":members,"next_cursor":if more{cursor}else{None},"page_count":pages,"may_have_more":more}),
                )
            }
            "search_members" | "resolve_member" => {
                let result = crate::lookup::search_members_with(
                    text("name"),
                    self.args["max_pages"].as_u64().unwrap_or(1) as usize,
                    |id| async move {
                        let value = get(format!("/servers/{server}/members/{id}")).await?;
                        Ok(serde_json::from_value::<MemberDetail>(
                            value["member"].clone(),
                        )?)
                    },
                    |after| member_page(api, server, 500, after),
                )
                .await?;
                if self.name() == "resolve_member" {
                    return Ok(serde_json::to_value(result.into_unique("member")?)?);
                }
                let status = result.status();
                let mut value = serde_json::to_value(result)?;
                value["status"] = serde_json::to_value(status)?;
                Ok(value)
            }
            "search_channels" | "resolve_channel" => {
                let value = get(format!("/servers/{server}/channels")).await?;
                let channels =
                    serde_json::from_value::<Vec<crate::Channel>>(value["channels"].clone())?;
                let result = crate::lookup::channel_matches(channels, text("name"));
                if self.name() == "resolve_channel" {
                    return Ok(serde_json::to_value(result.into_unique("channel")?)?);
                }
                let status = result.status();
                let mut value = serde_json::to_value(result)?;
                value["status"] = serde_json::to_value(status)?;
                Ok(value)
            }
            "get_message" => get(message_path()).await,
            "read_messages" => {
                let channel = self.uuid("channel");
                let limit = self.args["limit"].as_u64().unwrap_or(50) as u32;
                let forward = self.args.get("after").is_some();
                let mut cursor = if forward {
                    self.args["after"].as_str()
                } else {
                    self.args["before"].as_str()
                }
                .map(str::to_owned);
                let mut all = vec![];
                let mut authors = std::collections::BTreeMap::new();
                let mut seen = std::collections::HashSet::new();
                let mut pages = 0;
                let mut more = false;
                for _ in 0..self.args["max_pages"].as_u64().unwrap_or(1) {
                    let path = {
                        let mut query = url::form_urlencoded::Serializer::new(String::new());
                        query.append_pair("limit", &limit.to_string());
                        if let Some(cursor) = &cursor {
                            query.append_pair(if forward { "after" } else { "before" }, cursor);
                        }
                        format!("/channels/{channel}/messages?{}", query.finish())
                    };
                    let page: MessagesResponse = api
                        .api_json::<_, ()>("GET".parse().unwrap(), &path, None)
                        .await?;
                    if page.messages.iter().any(|m| m.channel_id != channel) {
                        return Err(YappinError::validation(
                            "scope",
                            "API returned a message from another channel",
                        ));
                    }
                    pages += 1;
                    more = page.messages.len() == limit as usize;
                    let next = if forward {
                        page.messages.iter().map(|m| m.created_at).max()
                    } else {
                        page.messages.iter().map(|m| m.created_at).min()
                    }
                    .map(|t| t.to_rfc3339());
                    for message in page.messages {
                        if seen.insert(message.id) {
                            all.push(message);
                        }
                    }
                    for author in page.authors {
                        authors.insert(author.id, author);
                    }
                    if !more || next.is_none() || next == cursor {
                        more = false;
                        break;
                    }
                    cursor = next;
                }
                all.sort_by_key(|m| m.created_at);
                Ok(
                    json!({"messages":all,"authors":authors.into_values().collect::<Vec<_>>(),"next_cursor":if more{cursor}else{None},"page_count":pages,"may_have_more":more,"direction":if forward{"after"}else{"before"}}),
                )
            }
            "send_message" => {
                let mut completed = vec![];
                for (step, body) in self.writes.iter().enumerate() {
                    match request(
                        "POST",
                        format!("/channels/{}/messages", self.uuid("channel")),
                        Some(body.clone()),
                    )
                    .await
                    {
                        Ok(value) => completed.push(api.sanitize(value)),
                        Err(source) => {
                            if self.args["split"] != true {
                                return Err(source);
                            }
                            return Err(YappinError::PartialWrite {
                                completed,
                                failed_step: step + 1,
                                source: Box::new(source),
                            });
                        }
                    }
                }
                Ok(json!({"messages":completed}))
            }
            "edit_message" => {
                request("PATCH", message_path(), Some(self.args["body"].clone())).await
            }
            "delete_message" => request("DELETE", message_path(), None).await,
            "add_reaction" | "remove_reaction" => {
                let emoji = url::form_urlencoded::byte_serialize(text("emoji").as_bytes())
                    .collect::<String>()
                    .replace('+', "%20");
                request(
                    if self.name() == "add_reaction" {
                        "PUT"
                    } else {
                        "DELETE"
                    },
                    format!("{}/reactions/{emoji}", message_path()),
                    None,
                )
                .await
            }
            "pin_message" | "unpin_message" => {
                request(
                    if self.name() == "pin_message" {
                        "PUT"
                    } else {
                        "DELETE"
                    },
                    format!("{}/pin", message_path()),
                    None,
                )
                .await
            }
            "create_role" => {
                request(
                    "POST",
                    format!("/servers/{server}/roles"),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "edit_role" => {
                request(
                    "PATCH",
                    format!("/servers/{server}/roles/{}", self.uuid("role")),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "delete_role" => {
                request(
                    "DELETE",
                    format!("/servers/{server}/roles/{}", self.uuid("role")),
                    None,
                )
                .await
            }
            "set_everyone_permissions" => {
                request(
                    "PATCH",
                    format!("/servers/{server}/roles/everyone"),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            "get_channel_permissions" => get(format!("/channels/{}/permissions", id())).await,
            "set_channel_permissions" | "clear_channel_permissions" => {
                request(
                    if self.entry.write && self.name().starts_with("set_") {
                        "PUT"
                    } else {
                        "DELETE"
                    },
                    format!("/channels/{}/permissions/{}", id(), text("target")),
                    self.args.get("body").cloned(),
                )
                .await
            }
            "set_category_permissions" | "clear_category_permissions" => {
                request(
                    if self.name().starts_with("set_") {
                        "PUT"
                    } else {
                        "DELETE"
                    },
                    format!(
                        "/servers/{server}/categories/{}/permissions/{}",
                        self.uuid("category"),
                        text("target")
                    ),
                    self.args.get("body").cloned(),
                )
                .await
            }
            "create_server_event" => {
                request(
                    "POST",
                    format!("/servers/{server}/events"),
                    Some(self.args["body"].clone()),
                )
                .await
            }
            _ => Err(YappinError::validation(
                "operation",
                "operation requires a runtime outside server-scoped API bindings",
            )),
        }
    }
}
async fn member_page<A: Api + ?Sized>(
    api: &A,
    server: Uuid,
    limit: u32,
    after: Option<Uuid>,
) -> Result<Vec<Member>> {
    let path = match after {
        Some(after) => format!("/servers/{server}/members?limit={limit}&after={after}"),
        None => format!("/servers/{server}/members?limit={limit}"),
    };
    let value = api.request("GET".parse().unwrap(), &path, None).await?;
    Ok(serde_json::from_value(value["members"].clone())?)
}
