//! Server permission evaluation, independent of command routing or storage.
use crate::{MemberDetail, Permission, Result, Role, Server, YappinError};
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

/// Application-supplied or API-fetched data. Evaluation is pure and does no I/O.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{PermissionSnapshot, Permission};
/// use uuid::Uuid;
/// let server = serde_json::from_value(serde_json::json!({
///     "id": Uuid::nil(), "name": "Example", "owner_id": Uuid::nil()
/// }))?;
/// let snapshot = PermissionSnapshot { server, actor: Uuid::nil(), member: None, roles: vec![] };
/// let access = snapshot.evaluate()?;
/// assert!(access.owner);
/// assert!(access.allows(&Permission::manage_messages()));
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub struct PermissionSnapshot {
    /// Server evidence, including the actual owner ID.
    pub server: Server,
    /// Actor being evaluated; assess requester and bot independently.
    pub actor: Uuid,
    /// Matching membership evidence, required for a non-owner.
    pub member: Option<MemberDetail>,
    /// Inventory resolving every assigned role and the everyone role.
    pub roles: Vec<Role>,
}

/// Server grants only: channel overrides, timeouts and role hierarchy still
/// require their own policy checks and remain enforced by Yapin on every write.
#[derive(Debug, Clone, PartialEq, Eq)]
/// # Example
///
/// ```rust
/// use rappin::{PermissionSnapshot, Permission};
/// use uuid::Uuid;
/// let server = serde_json::from_value(serde_json::json!({
///     "id": Uuid::nil(), "name": "Example", "owner_id": Uuid::nil()
/// }))?;
/// let snapshot = PermissionSnapshot { server, actor: Uuid::nil(), member: None, roles: vec![] };
/// let access = snapshot.evaluate()?;
/// assert!(access.owner);
/// assert!(access.allows(&Permission::manage_messages()));
/// # Ok::<(), rappin::YappinError>(())
/// ```
pub struct PermissionEvaluation {
    /// Server scope of this evaluation.
    pub server_id: Uuid,
    /// Actor whose grants were evaluated.
    pub actor_id: Uuid,
    /// Whether the actor matches the server owner ID.
    pub owner: bool,
    /// Whether assigned roles grant Administrator.
    pub administrator: bool,
    /// Union of server-wide grant names; owner bypass need not populate the set.
    pub granted: BTreeSet<String>,
}
impl PermissionEvaluation {
    /// Test server-wide grants, with owner and Administrator bypass; resource overrides and hierarchy are separate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{PermissionSnapshot, Permission};
    /// use uuid::Uuid;
    /// let server = serde_json::from_value(serde_json::json!({
    ///     "id": Uuid::nil(), "name": "Example", "owner_id": Uuid::nil()
    /// }))?;
    /// let snapshot = PermissionSnapshot { server, actor: Uuid::nil(), member: None, roles: vec![] };
    /// let access = snapshot.evaluate()?;
    /// assert!(access.owner);
    /// assert!(access.allows(&Permission::manage_messages()));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn allows(&self, permission: &Permission) -> bool {
        self.owner || self.administrator || self.granted.contains(permission.as_str())
    }
    /// Test server-wide grants, with owner and Administrator bypass; resource overrides and hierarchy are separate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{PermissionSnapshot, Permission};
    /// use uuid::Uuid;
    /// let server = serde_json::from_value(serde_json::json!({
    ///     "id": Uuid::nil(), "name": "Example", "owner_id": Uuid::nil()
    /// }))?;
    /// let snapshot = PermissionSnapshot { server, actor: Uuid::nil(), member: None, roles: vec![] };
    /// let access = snapshot.evaluate()?;
    /// assert!(access.owner);
    /// assert!(access.allows_all(&[Permission::manage_messages()]));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn allows_all(&self, permissions: &[Permission]) -> bool {
        permissions.iter().all(|p| self.allows(p))
    }
    /// Test server-wide grants, with owner and Administrator bypass; resource overrides and hierarchy are separate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{PermissionSnapshot, Permission};
    /// use uuid::Uuid;
    /// let server = serde_json::from_value(serde_json::json!({
    ///     "id": Uuid::nil(), "name": "Example", "owner_id": Uuid::nil()
    /// }))?;
    /// let snapshot = PermissionSnapshot { server, actor: Uuid::nil(), member: None, roles: vec![] };
    /// let access = snapshot.evaluate()?;
    /// assert!(access.owner);
    /// assert!(access.require(&[Permission::manage_messages()]).is_ok());
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn require(&self, permissions: &[Permission]) -> Result<()> {
        if let Some(permission) = permissions.iter().find(|p| !self.allows(p)) {
            return Err(crate::CommandError::Denied(format!(
                "missing permission {}",
                permission.as_str()
            ))
            .into());
        }
        Ok(())
    }
}

impl PermissionSnapshot {
    /// Evaluate supplied evidence without network access. Missing or inconsistent membership/roles returns an error; role grants do not establish resource-specific access.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{PermissionSnapshot, Permission};
    /// use uuid::Uuid;
    /// let server = serde_json::from_value(serde_json::json!({
    ///     "id": Uuid::nil(), "name": "Example", "owner_id": Uuid::nil()
    /// }))?;
    /// let snapshot = PermissionSnapshot { server, actor: Uuid::nil(), member: None, roles: vec![] };
    /// let access = snapshot.evaluate()?;
    /// assert!(access.owner);
    /// assert!(access.allows(&Permission::manage_messages()));
    /// # Ok::<(), rappin::YappinError>(())
    /// ```
    pub fn evaluate(&self) -> Result<PermissionEvaluation> {
        let mut result = PermissionEvaluation {
            server_id: self.server.id,
            actor_id: self.actor,
            owner: self.server.owner_id == self.actor,
            administrator: false,
            granted: BTreeSet::new(),
        };
        if result.owner {
            return Ok(result);
        }
        let member = self
            .member
            .as_ref()
            .filter(|m| m.member.user.id == self.actor)
            .ok_or_else(|| {
                YappinError::validation(
                    "permissions",
                    "matching membership is required for a non-owner",
                )
            })?;
        let ids: HashSet<_> = self.roles.iter().map(|r| r.id).collect();
        if member.member.roles.iter().any(|id| !ids.contains(id)) || ids.len() != self.roles.len() {
            return Err(YappinError::validation(
                "permissions",
                "role snapshot is incomplete or contains duplicate IDs",
            ));
        }
        for role in self.roles.iter().filter(|role| {
            member.member.roles.contains(&role.id) || role.auto.as_deref() == Some("everyone")
        }) {
            let mut keys = BTreeSet::from(["manage_server", "manage_messages"]);
            if let Some(perms) = role.perms.as_ref().and_then(|p| p.as_object()) {
                keys.extend(perms.keys().map(String::as_str));
            }
            if let Some(perms) = &role.permissions {
                keys.extend(perms.keys().map(String::as_str));
            }
            for key in keys {
                // New-format explicit false must not fall back to a stale legacy true.
                let enabled = role
                    .permissions
                    .as_ref()
                    .and_then(|p| p.get(key))
                    .copied()
                    .or_else(|| {
                        role.perms
                            .as_ref()
                            .and_then(|p| p.get(key))
                            .and_then(|p| p.as_bool())
                    })
                    .unwrap_or_else(|| match key {
                        "manage_server" => role.manage_server == Some(true),
                        "manage_messages" => role.manage_messages == Some(true),
                        _ => false,
                    });
                if enabled {
                    result.granted.insert(key.to_owned());
                }
            }
        }
        result.administrator = result.granted.contains("administrator");
        Ok(result)
    }
}
