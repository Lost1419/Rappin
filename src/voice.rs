//! Optional encrypted playback using Node, ffmpeg, @livekit/rtc-node and the official Yapin SDK.
//! No media/crypto protocol is reimplemented here. Download the SDK to a trusted local .mjs file.
use crate::{Client, Result, YappinError};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;
#[derive(Clone, Debug)]
pub struct VoiceOptions {
    pub sdk: PathBuf,
    pub node_modules: PathBuf,
    pub node: PathBuf,
    pub ffmpeg: PathBuf,
    pub timeout: Duration,
}
impl VoiceOptions {
    pub fn new(sdk: impl Into<PathBuf>, node_modules: impl Into<PathBuf>) -> Self {
        let node_modules = node_modules.into();
        let local_ffmpeg = node_modules.join("ffmpeg-static").join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        let ffmpeg = if local_ffmpeg.is_file() {
            local_ffmpeg
        } else {
            "ffmpeg".into()
        };
        Self {
            sdk: sdk.into(),
            node_modules,
            node: "node".into(),
            ffmpeg,
            timeout: Duration::from_secs(3600),
        }
    }
}
impl Client {
    /// Joins, publishes one encrypted audio track, waits for playout and leaves, including on failure.
    /// This helper needs neither bot.run() nor command synchronization. Audio is decoded locally.
    pub async fn play_audio(
        &self,
        server: Uuid,
        channel: Uuid,
        file: impl AsRef<Path>,
        options: &VoiceOptions,
    ) -> Result<()> {
        let file = tokio::fs::canonicalize(file).await?;
        let sdk = tokio::fs::canonicalize(&options.sdk).await?;
        let node_modules = tokio::fs::canonicalize(&options.node_modules).await?;
        let config = serde_json::json!({"token":self.token(),"base_url":self.base_url(),"server":server,"channel":channel,"file":file,"sdk":sdk,"node_modules":node_modules,"ffmpeg":options.ffmpeg});
        let mut child = Command::new(&options.node)
            .args([
                "--input-type=module",
                "--eval",
                include_str!("voice_bridge.mjs"),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&serde_json::to_vec(&config)?).await?;
            stdin.shutdown().await?;
        }
        let output = match tokio::time::timeout(options.timeout, child.wait_with_output()).await {
            Ok(output) => output?,
            Err(_) => {
                let _ = self.leave_voice(server).await;
                return Err(YappinError::Config(
                    "voice playback timed out and was disconnected".into(),
                ));
            }
        };
        if !output.status.success() {
            let _ = self.leave_voice(server).await;
            let error = String::from_utf8_lossy(&output.stderr).replace(self.token(), "[REDACTED]");
            return Err(YappinError::Config(format!(
                "voice bridge failed: {}",
                error.trim()
            )));
        }
        Ok(())
    }
}
