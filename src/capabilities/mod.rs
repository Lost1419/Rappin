//! Offline capabilities, validated operation bindings and host-owned authorization.
use crate::{Result, YappinError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod authorization;
mod catalog;
mod commands;
mod execution;
pub mod output;
mod schema;
mod validation;
pub use authorization::*;
pub use catalog::{Capability, OperationArgument};
pub use commands::{OperationCommandBinding, OperationParameter};
pub use execution::{OperationResult, ValidatedOperation};
pub use schema::{bundled_schema, endpoint_schema, validate_schema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDomain {
    Servers,
    Channels,
    Members,
    Messages,
    Roles,
    Permissions,
    Events,
    Diagnostics,
    Schema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "argument", rename_all = "snake_case")]
pub enum OperationScope {
    Server(&'static str),
    Channel(&'static str),
    OptionalServer(&'static str),
    ServerList,
    Offline,
    CliRuntime,
    GlobalEvents,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySupport {
    ScopedApi,
    Offline,
    CliRuntime,
    GlobalEventConsumer,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Availability {
    pub test_servers_only: bool,
    pub library: LibrarySupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "permissions", rename_all = "snake_case")]
pub enum PermissionRule {
    NotRequired,
    Member,
    All(Vec<&'static str>),
    Any(Vec<&'static str>),
    MessageAuthorOr(&'static str),
    BotMessageAuthor,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionRequirements {
    pub requester: PermissionRule,
    pub bot: PermissionRule,
    pub requester_channel: Vec<&'static str>,
    pub bot_channel: Vec<&'static str>,
    pub additional_checks: Vec<ExternalCheck>,
}

/// Filtering changes the model-visible inventory, never execution authority.
#[derive(Debug, Clone)]
pub struct CapabilityFilter {
    pub names: Vec<String>,
    pub domains: Vec<CapabilityDomain>,
    pub write: Option<bool>,
    pub library_only: bool,
    pub include_test_servers: bool,
}
impl Default for CapabilityFilter {
    fn default() -> Self {
        Self {
            names: vec![],
            domains: vec![],
            write: None,
            library_only: false,
            include_test_servers: true,
        }
    }
}
pub fn catalogue() -> &'static [Capability] {
    catalog::entries()
}
pub fn capability(name: &str) -> Option<&'static Capability> {
    catalogue().iter().find(|c| c.name == name)
}
pub fn filtered_catalogue(filter: &CapabilityFilter) -> Vec<&'static Capability> {
    catalogue()
        .iter()
        .filter(|c| {
            (filter.names.is_empty() || filter.names.iter().any(|name| name == c.name))
                && (filter.domains.is_empty() || filter.domains.contains(&c.domain))
                && filter.write.is_none_or(|write| write == c.write)
                && (!filter.library_only
                    || matches!(
                        c.availability.library,
                        LibrarySupport::ScopedApi | LibrarySupport::Offline
                    ))
                && (filter.include_test_servers || !c.availability.test_servers_only)
        })
        .collect()
}
impl CapabilityFilter {
    /// Enforce the host-selected inventory as well as validating the operation.
    /// This remains separate from requester/bot authorization at execution time.
    pub fn validate(&self, name: &str, args: &Value) -> Result<ValidatedOperation> {
        let descriptor = filtered_catalogue(self)
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| {
                YappinError::validation(
                    "operation",
                    "operation is not enabled in this capability selection",
                )
            })?;
        descriptor.validate(args)
    }
}
pub fn output_schema() -> Value {
    json!({"type":"object","properties":{"ok":{"type":"boolean"},"data":{"type":"object"},"error":{"type":"object"}},"required":["ok"],"additionalProperties":false,"oneOf":[{"properties":{"ok":{"const":true}},"required":["data"]},{"properties":{"ok":{"const":false}},"required":["error"]}]})
}
pub fn tools() -> Vec<Value> {
    catalogue()
        .iter()
        .map(Capability::tool_definition)
        .collect()
}

impl Capability {
    pub fn input_schema(&self) -> Value {
        let properties: serde_json::Map<String, Value> = self
            .arguments
            .iter()
            .map(|f| (f.name.into(), f.schema.clone()))
            .collect();
        let required: Vec<_> = self
            .arguments
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        let mut schema = json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        if self.name == "send_message" {
            schema["oneOf"] = json!([{"required":["content"]},{"required":["body"]}]);
            schema["allOf"] = json!([{"if":{"properties":{"split":{"const":true}},"required":["split"]},"then":{"required":["content"]},"else":{"properties":{"content":{"maxLength":2000}}}}]);
        } else if self.name == "get_schema" {
            schema["oneOf"] = json!([{"required":["name"],"not":{"anyOf":[{"required":["path"]},{"required":["method"]}]}},{"required":["path","method"],"not":{"required":["name"]}}]);
        } else if self.name == "read_messages" {
            schema["not"] = json!({"required":["before","after"]});
        }
        schema
    }
    pub fn tool_definition(&self) -> Value {
        json!({"name":self.name,"description":self.description,"inputSchema":self.input_schema(),"outputSchema":output_schema(),"annotations":{"readOnlyHint":!self.write,"destructiveHint":self.destructive,"openWorldHint":!matches!(self.availability.library,LibrarySupport::Offline|LibrarySupport::CliRuntime)},"x-rappin-permissions":self.permissions,"x-rappin-availability":self.availability,"x-rappin-scope":self.scope})
    }
    /// Offline, typed validation. The returned binding owns canonical arguments.
    pub fn validate(&self, args: &Value) -> Result<ValidatedOperation> {
        validation::validate(self, args)
    }
}

fn metadata(
    name: &str,
) -> (
    OperationScope,
    PermissionRequirements,
    Availability,
    CapabilityDomain,
) {
    use CapabilityDomain as D;
    use OperationScope as S;
    use PermissionRule as P;
    let mut permissions = PermissionRequirements {
        requester: P::Member,
        bot: P::Member,
        requester_channel: vec![],
        bot_channel: vec![],
        additional_checks: vec![],
    };
    let (scope, domain) = match name {
        "list_servers" => (S::ServerList, D::Servers),
        "get_server" | "get_member_counts" => (S::Server("id"), D::Servers),
        "list_channels" | "resolve_channel" | "search_channels" => {
            (S::OptionalServer("server"), D::Channels)
        }
        "get_channel"
        | "edit_channel"
        | "delete_channel"
        | "get_channel_permissions"
        | "set_channel_permissions"
        | "clear_channel_permissions" => (S::Channel("id"), D::Channels),
        "create_channel" => (S::Server("server"), D::Channels),
        "create_category" | "edit_category" | "delete_category" => (S::Server("id"), D::Channels),
        "list_members" | "get_member" | "resolve_member" | "search_members" => {
            (S::Server("server"), D::Members)
        }
        "list_roles" | "create_role" | "edit_role" | "delete_role" => (S::Server("id"), D::Roles),
        "set_everyone_permissions" | "set_category_permissions" | "clear_category_permissions" => {
            (S::Server("id"), D::Permissions)
        }
        "list_invites" => (S::Server("id"), D::Members),
        "list_server_events" | "create_server_event" => (S::Server("id"), D::Events),
        "read_messages" | "get_message" | "send_message" | "edit_message" | "delete_message"
        | "add_reaction" | "remove_reaction" | "pin_message" | "unpin_message" => {
            (S::Channel("channel"), D::Messages)
        }
        "poll_events" => (S::GlobalEvents, D::Events),
        "ack_events" => (S::CliRuntime, D::Events),
        "doctor" => (S::CliRuntime, D::Diagnostics),
        "get_capabilities" => (S::Offline, D::Schema),
        "get_schema" => (S::Offline, D::Schema),
        _ => unreachable!("operation metadata missing: {name}"),
    };
    if matches!(scope, S::Channel(_)) {
        permissions.requester_channel.push("view_channel");
        permissions.bot_channel.push("view_channel");
    }
    match name {
        "create_channel" | "edit_channel" | "delete_channel" | "create_category"
        | "edit_category" | "delete_category" => {
            permissions.requester = P::All(vec!["manage_channels"]);
            permissions.bot = permissions.requester.clone();
            if matches!(scope, S::Channel(_)) {
                permissions.requester_channel.push("manage_channels");
                permissions.bot_channel.push("manage_channels");
            }
            if matches!(
                name,
                "create_channel" | "edit_channel" | "edit_category" | "delete_category"
            ) {
                permissions
                    .additional_checks
                    .push(ExternalCheck::CategoryPermissions);
            }
            if name == "edit_channel" {
                permissions
                    .additional_checks
                    .push(ExternalCheck::OverrideAuthority);
            }
        }
        "send_message" => {
            permissions.requester_channel.push("send_messages");
            permissions.bot_channel.push("send_messages");
            permissions
                .additional_checks
                .push(ExternalCheck::MessageContentRules);
        }
        "edit_message" => {
            permissions.requester = P::All(vec!["manage_messages"]);
            permissions.bot = P::BotMessageAuthor;
            permissions
                .additional_checks
                .push(ExternalCheck::MessageContentRules);
        }
        "delete_message" => {
            permissions.requester = P::MessageAuthorOr("manage_messages");
            permissions.bot = P::MessageAuthorOr("manage_messages");
        }
        "pin_message" | "unpin_message" => {
            permissions.requester = P::All(vec!["manage_messages"]);
            permissions.bot = permissions.requester.clone();
        }
        "add_reaction" => {
            permissions.requester_channel.push("add_reactions");
            permissions.bot_channel.push("add_reactions");
        }
        "create_role" | "edit_role" | "delete_role" | "set_everyone_permissions" => {
            permissions.requester = P::All(vec!["manage_roles"]);
            permissions.bot = permissions.requester.clone();
            permissions
                .additional_checks
                .push(ExternalCheck::RoleHierarchy);
            if name != "delete_role" {
                permissions
                    .additional_checks
                    .push(ExternalCheck::GrantablePermissions);
            }
        }
        "get_channel_permissions"
        | "set_channel_permissions"
        | "clear_channel_permissions"
        | "set_category_permissions"
        | "clear_category_permissions" => {
            permissions.requester = P::Any(vec!["manage_roles", "manage_channels"]);
            permissions.bot = permissions.requester.clone();
            if name != "get_channel_permissions" {
                permissions
                    .additional_checks
                    .push(ExternalCheck::OverrideAuthority);
            }
            if name.contains("category") {
                permissions
                    .additional_checks
                    .push(ExternalCheck::CategoryScope);
            }
        }
        "list_invites" => {
            permissions.requester = P::All(vec!["manage_server"]);
            permissions.bot = permissions.requester.clone();
        }
        "create_server_event" => {
            permissions.requester = P::Any(vec!["manage_events", "manage_server"]);
            permissions.bot = permissions.requester.clone();
            permissions
                .additional_checks
                .push(ExternalCheck::EventDestinationScope);
        }
        _ => (),
    }
    let test_servers_only = matches!(
        name,
        "get_member_counts"
            | "list_invites"
            | "create_role"
            | "edit_role"
            | "delete_role"
            | "set_everyone_permissions"
            | "get_channel_permissions"
            | "set_channel_permissions"
            | "clear_channel_permissions"
            | "set_category_permissions"
            | "clear_category_permissions"
            | "list_server_events"
            | "create_server_event"
    );
    let library = match scope {
        S::Offline => LibrarySupport::Offline,
        S::CliRuntime => LibrarySupport::CliRuntime,
        S::GlobalEvents => LibrarySupport::GlobalEventConsumer,
        _ => LibrarySupport::ScopedApi,
    };
    if matches!(scope, S::Offline) {
        permissions.requester = P::NotRequired;
        permissions.bot = P::NotRequired;
    }
    (
        scope,
        permissions,
        Availability {
            test_servers_only,
            library,
        },
        domain,
    )
}
