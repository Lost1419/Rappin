use crate::{
    Command, InteractionEvent, MemberJoinEvent, MemberLeaveEvent, MessageEvent, Result, YappinError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) const VERSION: u32 = 7;
const MAX_FRAME: u64 = 1024 * 1024;

/// Optional listeners implemented by a worker in addition to commands and buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Listener {
    Message,
    DmMessage,
    ReactionAdd,
    ReactionRemove,
    Interaction,
    MemberJoin,
    MemberLeave,
    InviteCreate,
    InviteDelete,
    /// Opt-in fallible server-message listener with explicit completion.
    /// Appended to preserve the discriminants of existing public variants.
    ServerMessage,
    /// API 1.3 values; opt-in parent protocol v7.
    UiInteraction,
}

/// Serializable handler input. The bot credential is never part of this message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CogEvent {
    DmMessage {
        message: MessageEvent,
    },
    ReactionAdd {
        event: crate::ReactionEvent,
    },
    ReactionRemove {
        event: crate::ReactionEvent,
    },
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
    InviteCreate {
        event: crate::InviteEvent,
    },
    InviteDelete {
        event: crate::InviteEvent,
    },
    UiInteraction {
        event: crate::UiInteraction,
    },
}
impl CogEvent {
    pub fn message(&self) -> Option<&MessageEvent> {
        match self {
            Self::Command { message, .. }
            | Self::Message { message }
            | Self::DmMessage { message } => Some(message),
            _ => None,
        }
    }
    pub fn interaction(&self) -> Option<&InteractionEvent> {
        match self {
            Self::Button { interaction, .. } | Self::Interaction { interaction } => {
                Some(interaction)
            }
            Self::UiInteraction { event } => Some(&event.interaction),
            _ => None,
        }
    }
    pub fn args(&self) -> &str {
        match self {
            Self::Command { args, .. } => args,
            Self::UiInteraction { event } => event.args.as_deref().unwrap_or(""),
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
        if let Self::ReactionAdd { event } | Self::ReactionRemove { event } = self {
            return Some(event.user_id);
        }
        self.message()
            .map(|m| m.author_id)
            .or_else(|| self.interaction().and_then(|i| i.user_id))
    }
    pub fn channel_id(&self) -> Option<uuid::Uuid> {
        if let Self::ReactionAdd { event } | Self::ReactionRemove { event } = self {
            return Some(event.channel_id);
        }
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
        for info in &self.help {
            info.validate()?;
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
        if self.listeners.contains(&Listener::Message)
            && self.listeners.contains(&Listener::ServerMessage)
        {
            return Err(YappinError::validation(
                "cog.listeners",
                "choose legacy Message or fallible ServerMessage, not both",
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
pub(super) struct UploadDescriptor {
    pub name: String,
    pub size: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Wire {
    ApiOnce {
        method: String,
        path: String,
        body: Option<Value>,
    },
    Public {
        path: String,
    },
    Upload {
        path: String,
        body: Value,
        files: Vec<UploadDescriptor>,
    },
    UploadChunk {
        file: usize,
        data: String,
    },
    UploadFinish,
    Wait {
        filter: crate::collectors::EventFilter,
        timeout_ms: u64,
        prompt: Option<crate::CreateMessage>,
        channel: Option<uuid::Uuid>,
    },
    EventResult {
        event: Option<Box<crate::Event>>,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_error: Option<crate::RemoteError>,
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
    Permissions {
        server: uuid::Uuid,
        actor: uuid::Uuid,
        prefer_cache: bool,
    },
    ApiResult {
        value: Option<Value>,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_error: Option<crate::RemoteError>,
    },
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        disposition: Option<crate::MessageDisposition>,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_error: Option<crate::RemoteError>,
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

/// Receive a bounded upload before executing any HTTP mutation.
pub(super) async fn read_upload<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    descriptors: Vec<UploadDescriptor>,
) -> Result<Vec<crate::UploadFile>> {
    use base64::Engine;
    if descriptors.is_empty()
        || descriptors.len() > 4
        || descriptors
            .iter()
            .any(|f| f.size == 0 || f.size > 10 * 1024 * 1024 || f.name.len() > 255)
        || descriptors.iter().map(|f| f.size).sum::<usize>() > 12 * 1024 * 1024
    {
        return Err(YappinError::validation(
            "upload",
            "invalid bounded worker upload metadata",
        ));
    }
    let mut buffers: Vec<Vec<u8>> = descriptors
        .iter()
        .map(|f| Vec::with_capacity(f.size))
        .collect();
    loop {
        match read(reader).await? {
            Wire::UploadChunk { file, data } => {
                if file >= buffers.len() || data.len() > 87384 {
                    return Err(protocol_error());
                }
                let chunk = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| protocol_error())?;
                if chunk.is_empty()
                    || chunk.len() != (descriptors[file].size - buffers[file].len()).min(65536)
                {
                    return Err(protocol_error());
                }
                buffers[file].extend_from_slice(&chunk);
            }
            Wire::UploadFinish => break,
            _ => return Err(protocol_error()),
        }
    }
    if buffers
        .iter()
        .zip(&descriptors)
        .any(|(b, d)| b.len() != d.size)
    {
        return Err(protocol_error());
    }
    let files: Vec<_> = descriptors
        .into_iter()
        .zip(buffers)
        .map(|(d, b)| crate::UploadFile::new(d.name, b))
        .collect();
    crate::validate_uploads(&files)?;
    Ok(files)
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    use base64::Engine;
    fn descriptor(size: usize) -> Vec<UploadDescriptor> {
        vec![UploadDescriptor {
            name: "a.txt".into(),
            size,
        }]
    }
    fn stream(frames: Vec<Wire>) -> Vec<u8> {
        frames
            .into_iter()
            .flat_map(|frame| {
                let mut data = serde_json::to_vec(&frame).unwrap();
                data.push(b'\n');
                data
            })
            .collect()
    }
    #[tokio::test]
    async fn upload_frames_are_bounded_and_incomplete_transfers_never_complete() {
        let bytes = stream(vec![
            Wire::UploadChunk {
                file: 0,
                data: base64::engine::general_purpose::STANDARD.encode(b"abc"),
            },
            Wire::UploadFinish,
        ]);
        let files = read_upload(&mut bytes.as_slice(), descriptor(3))
            .await
            .unwrap();
        assert_eq!(files[0].data.as_ref(), b"abc");
        assert!(
            read_upload(&mut (&b""[..]), descriptor(12 * 1024 * 1024 + 1))
                .await
                .is_err()
        );
        for frames in [
            vec![Wire::UploadFinish],
            vec![
                Wire::UploadChunk {
                    file: 1,
                    data: "YQ==".into(),
                },
                Wire::UploadFinish,
            ],
            vec![Wire::UploadChunk {
                file: 0,
                data: "YQ==".into(),
            }],
        ] {
            let bytes = stream(frames);
            assert!(read_upload(&mut bytes.as_slice(), descriptor(3))
                .await
                .is_err());
        }
    }
}
