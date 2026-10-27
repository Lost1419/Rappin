#![cfg(all(feature = "voice", unix))]
mod support;
use rappin::{voice::VoiceOptions, Client};
use std::{os::unix::fs::PermissionsExt, time::Duration};
#[tokio::test]
async fn voice_bridge_pads_frames_drains_and_leaves() {
    if std::env::var_os("RAPPIN_VOICE_ENV_TEST").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "voice_bridge_pads_frames_drains_and_leaves",
                "--nocapture",
            ])
            .env("RAPPIN_VOICE_ENV_TEST", "1")
            .env("YAPPER_BOT_TOKEN", "yb_environment_fixture")
            .env("CARGO_REGISTRY_TOKEN", "cargo_environment_fixture")
            .env("NODE_OPTIONS", "--require=/nonexistent/untrusted-hook.cjs")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("node unavailable; optional voice bridge runtime test skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rappin-voice-{}", uuid::Uuid::new_v4()));
    let modules = dir.join("node_modules/@livekit/rtc-node");
    tokio::fs::create_dir_all(&modules).await.unwrap();
    tokio::fs::write(
        modules.join("package.json"),
        r#"{"type":"module","main":"index.js"}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(modules.join("index.js"),"export class AudioFrame { constructor(pcm, rate, channels, samples) { if (pcm.length !== 960 || rate !== 48000 || channels !== 2 || samples !== 480) throw Error('bad frame'); }}").await.unwrap();
    let sdk = dir.join("sdk.mjs");
    let state = dir.join("state.json");
    let source = format!(
        r#"import {{writeFile}} from 'node:fs/promises';
export class Bot {{
 async connectVoice() {{ if(process.env.YAPPER_BOT_TOKEN || process.env.CARGO_REGISTRY_TOKEN || process.env.NODE_OPTIONS)throw Error('inherited credential or startup hook'); let frames=0; return {{source:{{captureFrame:async()=>{{frames++;}},waitForPlayout:async()=>{{if(frames!==2)throw Error('lost final partial frame');}}}},leave:async()=>{{await writeFile({},JSON.stringify({{frames,disconnected:true}}));}} }}; }}
 async leaveVoice() {{}}
}}"#,
        serde_json::to_string(&state).unwrap()
    );
    tokio::fs::write(&sdk, source).await.unwrap();
    let audio = dir.join("audio.bin");
    tokio::fs::write(&audio, b"fixture").await.unwrap();
    let ffmpeg = dir.join("ffmpeg");
    tokio::fs::write(&ffmpeg, "#!/bin/sh\nhead -c 2000 /dev/zero\n")
        .await
        .unwrap();
    tokio::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755))
        .await
        .unwrap();
    let mut options = VoiceOptions::new(sdk, dir.join("node_modules"));
    options.ffmpeg = ffmpeg;
    options.timeout = Duration::from_secs(10);
    Client::new("yb_fixture")
        .play_audio(uuid::Uuid::nil(), uuid::Uuid::nil(), audio, &options)
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(state).await.unwrap()).unwrap();
    assert_eq!(value["frames"], 2);
    assert_eq!(value["disconnected"], true);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
