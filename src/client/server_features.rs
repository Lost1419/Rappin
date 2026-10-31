//! Endpoints currently restricted to Yapin test servers.
use super::*;

#[derive(Deserialize)]
struct RoleResponse {
    role: Role,
}

impl Client {
    /// Creates a role (Manage Roles; test servers only).
    pub async fn create_role(&self, server: Uuid, body: &CreateRole) -> Result<Role> {
        let result: Result<RoleResponse> = self
            .request_json(
                Method::POST,
                &format!("/servers/{server}/roles"),
                Some(body),
            )
            .await;
        Ok(result?.role)
    }

    /// Edits supplied fields only. Permission keys merge on the server.
    pub async fn edit_role(&self, server: Uuid, role: Uuid, body: &EditRole) -> Result<Role> {
        let result: Result<RoleResponse> = self
            .request_json(
                Method::PATCH,
                &format!("/servers/{server}/roles/{role}"),
                Some(body),
            )
            .await;
        Ok(result?.role)
    }

    /// Deletes a role below the bot's highest role (test servers only).
    pub async fn delete_role(&self, server: Uuid, role: Uuid) -> Result<()> {
        let result = self
            .request_json::<serde_json::Value, ()>(
                Method::DELETE,
                &format!("/servers/{server}/roles/{role}"),
                None,
            )
            .await;
        result.map(|_| ())
    }

    /// Changes only supplied @everyone permission keys (test servers only).
    pub async fn set_everyone_permissions(
        &self,
        server: Uuid,
        permissions: &Permissions,
    ) -> Result<()> {
        let result = self
            .request_json::<serde_json::Value, _>(
                Method::PATCH,
                &format!("/servers/{server}/roles/everyone"),
                Some(&serde_json::json!({"permissions":permissions})),
            )
            .await;
        result.map(|_| ())
    }

    /// Lists channel overrides (test servers only). Null role IDs mean @everyone.
    pub async fn get_channel_permissions(
        &self,
        channel: Uuid,
    ) -> Result<Vec<ChannelPermissionOverride>> {
        #[derive(Deserialize)]
        struct Response {
            overrides: Vec<ChannelPermissionOverride>,
        }
        let response: Response = self
            .request_json::<_, ()>(
                Method::GET,
                &format!("/channels/{channel}/permissions"),
                None,
            )
            .await?;
        Ok(response.overrides)
    }

    /// Replaces an override; empty allow/deny lists clear it (test servers only).
    pub async fn set_channel_permissions(
        &self,
        channel: Uuid,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<()> {
        self.request_json::<serde_json::Value, _>(
            Method::PUT,
            &format!("/channels/{channel}/permissions/{target}"),
            Some(body),
        )
        .await
        .map(|_| ())
    }

    /// Removes a channel override (test servers only).
    pub async fn clear_channel_permissions(
        &self,
        channel: Uuid,
        target: PermissionTarget,
    ) -> Result<()> {
        self.request_json::<serde_json::Value, ()>(
            Method::DELETE,
            &format!("/channels/{channel}/permissions/{target}"),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Replaces a category override (Manage Roles or Manage Channels; test servers only).
    pub async fn set_category_permissions(
        &self,
        server: Uuid,
        category: Uuid,
        target: PermissionTarget,
        body: &PermissionOverride,
    ) -> Result<()> {
        let result = self
            .request_json::<serde_json::Value, _>(
                Method::PUT,
                &format!("/servers/{server}/categories/{category}/permissions/{target}"),
                Some(body),
            )
            .await;
        result.map(|_| ())
    }

    /// Removes a category override (test servers only).
    pub async fn clear_category_permissions(
        &self,
        server: Uuid,
        category: Uuid,
        target: PermissionTarget,
    ) -> Result<()> {
        let result = self
            .request_json::<serde_json::Value, ()>(
                Method::DELETE,
                &format!("/servers/{server}/categories/{category}/permissions/{target}"),
                None,
            )
            .await;
        result.map(|_| ())
    }

    /// Live member/online counts (test servers only). Ask every few minutes.
    pub async fn get_member_counts(&self, server: Uuid) -> Result<MemberCounts> {
        self.request_json::<_, ()>(Method::GET, &format!("/servers/{server}/counts"), None)
            .await
    }

    /// Lists invites (Manage Server; test servers only).
    pub async fn get_invites(&self, server: Uuid) -> Result<Vec<Invite>> {
        #[derive(Deserialize)]
        struct Response {
            invites: Vec<Invite>,
        }
        let response: Response = self
            .request_json::<_, ()>(Method::GET, &format!("/servers/{server}/invites"), None)
            .await?;
        Ok(response.invites)
    }

    /// Lists upcoming and live scheduled events (test servers only).
    pub async fn get_server_events(&self, server: Uuid) -> Result<Vec<ServerEvent>> {
        #[derive(Deserialize)]
        struct Response {
            events: Vec<ServerEvent>,
        }
        let response: Response = self
            .request_json::<_, ()>(Method::GET, &format!("/servers/{server}/events"), None)
            .await?;
        Ok(response.events)
    }

    /// Creates a scheduled event (Manage Events or Manage Server; test servers only).
    /// Yapin enforces its limit of 25 upcoming events per server.
    pub async fn create_server_event(
        &self,
        server: Uuid,
        body: &CreateServerEvent,
    ) -> Result<ServerEvent> {
        body.validate()?;
        #[derive(Deserialize)]
        struct Response {
            event: ServerEvent,
        }
        let response: Response = self
            .request_json(
                Method::POST,
                &format!("/servers/{server}/events"),
                Some(body),
            )
            .await?;
        Ok(response.event)
    }
}
