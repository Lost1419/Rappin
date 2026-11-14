//! Fresh API 1.4 resource reads and mutations through native or worker transports.
use crate::capabilities::{AccessQuestion, CheckResolution, ExternalCheck, OperationPolicy};
use crate::*;
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn validate_role_order(ids: &[Uuid]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if ids.iter().any(|id| !seen.insert(id)) {
        return Err(Error::validation(
            "ids",
            "role order contains duplicate IDs",
        ));
    }
    Ok(())
}
pub(crate) fn decode_overrides(
    value: Value,
    channel: Option<Uuid>,
    category: Option<Uuid>,
) -> Result<Vec<ResourceOverride>> {
    let rules: Vec<ResourceOverride> = serde_json::from_value(value["overrides"].clone())?;
    if rules.iter().any(|r| {
        r.object != "override"
            || channel.is_some_and(|id| r.channel_id != Some(id))
            || category.is_some_and(|id| r.category_id != Some(id))
            || r.id != r.role_id
            || !matches!(
                (r.kind.as_str(), r.role_id),
                ("everyone", None) | ("role", Some(_))
            )
    }) {
        return Err(Error::validation(
            "overrides",
            "override response has inconsistent resource or target identity",
        ));
    }
    Ok(rules)
}

/// Fresh authoritative channel-permission policy composable with other operation policies.
/// Unknown category, hierarchy, content or rollout facts require another policy.
///
/// ```no_run
/// use rappin::{ComputedPermissionsPolicy, Api};
/// fn policy(api: &impl Api) { let _policy = ComputedPermissionsPolicy::new(api); }
/// ```
pub struct ComputedPermissionsPolicy<'a, A: Api + ?Sized> {
    api: &'a A,
}
impl<'a, A: Api + ?Sized> ComputedPermissionsPolicy<'a, A> {
    /// Bind a native or worker transport for fresh reads on each assessment.
    ///
    /// ```no_run
    /// # use rappin::{ComputedPermissionsPolicy, Api};
    /// fn policy(api: &impl Api) { let _policy = ComputedPermissionsPolicy::new(api); }
    /// ```
    pub fn new(api: &'a A) -> Self {
        Self { api }
    }
}
#[async_trait::async_trait]
impl<A: Api + ?Sized> OperationPolicy for ComputedPermissionsPolicy<'_, A> {
    async fn check(&self, question: &AccessQuestion<'_>) -> Result<CheckResolution> {
        if question.check != ExternalCheck::ChannelPermissions {
            return Ok(CheckResolution::unknown(
                "this fact requires a separate operation policy",
            ));
        }
        let (Some(actor), Some(channel)) = (question.actor_id, question.channel_id) else {
            return Ok(CheckResolution::unknown(
                "actor and channel identity are required",
            ));
        };
        let access = self
            .api
            .get_member_permissions(question.server_id, actor, Some(channel))
            .await?;
        let mut unknown = false;
        for permission in question.permissions {
            match access.permission(permission) {
                Some(true) => (),
                Some(false) => {
                    return Ok(CheckResolution::denied(format!(
                        "computed channel permission {permission} is denied"
                    )))
                }
                None => unknown = true,
            }
        }
        if unknown {
            Ok(CheckResolution::unknown(
                "computed response omitted a required permission",
            ))
        } else {
            Ok(CheckResolution::allowed(
                "fresh computed channel permissions allow this actor",
            ))
        }
    }
}

pub(crate) fn role_order_ids(body: &Value) -> Result<Vec<Uuid>> {
    let ids: Vec<Uuid> = if body.is_array() {
        serde_json::from_value::<Vec<RolePosition>>(body.clone())?
            .into_iter()
            .map(|position| position.id)
            .collect()
    } else {
        serde_json::from_value(body["ids"].clone())?
    };
    validate_role_order(&ids)?;
    Ok(ids)
}
