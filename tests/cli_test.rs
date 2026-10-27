#![cfg(feature = "cli")]
use std::{path::PathBuf, process::Command};
fn command() -> (Command, PathBuf) {
    let config = std::env::temp_dir().join(format!("rappin-cli-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(&config, "").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rappin"));
    command
        .env_remove("YAPPER_BOT_TOKEN")
        .env_remove("YAPPER_API_BASE")
        .arg("--config")
        .arg(&config);
    (command, config)
}
#[test]
fn doctor_without_auth_is_useful_json() {
    let (mut c, p) = command();
    let out = c.args(["--json", "doctor", "--offline"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(out.status.success());
    assert_eq!(v["data"]["auth_source"], "missing");
    assert_eq!(v["data"]["token_available"], false);
    std::fs::remove_file(p).unwrap();
}
#[test]
fn dry_run_needs_no_token_and_does_not_contact_server() {
    let (mut c, p) = command();
    let out = c
        .args([
            "--base-url",
            "http://127.0.0.1:1",
            "--dry-run",
            "messages",
            "send",
            "00000000-0000-0000-0000-000000000000",
            "--content",
            "hello",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["messages"][0]["body"]["content"], "hello");
    assert_eq!(v["data"]["messages"][0]["dry_run"], true);
    std::fs::remove_file(p).unwrap();
}
#[test]
fn invalid_arguments_and_payloads_are_structured() {
    let (mut c, p) = command();
    let out = c
        .args(["--json", "messages", "list", "not-a-uuid"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "arguments");
    std::fs::remove_file(p).unwrap();
}
#[test]
fn help_lists_major_capabilities() {
    let output = Command::new(env!("CARGO_BIN_EXE_rappin"))
        .arg("--help")
        .output()
        .unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    for command in [
        "doctor", "schema", "servers", "channels", "members", "messages", "events", "request",
    ] {
        assert!(help.contains(command));
    }
}

#[test]
fn mcp_lists_tools_and_previews_writes_without_auth() {
    use std::io::Write;
    use std::process::Stdio;
    let (mut command, path) = command();
    let mut child = command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send_message","arguments":{"channel":"00000000-0000-0000-0000-000000000000","content":"preview","dry_run":true}}}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for request in requests {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let values: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(values.len(), 3);
    assert_eq!(values[0]["result"]["serverInfo"]["name"], "rappin");
    assert!(values[1]["result"]["tools"].as_array().unwrap().len() >= 7);
    let result: serde_json::Value =
        serde_json::from_str(values[2]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["data"]["messages"][0]["dry_run"], true);
    std::fs::remove_file(path).unwrap();
}
