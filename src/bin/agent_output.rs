use super::*;
pub use rappin::capabilities::output::{apply, validate, Limits};
pub fn collection(command: &Command) -> Option<&'static str> {
    match command {
        Command::Platform { operation, .. } => match operation.as_str() {
            "list_emojis" => Some("emojis"),
            "list_channel_webhooks" | "list_bot_webhooks" => Some("webhooks"),
            _ => None,
        },
        Command::Channels {
            action: Channels::Search { .. },
        }
        | Command::Members {
            action: Members::Search { .. },
        } => Some("matches"),
        Command::Servers {
            action: Servers::Invites { .. },
        } => Some("invites"),
        Command::Servers {
            action: Servers::Roles { .. },
        } => Some("roles"),
        Command::Servers {
            action: Servers::ScheduledEvents { .. },
        } => Some("events"),
        Command::Channels {
            action: Channels::Permissions { .. },
        } => Some("overrides"),
        Command::Servers {
            action: Servers::List,
        } => Some("servers"),
        Command::Channels {
            action: Channels::List { .. },
        } => Some("channels"),
        Command::Members {
            action: Members::List { .. },
        } => Some("members"),
        Command::Messages {
            action: Messages::List { .. },
        } => Some("messages"),
        _ => None,
    }
}
