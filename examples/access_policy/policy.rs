//! Pre-1.4 evidence-composition reference. Current applications can use
//! ComputedPermissionsPolicy for authoritative channel checks; see docs/api-1.4.md.
use async_trait::async_trait;
use rappin::{capabilities::*, Api, ApiExt, Error, Freshness, Permission, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub struct PublicApiPolicy<'a, A: Api + ?Sized> {
    pub api: &'a A,
}

#[derive(Deserialize)]
struct OverrideResponse {
    overrides: Vec<rappin::ChannelPermissionOverride>,
}

fn uuid(value: &Value) -> Option<Uuid> {
    value.as_str().and_then(|s| s.parse().ok())
}

#[async_trait]
impl<A: Api + ?Sized> OperationPolicy for PublicApiPolicy<'_, A> {
    async fn check(&self, q: &AccessQuestion<'_>) -> Result<CheckResolution> {
        if !matches!(
            q.check,
            ExternalCheck::ChannelPermissions | ExternalCheck::CategoryPermissions
        ) {
            return Ok(CheckResolution::unknown(
                "no public-API evidence evaluator for this check",
            ));
        }
        let Some(actor) = q.actor_id else {
            return Ok(CheckResolution::unknown("actor identity is missing"));
        };
        if q.principal == Principal::Context {
            return Ok(CheckResolution::unknown(
                "a requester or bot principal is required",
            ));
        }
        // Always authenticate the bot separately; a requester's failed read would
        // describe the credential performing the read, not that requester's ACL.
        let me = self.api.request(Method::GET, "/me", None).await?;
        let Some(bot) = uuid(&me["id"]) else {
            return Ok(CheckResolution::unknown(
                "authenticated bot identity is unavailable",
            ));
        };
        if q.principal == Principal::Bot && actor != bot {
            return Ok(CheckResolution::denied(
                "bot actor does not match authenticated /me identity",
            ));
        }

        let (category, channel_read) = match q.check {
            ExternalCheck::ChannelPermissions => {
                let Some(channel) = q.channel_id else {
                    return Ok(CheckResolution::unknown("channel identity is missing"));
                };
                let data = self
                    .api
                    .request(Method::GET, &format!("/channels/{channel}"), None)
                    .await?;
                let row = &data["channel"];
                if uuid(&row["id"]) != Some(channel) {
                    return Ok(CheckResolution::denied(
                        "channel response identity does not match the requested resource",
                    ));
                }
                match uuid(&row["server_id"]) {
                    Some(server) if server == q.server_id => (),
                    Some(_) => {
                        return Ok(CheckResolution::denied(
                            "channel belongs to a different server",
                        ))
                    }
                    None => {
                        return Ok(CheckResolution::unknown(
                            "channel server scope is unavailable",
                        ))
                    }
                }
                (row.get("category_id").cloned(), true)
            }
            ExternalCheck::CategoryPermissions => {
                let category = match q.operation {
                    "create_channel" | "edit_channel" => uuid(&q.arguments["body"]["category_id"]),
                    "edit_category" | "delete_category" => uuid(&q.arguments["category"]),
                    _ => None,
                };
                let Some(category) = category else {
                    return Ok(CheckResolution::unknown(
                        "no destination/target category was established for this check",
                    ));
                };
                // Membership in this bot-visible list proves only scoped discovery.
                let data = self
                    .api
                    .request(
                        Method::GET,
                        &format!("/servers/{}/channels", q.server_id),
                        None,
                    )
                    .await?;
                let Some(rows) = data["categories"].as_array() else {
                    return Ok(CheckResolution::unknown(
                        "category inventory is unavailable",
                    ));
                };
                if !rows.iter().any(|row| uuid(&row["id"]) == Some(category)) {
                    return Ok(CheckResolution::unknown("category is not in the bot-visible server inventory; existence/access is unproven"));
                }
                (Some(json!(category)), false)
            }
            _ => unreachable!(),
        };

        // Refresh is honored by Client and by the parent behind WorkerContext.
        // This proves SERVER grants only; it cannot grant effective channel access.
        let snapshot = self
            .api
            .permission_snapshot(q.server_id, actor, Freshness::Refresh)
            .await?;
        if snapshot.server.id != q.server_id || snapshot.actor != actor {
            return Ok(CheckResolution::denied(
                "permission snapshot actor/server identity mismatch",
            ));
        }
        let server = snapshot.evaluate()?;

        // The only positive effective fact this example can prove is a read made
        // as that exact bot, for view_channel alone. Edits/deletes also ask for
        // manage_channels and never take this branch. No owner/Admin shortcut.
        if channel_read
            && q.principal == Principal::Bot
            && actor == bot
            && q.permissions == ["view_channel"]
        {
            return Ok(CheckResolution::allowed("fresh channel GET succeeded as the authenticated bot; proves bot view_channel only"));
        }

        let base = format!("fresh server grants: owner={}, administrator={}, manage_channels={}; these do not prove effective resource access",
            server.owner, server.administrator, server.allows(&Permission::new("manage_channels")));
        match q.check {
            ExternalCheck::ChannelPermissions => {
                let channel = q.channel_id.expect("verified channel");
                // Preserve typed API errors (including 403/404/429) as UNKNOWN
                // checks in AuthorizationReport, not requester permission denials.
                let overrides: OverrideResponse = self.api.api_json::<_, ()>(Method::GET,
                    &format!("/channels/{channel}/permissions"), None).await?;
                Ok(CheckResolution::unknown(format!("{base}; category_id={category:?}; {} configured channel override rows observed. Public data does not establish category rule contents/completeness, role conflict order, private allowlists or effective permissions for this actor", overrides.overrides.len())))
            }
            ExternalCheck::CategoryPermissions => Ok(CheckResolution::unknown(format!("{base}; category={category:?} is discoverable. Category ACL read/response semantics, category access prerequisites, inheritance and role conflict precedence are not specified sufficiently to prove this actor's access"))),
            _ => unreachable!(),
        }
    }
}

pub async fn execute<A: Api + ?Sized>(
    api: &A,
    context: &OperationContext,
    call: &ValidatedOperation,
) -> Result<OperationResult> {
    let context = context.clone().freshness(Freshness::Refresh);
    call.execute_with_policy(api, &context, &PublicApiPolicy { api })
        .await
}

/// Native and worker commands retain their authenticated requester and resource scope.
#[allow(dead_code)]
pub async fn command(
    ctx: &rappin::CommandContext,
    call: &ValidatedOperation,
) -> Result<OperationResult> {
    let server = ctx
        .server_id()
        .ok_or_else(|| Error::validation("server", "server command required"))?;
    let requester = ctx
        .author_id()
        .ok_or_else(|| Error::validation("requester", "requester required"))?;
    execute(
        ctx,
        &OperationContext::server(server, requester).freshness(Freshness::Refresh),
        call,
    )
    .await
}

#[allow(dead_code)]
pub fn native_command(binding: OperationCommandBinding) -> Result<rappin::RegisteredCommand> {
    binding.bind(|ctx, call| async move {
        command(&ctx, &call).await?;
        Ok(())
    })
}

#[cfg(feature = "hot-reload")]
#[allow(dead_code)]
pub fn worker_cog(binding: OperationCommandBinding) -> Result<rappin::WorkerCog> {
    let schema = binding.command_schema()?;
    rappin::WorkerCog::new("resource_access").dynamic_command(schema, move |ctx, values| {
        let binding = binding.clone();
        async move {
            let call = binding.prepare(&values)?;
            command(&ctx, &call).await?;
            Ok(())
        }
    })
}
