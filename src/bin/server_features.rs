use super::*;

pub async fn server_action(action: Servers, ctx: &Context) -> Result<Value> {
    match action {
        Servers::Counts { id } => {
            ctx.request(Method::GET, &format!("/servers/{id}/counts"), None)
                .await
        }
        Servers::Invites { id } => {
            ctx.request(Method::GET, &format!("/servers/{id}/invites"), None)
                .await
        }
        Servers::Roles { id } => {
            ctx.request(Method::GET, &format!("/servers/{id}/roles"), None)
                .await
        }
        Servers::ScheduledEvents { id } => {
            ctx.request(Method::GET, &format!("/servers/{id}/events"), None)
                .await
        }
        Servers::RoleCreate { id, body_file } => {
            let body: rappin::CreateRole = serde_json::from_value(ctx.body(&body_file)?)?;
            ctx.request(
                Method::POST,
                &format!("/servers/{id}/roles"),
                Some(serde_json::to_value(body)?),
            )
            .await
        }
        Servers::RoleEdit {
            id,
            role,
            body_file,
        } => {
            let body: rappin::EditRole = serde_json::from_value(ctx.body(&body_file)?)?;
            ctx.request(
                Method::PATCH,
                &format!("/servers/{id}/roles/{role}"),
                Some(serde_json::to_value(body)?),
            )
            .await
        }
        Servers::RoleDelete { id, role } => {
            ctx.request(Method::DELETE, &format!("/servers/{id}/roles/{role}"), None)
                .await
        }
        Servers::EveryonePermissions { id, body_file } => {
            #[derive(serde::Deserialize, serde::Serialize)]
            struct Body {
                permissions: rappin::Permissions,
            }
            let body: Body = serde_json::from_value(ctx.body(&body_file)?)?;
            ctx.request(
                Method::PATCH,
                &format!("/servers/{id}/roles/everyone"),
                Some(serde_json::to_value(body)?),
            )
            .await
        }
        Servers::CategoryPermissionsSet {
            id,
            category,
            target,
            body_file,
        } => {
            let body: rappin::PermissionOverride = serde_json::from_value(ctx.body(&body_file)?)?;
            ctx.request(
                Method::PUT,
                &format!("/servers/{id}/categories/{category}/permissions/{target}"),
                Some(serde_json::to_value(body)?),
            )
            .await
        }
        Servers::CategoryPermissionsClear {
            id,
            category,
            target,
        } => {
            ctx.request(
                Method::DELETE,
                &format!("/servers/{id}/categories/{category}/permissions/{target}"),
                None,
            )
            .await
        }
        Servers::EventCreate { id, body_file } => {
            let body: rappin::CreateServerEvent = serde_json::from_value(ctx.body(&body_file)?)?;
            body.validate()?;
            ctx.request(
                Method::POST,
                &format!("/servers/{id}/events"),
                Some(serde_json::to_value(body)?),
            )
            .await
        }
        Servers::List | Servers::Get { .. } => unreachable!("handled by caller"),
    }
}
