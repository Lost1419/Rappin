//! Fresh hierarchy assessment; deletion requires an explicit operator flag.
use rappin::{capabilities::*, Api, Client, Result, RoleHierarchyPolicy, YappinError};
use serde_json::json;
use uuid::Uuid;

async fn apply(
    api: &(impl Api + ?Sized),
    server: Uuid,
    requester: Uuid,
    role: Uuid,
    execute: bool,
) -> Result<serde_json::Value> {
    let operation = capability("delete_role")
        .expect("built-in operation")
        .validate(&json!({"id":server,"role":role}))?;
    let mut context = OperationContext::server(server, requester);
    // Trusted host configuration, never an operation/model argument. Workers
    // inherit no arbitrary environment; configure their trust in host policy.
    if std::env::var("RAPPIN_TEST_SERVER").as_deref() == Ok("1") {
        context = context.test_server_access(true);
    }
    let policy = RoleHierarchyPolicy::new(api).owner_admin_strict();
    if execute {
        let result = policy.execute_deletion(&operation, &context).await?;
        Ok(serde_json::to_value(result)?)
    } else {
        Ok(serde_json::to_value(
            operation.assess_with_policy(api, &context, &policy).await?,
        )?)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 3 || args.len() == 4 && args[3] == "--execute") {
        return Err(YappinError::validation(
            "args",
            "SERVER REQUESTER ROLE [--execute]",
        ));
    }
    let parse = |index: usize| {
        args[index]
            .parse::<Uuid>()
            .map_err(|_| YappinError::validation("args", "expected UUID"))
    };
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("YAPPER_BOT_TOKEN is required".into()))?;
    let report = apply(
        &Client::new(token),
        parse(0)?,
        parse(1)?,
        parse(2)?,
        args.len() == 4,
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
