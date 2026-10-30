#![cfg(feature = "cli")]
mod support;
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

#[test]
fn mcp_rejects_tool_selected_files_and_never_overwrites_them() {
    use std::io::Write;
    use std::process::Stdio;
    let (mut command, config) = command();
    let file = std::env::temp_dir().join(format!("rappin-private-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, "\"private_file_contents\"").unwrap();
    let mut child = command
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for (id, name, args) in [
        (1, "poll_events", serde_json::json!({"cursor_file":file})),
        (
            2,
            "ack_events",
            serde_json::json!({"id":1,"cursor_file":file}),
        ),
        (3, "ack_events", serde_json::json!({"id":1})),
    ] {
        writeln!(stdin,"{}",serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("private_file_contents"));
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["result"]["isError"], true);
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "\"private_file_contents\""
    );
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(config).unwrap();
}

#[test]
fn mcp_state_is_fixed_at_startup_instead_of_tool_arguments() {
    use std::io::Write;
    use std::process::Stdio;
    let (mut command, config) = command();
    let file = std::env::temp_dir().join(format!("rappin-cursor-{}.json", uuid::Uuid::new_v4()));
    let mut child = command
        .arg("mcp")
        .arg("--cursor-file")
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(),"{}",serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ack_events","arguments":{"id":42}}})).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["result"]["isError"], true);
    assert!(
        !file.exists(),
        "undelivered acknowledgements must not create checkpoints"
    );
    let _ = std::fs::remove_file(format!("{}.lock", file.display()));
    std::fs::remove_file(config).unwrap();
}

#[tokio::test]
async fn mcp_defaults_omitted_arguments_but_rejects_explicit_invalid_values() {
    use serde_json::{json, Value};
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    let (base, mut requests, server) = support::mock(vec![
        support::response(200, json!({"servers": []})),
        support::response(200, json!({"servers": []})),
    ])
    .await;
    let (mut command, config) = command();
    command
        .env("YAPPER_BOT_TOKEN", "yb_fixture")
        .args(["--base-url", &base, "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = tokio::process::Command::from(command).spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_servers"}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_servers","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_servers","arguments":null}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"resolve_channel"}}),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_servers","arguments":[]}}),
    ] {
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    drop(input);
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    let mut replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    replies.sort_by_key(|value| value["id"].as_u64());
    assert_eq!(replies.len(), 6);
    for reply in &replies[1..3] {
        assert_eq!(reply["result"]["isError"], false);
        let result: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(result["data"]["servers"], json!([]));
    }
    for (reply, field) in replies[3..].iter().zip(["arguments", "name", "arguments"]) {
        assert_eq!(reply["result"]["isError"], true);
        let result: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(result["error"]["field"], field);
    }
    for _ in 0..2 {
        assert!(requests.recv().await.unwrap().starts_with("GET /servers "));
    }
    server.await.unwrap();
    std::fs::remove_file(config).unwrap();
}
