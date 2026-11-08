//! Platform command publication metadata; executable handlers are registered through Bot, cogs or CommandSpec.
//!
//! # Example
//!
//! ```rust
//! use rappin::{Command, CommandList};
//! let list = CommandList::new([Command::new("ping", "Check the bot")]);
//! assert_eq!(list.commands.len(), 1);
//! ```
use serde::{Deserialize, Serialize};

/// A registered bot slash command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::Command;
/// let command = Command::new("ping", "Check the bot");
/// assert_eq!(command.name, "ping");
/// ```
pub struct Command {
    /// Command name (lowercase, 1-32 chars, regex `^[a-z0-9_-]{1,32}$`).
    pub name: String,
    /// Human readable command description.
    pub description: String,
}

impl Command {
    /// Create slash-command metadata. Publication validates the platform naming and description bounds.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::Command;
    /// let command = Command::new("ping", "Check the bot");
    /// assert_eq!(command.name, "ping");
    /// ```
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

/// Request/response container for list of commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{Command, CommandList};
/// let list = CommandList::new([Command::new("ping", "Check the bot")]);
/// assert_eq!(list.commands.len(), 1);
/// ```
pub struct CommandList {
    /// Commands to publish, or commands returned by the platform.
    pub commands: Vec<Command>,
}

impl CommandList {
    /// Collect command metadata in publication order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Command, CommandList};
    /// let list = CommandList::new([Command::new("ping", "Check the bot")]);
    /// assert_eq!(list.commands.len(), 1);
    /// ```
    pub fn new(commands: impl IntoIterator<Item = Command>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
        }
    }
}
