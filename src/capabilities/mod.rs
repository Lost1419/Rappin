//! Offline capabilities, validated operation bindings and host-owned authorization.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod authorization;
mod catalog;
mod commands;
mod execution;
mod platform;
pub use platform::OperationRequest;
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
#[non_exhaustive]
/// Broad capability family for offline inventory filtering.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub enum CapabilityDomain {
    /// Server discovery and metadata.
    Servers,
    /// Channel/category lookup and management.
    Channels,
    /// Membership lookup and moderation.
    Members,
    /// Message history, publishing and interaction operations.
    Messages,
    /// Role inventory and mutations.
    Roles,
    /// Server/resource permission management.
    Permissions,
    /// Event delivery and scheduled events.
    Events,
    /// Transport and service metadata.
    Diagnostics,
    /// Custom emoji management.
    Emojis,
    /// Bot-owned channel webhook operations.
    Webhooks,
    /// Offline capability/schema discovery.
    Schema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "argument", rename_all = "snake_case")]
#[non_exhaustive]
/// Argument/resource scope a validated binding must establish before execution.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub enum OperationScope {
    /// Bind the named server argument to the host server.
    Server(#[doc = "Argument key carrying the server UUID."] &'static str),
    /// Resolve the named channel argument and verify its server.
    Channel(#[doc = "Argument key carrying the channel UUID."] &'static str),
    /// Bind an optional server filter; execution still enforces the host scope.
    OptionalServer(#[doc = "Optional server-filter argument key."] &'static str),
    /// Scope a list response to the host server.
    ServerList,
    /// No API access is required.
    Offline,
    /// Public metadata read without bot authentication.
    Public,
    /// Requires CLI-owned runtime state; not executable by a scoped library binding.
    CliRuntime,
    /// Requires a global event consumer, not a server-bound command.
    GlobalEvents,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Runtime context required by a capability.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub enum LibrarySupport {
    /// Supported through the shared Api interface with scoped authorization.
    ScopedApi,
    /// Pure offline schema/capability operation.
    Offline,
    /// Requires CLI-specific state or diagnostics.
    CliRuntime,
    /// Requires global intake and cursor ownership.
    GlobalEventConsumer,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
/// Library support and documented rollout restrictions; not proof of current server eligibility.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub struct Availability {
    /// Whether the pinned endpoint contract restricts this operation to test servers.
    pub test_servers_only: bool,
    /// Supported execution surface for this operation.
    pub library: LibrarySupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "permissions", rename_all = "snake_case")]
/// Server-wide grant rule evaluated independently for requester and bot.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub enum PermissionRule {
    /// No server grant required by this rule; separate scope/external checks still apply.
    NotRequired,
    /// Require membership in the host server.
    Member,
    /// Require every listed server-wide permission.
    All(#[doc = "Every required server-wide permission name."] Vec<&'static str>),
    /// Require at least one listed server-wide permission.
    Any(#[doc = "Alternative server-wide permission names."] Vec<&'static str>),
    /// Allow the message author or the named permission.
    MessageAuthorOr(
        #[doc = "Permission required when the actor is not the message author."] &'static str,
    ),
    /// Require that the message belongs to the authenticated bot.
    BotMessageAuthor,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Independent principal grants and additional resource/availability checks.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub struct PermissionRequirements {
    /// Server-wide requirement for the original authenticated requester.
    pub requester: PermissionRule,
    /// Independent server-wide requirement for the bot.
    pub bot: PermissionRule,
    /// Effective channel permissions the requester must independently possess.
    pub requester_channel: Vec<&'static str>,
    /// Effective channel permissions the bot must independently possess.
    pub bot_channel: Vec<&'static str>,
    /// Facts requiring host policy or additional authoritative evidence.
    pub additional_checks: Vec<ExternalCheck>,
}

/// Filtering changes the model-visible inventory, never execution authority.
#[derive(Debug, Clone)]
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
pub struct CapabilityFilter {
    /// Allowed operation names; an empty vector does not restrict names.
    pub names: Vec<String>,
    /// Selected families; an empty vector does not restrict domains.
    pub domains: Vec<CapabilityDomain>,
    /// Some(true) selects writes, Some(false) selects reads, None selects both.
    pub write: Option<bool>,
    /// Exclude operations requiring CLI/global consumer state.
    pub library_only: bool,
    /// Include entries marked test-server-only; inclusion is not an availability grant.
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
/// Borrow the shared offline catalogue without authentication, network access or CLI features.
///
/// # Example
///
/// ```rust
/// let entries = rappin::capabilities::catalogue();
/// assert!(!entries.is_empty());
/// ```
pub fn catalogue() -> &'static [Capability] {
    catalog::entries()
}
/// Look up an exact operation name in the offline catalogue.
///
/// # Example
///
/// ```rust
/// let entry = rappin::capabilities::capability("send_message").unwrap();
/// assert!(entry.write);
/// ```
pub fn capability(name: &str) -> Option<&'static Capability> {
    catalogue().iter().find(|c| c.name == name)
}
/// Select model-visible metadata without changing execution authority.
///
/// # Example
///
/// ```rust
/// use rappin::capabilities::{CapabilityDomain, CapabilityFilter, filtered_catalogue};
/// let filter = CapabilityFilter {
///     domains: vec![CapabilityDomain::Messages], write: Some(false), library_only: true,
///     ..Default::default()
/// };
/// for operation in filtered_catalogue(&filter) {
///     assert!(!operation.write);
///     println!("{}: {}", operation.name, operation.description);
///     // Discovery metadata is not authorization to execute.
/// }
/// ```
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
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::{capability, CapabilityFilter};
    /// use serde_json::json;
    /// let filter = CapabilityFilter { names: vec!["send_message".into()], library_only: true, ..Default::default() };
    /// let operation = filter.validate("send_message", &json!({"channel":uuid::Uuid::nil(), "content":"Hello"}))?;
    /// assert_eq!(operation.name(), "send_message");
    /// assert!(operation.preview().dry_run);
    /// assert!(operation.preview().authorization.is_none());
    /// # Ok::<(), rappin::Error>(())
    /// ```
    pub fn validate(&self, name: &str, args: &Value) -> Result<ValidatedOperation> {
        let descriptor = filtered_catalogue(self)
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| {
                Error::validation(
                    "operation",
                    "operation is not enabled in this capability selection",
                )
            })?;
        descriptor.validate(args)
    }
}
/// Return the shared success/error tool envelope schema.
///
/// # Example
///
/// ```rust
/// let schema = rappin::capabilities::output_schema();
/// assert_eq!(schema["type"], "object");
/// ```
pub fn output_schema() -> Value {
    json!({"type":"object","properties":{"ok":{"type":"boolean"},"data":{"type":"object"},"error":{"type":"object"}},"required":["ok"],"additionalProperties":false,"oneOf":[{"properties":{"ok":{"const":true}},"required":["data"]},{"properties":{"ok":{"const":false}},"required":["error"]}]})
}
/// Render all catalogue entries as JSON tool definitions; consumers should apply their own operation selection.
///
/// # Example
///
/// ```rust
/// let tools = rappin::capabilities::tools();
/// assert!(tools.iter().any(|tool| tool["name"] == "send_message"));
/// ```
pub fn tools() -> Vec<Value> {
    catalogue()
        .iter()
        .map(Capability::tool_definition)
        .collect()
}

impl Capability {
    /// Render canonical input properties, required fields and supported cross-field constraints.
    ///
    /// # Example
    ///
    /// ```rust
    /// let entry = rappin::capabilities::capability("send_message").unwrap();
    /// assert_eq!(entry.input_schema()["type"], "object");
    /// ```
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
    /// Render input/output schemas, hints, scope, permission and availability metadata for one operation.
    ///
    /// # Example
    ///
    /// ```rust
    /// let entry = rappin::capabilities::capability("send_message").unwrap();
    /// assert_eq!(entry.tool_definition()["name"], "send_message");
    /// ```
    pub fn tool_definition(&self) -> Value {
        json!({"name":self.name,"description":self.description,"inputSchema":self.input_schema(),"outputSchema":output_schema(),"annotations":{"readOnlyHint":!self.write,"destructiveHint":self.destructive,"openWorldHint":!matches!(self.availability.library,LibrarySupport::Offline|LibrarySupport::CliRuntime)},"x-rappin-permissions":self.permissions,"x-rappin-availability":self.availability,"x-rappin-scope":self.scope})
    }
    /// Offline, typed validation. The returned binding owns canonical arguments.
    /// # Example
    ///
    /// ```rust
    /// use rappin::capabilities::{capability, CapabilityFilter};
    /// use serde_json::json;
    /// let filter = CapabilityFilter { names: vec!["send_message".into()], library_only: true, ..Default::default() };
    /// let operation = filter.validate("send_message", &json!({"channel":uuid::Uuid::nil(), "content":"Hello"}))?;
    /// assert_eq!(operation.name(), "send_message");
    /// assert!(operation.preview().dry_run);
    /// assert!(operation.preview().authorization.is_none());
    /// # Ok::<(), rappin::Error>(())
    /// ```
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
        "list_roles" | "get_role" | "reorder_roles" | "create_role" | "edit_role"
        | "delete_role" => (S::Server("id"), D::Roles),
        "get_member_permissions"
        | "set_everyone_permissions"
        | "set_category_permissions"
        | "clear_category_permissions" => (S::Server("id"), D::Permissions),
        "get_category_overrides" | "set_category_override" | "delete_category_override" => {
            (S::Server("id"), D::Permissions)
        }
        "resolve_id" => (S::Server("server"), D::Schema),
        "get_channel_member_permissions"
        | "get_channel_overrides"
        | "set_channel_override"
        | "delete_channel_override" => (S::Channel("channel"), D::Permissions),
        "send_sticker" => (S::Channel("channel"), D::Messages),
        "list_invites" => (S::Server("id"), D::Members),
        "list_server_events" | "create_server_event" => (S::Server("id"), D::Events),
        "read_messages" | "get_message" | "get_message_emojis" | "send_message"
        | "edit_message" | "delete_message" | "add_reaction" | "remove_reaction"
        | "pin_message" | "unpin_message" => (S::Channel("channel"), D::Messages),
        "list_emojis" | "create_emoji" | "edit_emoji" | "delete_emoji" => {
            (S::Server("id"), D::Emojis)
        }
        "list_bot_webhooks" => (S::ServerList, D::Webhooks),
        "list_channel_webhooks"
        | "create_bot_webhook"
        | "delete_bot_webhook"
        | "post_bot_webhook" => (S::Channel("channel"), D::Webhooks),
        "send_files" => (S::Channel("channel"), D::Messages),
        "get_api_version" | "get_api_changelog" => (S::Public, D::Diagnostics),
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
        "create_emoji" | "edit_emoji" | "delete_emoji" => {
            permissions.requester = P::All(vec!["manage_emojis"]);
            permissions.bot = permissions.requester.clone();
        }
        "list_channel_webhooks"
        | "list_bot_webhooks"
        | "create_bot_webhook"
        | "delete_bot_webhook"
        | "post_bot_webhook" => {
            permissions.requester = P::All(vec!["manage_webhooks"]);
            permissions.bot = permissions.requester.clone();
            if matches!(scope, S::Channel(_)) {
                permissions.requester_channel.push("manage_webhooks");
                permissions.bot_channel.push("manage_webhooks");
            }
            if name == "list_bot_webhooks" {
                permissions
                    .additional_checks
                    .push(ExternalCheck::ChannelPermissions);
            }
            if name == "post_bot_webhook" {
                permissions
                    .additional_checks
                    .push(ExternalCheck::MessageContentRules);
            }
        }
        "get_message_emojis" => {
            permissions.requester_channel.push("read_history");
            permissions.bot_channel.push("read_history");
        }
        "send_files" => {
            permissions
                .requester_channel
                .extend(["send_messages", "attach_files"]);
            permissions
                .bot_channel
                .extend(["send_messages", "attach_files"]);
            permissions
                .additional_checks
                .push(ExternalCheck::MessageContentRules);
        }
        "send_message" | "send_sticker" => {
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
        "create_role"
        | "edit_role"
        | "delete_role"
        | "reorder_roles"
        | "set_everyone_permissions" => {
            permissions.requester = P::All(vec!["manage_roles"]);
            permissions.bot = permissions.requester.clone();
            permissions
                .additional_checks
                .push(ExternalCheck::RoleHierarchy);
            if !matches!(name, "delete_role" | "reorder_roles") {
                permissions
                    .additional_checks
                    .push(ExternalCheck::GrantablePermissions);
            }
        }
        "get_channel_overrides"
        | "set_channel_override"
        | "delete_channel_override"
        | "get_category_overrides"
        | "set_category_override"
        | "delete_category_override"
        | "get_channel_permissions"
        | "set_channel_permissions"
        | "clear_channel_permissions"
        | "set_category_permissions"
        | "clear_category_permissions" => {
            permissions.requester = P::Any(vec!["manage_roles", "manage_channels"]);
            permissions.bot = permissions.requester.clone();
            if !matches!(
                name,
                "get_channel_permissions" | "get_channel_overrides" | "get_category_overrides"
            ) {
                permissions
                    .additional_checks
                    .push(ExternalCheck::OverrideAuthority);
            }
            if name.contains("category") {
                permissions
                    .additional_checks
                    .push(ExternalCheck::CategoryScope);
                if matches!(
                    name,
                    "get_category_overrides" | "set_category_override" | "delete_category_override"
                ) {
                    permissions
                        .additional_checks
                        .push(ExternalCheck::CategoryPermissions);
                }
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
        "resolve_id"
            | "get_member_permissions"
            | "get_channel_member_permissions"
            | "get_role"
            | "reorder_roles"
            | "get_channel_overrides"
            | "set_channel_override"
            | "delete_channel_override"
            | "get_category_overrides"
            | "set_category_override"
            | "delete_category_override"
            | "send_sticker"
            | "get_message_emojis"
            | "send_files"
            | "list_emojis"
            | "create_emoji"
            | "edit_emoji"
            | "delete_emoji"
            | "list_bot_webhooks"
            | "list_channel_webhooks"
            | "create_bot_webhook"
            | "delete_bot_webhook"
            | "post_bot_webhook"
            | "get_member_counts"
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
    if matches!(scope, S::Offline | S::Public) {
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
