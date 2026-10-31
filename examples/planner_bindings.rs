//! Offline discovery plus reusable integration helpers; no model or storage SDK.
use rappin::{capabilities::*, Api, BotHandle, CogBuilder, Result, YappinError};
use serde_json::{json, Value};
use uuid::Uuid;

pub fn planner_filter() -> CapabilityFilter {
    CapabilityFilter {
        names: vec!["list_roles".into(), "get_server".into()],
        library_only: true,
        include_test_servers: false,
        ..Default::default()
    }
}
pub fn planner_capabilities() -> Vec<Value> {
    filtered_catalogue(&planner_filter())
        .into_iter()
        .map(Capability::tool_definition)
        .collect()
}
pub async fn execute_plan(
    api: &(impl Api + ?Sized),
    server: Uuid,
    requester: Uuid,
    name: &str,
    arguments: &Value,
) -> Result<OperationResult> {
    let call = planner_filter().validate(name, arguments)?;
    // Both IDs come from the authenticated event/host, not from model JSON.
    call.execute(api, &OperationContext::server(server, requester))
        .await
}
pub async fn install_permanent_roles_command(handle: &BotHandle, server: Uuid) -> Result<()> {
    let binding: OperationCommandBinding = serde_json::from_value(json!({
        "operation":"list_roles","name":"roles","description":"List this server's roles","fixed_arguments":{"id":server}
    }))?;
    binding.preflight(handle, Some("permanent_roles")).await?;
    let command = binding.bind(|ctx, call| async move {
        let server = ctx
            .server_id()
            .ok_or_else(|| YappinError::validation("server", "server command required"))?;
        let result = call
            .execute(&ctx, &OperationContext::server(server, ctx.author_id()))
            .await?;
        let count = result.data["roles"].as_array().map_or(0, Vec::len);
        ctx.reply(format!("{count} roles in this server")).await?;
        Ok(())
    })?;
    handle
        .add_cog(
            CogBuilder::new("permanent_roles")
                .registered_command(command)
                .build(),
        )
        .await
}
fn main() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&planner_capabilities())?);
    Ok(())
}
