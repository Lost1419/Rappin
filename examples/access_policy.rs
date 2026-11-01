//! Read-only public API policy example. See docs/authorization-policy.md.
#[path = "access_policy/fixture.rs"]
mod fixture;
#[path = "access_policy/policy.rs"]
mod policy;
use rappin::{capabilities::*, Api, Client, Result, YappinError};
use reqwest::Method;
use serde_json::{json, Value};
use uuid::Uuid;

fn call(name: &str, args: &Value) -> Result<ValidatedOperation> {
    CapabilityFilter {
        names: [
            "create_channel",
            "edit_channel",
            "delete_channel",
            "edit_category",
            "delete_category",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        library_only: true,
        ..Default::default()
    }
    .validate(name, args)
}
async fn demo() -> Result<Value> {
    let api = fixture::Fixture::default();
    let call = call(
        "edit_channel",
        &json!({"id":fixture::channel(),"body":{"name":"Renamed"}}),
    )?;
    let context = OperationContext::server(fixture::server(), fixture::requester());
    let assessment = call
        .assess_with_policy(&api, &context, &policy::PublicApiPolicy { api: &api })
        .await?;
    let error = policy::execute(&api, &context, &call)
        .await
        .expect_err("incomplete evidence must block execution");
    Ok(
        json!({"mode":"offline_fixture","assessment":assessment,"execution_error":error.to_json(),"writes":api.writes.load(std::sync::atomic::Ordering::SeqCst)}),
    )
}
fn client() -> Result<Client> {
    let token = std::env::var("YAPPER_BOT_TOKEN").map_err(|_| {
        YappinError::Config("set YAPPER_BOT_TOKEN for read-only live inspection".into())
    })?;
    Ok(Client::new(token))
}
fn keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}
async fn inspect(api: &impl Api) -> Result<Value> {
    let me = api.request(Method::GET, "/me", None).await?;
    let servers = api.request(Method::GET, "/servers", None).await?;
    let server = servers["servers"][0]["id"]
        .as_str()
        .ok_or_else(|| YappinError::Config("no server visible to this credential".into()))?;
    let inventory = api
        .request(Method::GET, &format!("/servers/{server}/channels"), None)
        .await?;
    let channel = inventory["channels"][0]["id"]
        .as_str()
        .ok_or_else(|| YappinError::Config("no channel visible to this credential".into()))?;
    let row = api
        .request(Method::GET, &format!("/channels/{channel}"), None)
        .await?;
    let member = api
        .request(
            Method::GET,
            &format!(
                "/servers/{server}/members/{}",
                me["id"].as_str().unwrap_or("missing")
            ),
            None,
        )
        .await?;
    let roles = api
        .request(Method::GET, &format!("/servers/{server}/roles"), None)
        .await?;
    let summary = |result: Result<Value>| match result {
        Ok(value) => {
            json!({"status":200,"keys":keys(&value),"first_override_keys":keys(&value["overrides"][0])})
        }
        Err(error) => json!({"status":error.status_code(),"error_kind":error.to_json()["kind"]}),
    };
    let overrides = summary(
        api.request(
            Method::GET,
            &format!("/channels/{channel}/permissions"),
            None,
        )
        .await,
    );
    // Diagnostic only: the guide groups GET/PUT/DELETE for these paths but does
    // not give an unambiguous category-GET response contract. Never authorize
    // from a guessed shape, nor turn 403/404 into a requester access decision.
    let category_read = if let Some(category) = inventory["categories"][0]["id"].as_str() {
        summary(
            api.request(
                Method::GET,
                &format!("/servers/{server}/categories/{category}/permissions/everyone"),
                None,
            )
            .await,
        )
    } else {
        json!({"skipped":"no category visible"})
    };
    Ok(
        json!({"mode":"live_read_only_field_inventory","channel_keys":keys(&row["channel"]),"category_keys":keys(&inventory["categories"][0]),"member_keys":keys(&member["member"]),"first_role_keys":keys(&roles["roles"][0]),"channel_override_read":overrides,"category_role_get_diagnostic":category_read}),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = match args.as_slice() {
        [] => demo().await?,
        [flag] if flag == "--inspect" => inspect(&client()?).await?,
        [flag, server, requester, operation, file] if flag == "--assess" => {
            let parse = |s: &str| {
                s.parse::<Uuid>()
                    .map_err(|_| YappinError::validation("identity", "expected UUID"))
            };
            let context = OperationContext::server(parse(server)?, parse(requester)?);
            let call = call(operation, &serde_json::from_slice(&std::fs::read(file)?)?)?;
            let api = client()?;
            // Assessment only. This executable never submits a live mutation.
            serde_json::to_value(
                call.assess_with_policy(&api, &context, &policy::PublicApiPolicy { api: &api })
                    .await?,
            )?
        }
        _ => return Err(YappinError::Config(
            "usage: access_policy [--inspect | --assess SERVER REQUESTER OPERATION ARGUMENTS.json]"
                .into(),
        )),
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
#[path = "access_policy/tests.rs"]
mod tests;
