use serde::{Deserialize, Serialize};

/// A registered bot slash command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Command name (lowercase, 1-32 chars, regex `^[a-z0-9_-]{1,32}$`).
    pub name: String,
    /// Human readable command description.
    pub description: String,
}

impl Command {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

/// Request/response container for list of commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandList {
    pub commands: Vec<Command>,
}

impl CommandList {
    pub fn new(commands: impl IntoIterator<Item = Command>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
        }
    }
}

