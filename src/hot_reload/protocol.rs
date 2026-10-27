use crate::{
    Command, InteractionEvent, MemberJoinEvent, MemberLeaveEvent, MessageEvent, Result, YappinError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) const VERSION: u32 = 2;
const MAX_FRAME: u64 = 1024 * 1024;

/// Optional listeners implemented by a worker in addition to commands and buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Listener {
    Message,
    Interaction,
    MemberJoin,
    MemberLeave,
}

/// Serializable handler input. The bot credential is never part of this message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CogEvent {
    Command {
        command: String,
        args: String,
        argv: Vec<String>,
        message: MessageEvent,
    },
    Button {
        prefix: String,
        interaction: InteractionEvent,
    },
    Message {
        message: MessageEvent,
    },
    Interaction {
        interaction: InteractionEvent,
    },
    MemberJoin {
        event: MemberJoinEvent,
    },
    MemberLeave {
        event: MemberLeaveEvent,
    },
}
impl CogEvent {
    pub fn message(&self) -> Option<&MessageEvent> {
        match self {
            Self::Command { message, .. } | Self::Message { message } => Some(message),
            _ => None,
        }
    }
    pub fn interaction(&self) -> Option<&InteractionEvent> {
        match self {
            Self::Button { interaction, .. } | Self::Interaction { interaction } => {
                Some(interaction)
            }
            _ => None,
        }
    }
    pub fn args(&self) -> &str {
        match self {
            Self::Command { args, .. } => args,
            _ => "",
        }
    }
    pub fn argv(&self) -> &[String] {
        match self {
            Self::Command { argv, .. } => argv,
            _ => &[],
        }
    }
    pub fn author_id(&self) -> Option<uuid::Uuid> {
        self.message()
            .map(|m| m.author_id)
            .or_else(|| self.interaction().and_then(|i| i.user_id))
    }
    pub fn channel_id(&self) -> Option<uuid::Uuid> {
        self.message()
            .map(|m| m.channel_id)
            .or_else(|| self.interaction().and_then(|i| i.channel_id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub name: String,
    pub commands: Vec<Command>,
    pub buttons: Vec<String>,
    pub listeners: Vec<Listener>,
    #[serde(default)]
    pub help: Vec<crate::commands::CommandInfo>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(YappinError::validation(
                "cog.name",
                "requires 1 to 64 letters, digits, _ or -",
            ));
        }
        crate::validation::validate_commands(&self.commands)?;
        let mut help_names = std::collections::HashSet::new();
        if self.help.iter().any(|info| {
            !self
                .commands
                .iter()
                .any(|command| command.name == info.name)
                || !info.aliases.is_empty()
                || !help_names.insert(&info.name)
        }) {
            return Err(YappinError::validation(
                "cog.help",
                "help entries require unique existing command names",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        if self.buttons.len() > 100
            || self
                .buttons
                .iter()
                .any(|b| b.is_empty() || b.len() > 100 || !seen.insert(b))
        {
            return Err(YappinError::validation(
                "cog.buttons",
                "requires unique nonempty prefixes, up to 100",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        if self.listeners.iter().any(|kind| !seen.insert(kind)) {
            return Err(YappinError::validation(
                "cog.listeners",
                "duplicate listener",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Wire {
    Wait {
        filter: crate::collectors::EventFilter,
        timeout_ms: u64,
        prompt: Option<crate::CreateMessage>,
        channel: Option<uuid::Uuid>,
    },
    EventResult {
        event: Option<Box<crate::Event>>,
        error: Option<String>,
    },
    Shutdown,
    Ready {
        protocol: u32,
        manifest: Manifest,
    },
    Invoke {
        event: Box<CogEvent>,
    },
    Api {
        method: String,
        path: String,
        body: Option<Value>,
    },
    ApiResult {
        value: Option<Value>,
        error: Option<String>,
    },
    Done {
        error: Option<String>,
    },
}
pub(super) fn protocol_error() -> YappinError {
    YappinError::Config("cog worker protocol failed; old routing is preserved during reload".into())
}
pub(super) async fn read<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Wire> {
    let mut bytes = Vec::new();
    (&mut *reader)
        .take(MAX_FRAME + 1)
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.is_empty() {
        return Err(YappinError::Config(
            "cog worker exited or closed its output".into(),
        ));
    }
    if bytes.len() as u64 > MAX_FRAME || bytes.last() != Some(&b'\n') {
        return Err(YappinError::Config(
            "cog IPC frame exceeds 1 MiB or is incomplete".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| protocol_error())
}
pub(super) async fn write<W: AsyncWrite + Unpin>(writer: &mut W, message: &Wire) -> Result<()> {
    let bytes = serde_json::to_vec(message)?;
    if bytes.len() as u64 >= MAX_FRAME {
        return Err(YappinError::Config("cog IPC frame exceeds 1 MiB".into()));
    }
    writer.write_all(&bytes).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}
