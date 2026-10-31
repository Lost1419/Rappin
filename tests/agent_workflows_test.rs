#![cfg(feature = "cli")]
mod support;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use support::*;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};
use uuid::Uuid;
fn temp(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rappin-agent-{}.{suffix}", Uuid::new_v4()))
}
fn config() -> PathBuf {
    let path = temp("toml");
    std::fs::write(&path, "").unwrap();
    path
}
fn command(path: &Path, base: Option<&str>) -> std::process::Command {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_rappin"));
    c.env_remove("YAPPER_API_BASE")
        .env_remove("YAPPER_BOT_TOKEN")
        .args(["--config", path.to_str().unwrap()]);
    if let Some(base) = base {
        c.args(["--base-url", base])
            .env("YAPPER_BOT_TOKEN", "fixture-secret");
    }
    c
}
async fn cli(path: &Path, base: Option<&str>, args: &[&str]) -> (bool, Value) {
    let out = Command::from(command(path, base))
        .args(args)
        .output()
        .await
        .unwrap();
    (
        out.status.success(),
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr))),
    )
}
struct Mcp {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    config: PathBuf,
}
impl Mcp {
    fn new(base: Option<&str>, cursor: Option<&PathBuf>) -> Self {
        let path = config();
        let mut command = Command::from(command(&path, base));
        command.arg("mcp");
        if let Some(cursor) = cursor {
            command.args(["--cursor-file", cursor.to_str().unwrap()]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            input,
            output,
            config: path,
        }
    }
    async fn send(&mut self, v: Value) {
        self.input
            .write_all(format!("{v}\n").as_bytes())
            .await
            .unwrap();
    }
    async fn call(&mut self, id: u64, name: &str, args: Value) {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})).await;
    }
    async fn recv(&mut self) -> Value {
        serde_json::from_str(
            &tokio::time::timeout(Duration::from_secs(4), self.output.next_line())
                .await
                .expect("MCP response timed out")
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }
    async fn close(mut self) {
        drop(self.input);
        assert!(self.child.wait().await.unwrap().success());
        std::fs::remove_file(self.config).unwrap();
    }
}
fn data(v: &Value) -> &Value {
    &v["result"]["structuredContent"]["data"]
}
fn cleanup(path: &Path) {
    for suffix in ["", ".pending.json", ".lock"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
async fn developer_update_tools_preview_typed_payloads_and_reject_path_targets() {
    let id = Uuid::nil().to_string();
    let mut mcp = Mcp::new(None, None);
    for (n, name, args, method, path) in [
        (
            1,
            "create_role",
            json!({"id":id,"body":{"name":"Helper","permissions":{"manage_messages":true}},"dry_run":true}),
            "POST",
            format!("/servers/{id}/roles"),
        ),
        (
            2,
            "set_channel_permissions",
            json!({"id":id,"target":"everyone","body":{"allow":[],"deny":["send_messages"]},"dry_run":true}),
            "PUT",
            format!("/channels/{id}/permissions/everyone"),
        ),
        (
            3,
            "create_server_event",
            json!({"id":id,"body":{"title":"Meetup","starts_at":"2026-11-01T12:00:00Z","location":"Outside"},"dry_run":true}),
            "POST",
            format!("/servers/{id}/events"),
        ),
    ] {
        mcp.call(n, name, args).await;
        let result = mcp.recv().await;
        assert_eq!(data(&result)["method"], method, "{result}");
        assert_eq!(data(&result)["path"], path);
    }
    mcp.call(
        4,
        "clear_channel_permissions",
        json!({"id":id,"target":"../../me","dry_run":true}),
    )
    .await;
    assert_eq!(mcp.recv().await["result"]["isError"], true);
    mcp.call(
        5,
        "get_schema",
        json!({"method":"POST","path":"/servers/{id}/events"}),
    )
    .await;
    let schema = mcp.recv().await;
    assert_eq!(
        data(&schema)["operation"]["x-yapin-test-servers-only"],
        true
    );
    assert_eq!(
        data(&schema)["operation"]["x-rappin-source"],
        "https://www.yapin.gg/developers"
    );
    mcp.close().await;
}

#[tokio::test]
async fn invite_reads_keep_identity_and_supply_client_offset_continuation() {
    let path = config();
    let id = Uuid::nil().to_string();
    let (base, mut requests, task) = mock(vec![response(
        200,
        json!({"invites":[{"code":"a","uses":2},{"code":"b","uses":3}]}),
    )])
    .await;
    let (ok, out) = cli(
        &path,
        Some(&base),
        &[
            "servers",
            "invites",
            &id,
            "--max-items",
            "1",
            "--fields",
            "uses",
        ],
    )
    .await;
    assert!(ok, "{out}");
    assert_eq!(out["data"]["invites"][0]["code"], "a");
    assert_eq!(out["data"]["next_offset"], 1);
    assert_eq!(out["data"]["truncated"], true);
    assert!(requests
        .recv()
        .await
        .unwrap()
        .starts_with(&format!("GET /servers/{id}/invites ")));
    task.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn member_search_tools_report_incompleteness_and_resolvers_refuse_candidates() {
    let members:Vec<_>=(1..=500).map(|n|json!({"id":Uuid::from_u128(n),"username":if n==1 {"target".into()} else {format!("user{n}")}})).collect();
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"members":members})),
        response(200, json!({"members":members})),
    ])
    .await;
    let mut mcp = Mcp::new(Some(&base), None);
    mcp.call(
        1,
        "search_members",
        json!({"server":Uuid::nil(),"name":"target","max_pages":1,"max_items":1}),
    )
    .await;
    let result = mcp.recv().await;
    assert_eq!(data(&result)["status"], "incomplete");
    assert_eq!(data(&result)["complete"], false);
    assert_eq!(data(&result)["scanned"], 500);
    assert_eq!(data(&result)["matches"].as_array().unwrap().len(), 1);
    mcp.call(
        2,
        "resolve_member",
        json!({"server":Uuid::nil(),"name":"target","max_pages":1}),
    )
    .await;
    let result = mcp.recv().await;
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(
        result["result"]["structuredContent"]["error"]["kind"],
        "incomplete_search"
    );
    for _ in 0..2 {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
    mcp.close().await;
}

#[tokio::test]
async fn capabilities_and_catalog_are_identical_across_cli_and_mcp() {
    let path = config();
    let (_, cap) = cli(&path, None, &["capabilities"]).await;
    assert_eq!(cap["data"]["features"]["cli"], true);
    let (_, tools) = cli(&path, None, &["tools", "list"]).await;
    let mut mcp = Mcp::new(None, None);
    mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .await;
    let result = mcp.recv().await;
    assert_eq!(result["result"]["tools"], tools["data"]["tools"]);
    assert_eq!(cap["data"]["tools"], tools["data"]["tools"]);
    assert!(tools["data"]["tools"].as_array().unwrap().len() >= 20);
    for tool in tools["data"]["tools"].as_array().unwrap() {
        assert!(tool["outputSchema"].is_object());
        assert!(tool["inputSchema"]["properties"]
            .get("cursor_file")
            .is_none());
    }
    mcp.close().await;
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn rich_payloads_and_catalog_cli_validate_without_reading_files() {
    let body = json!({"channel":Uuid::nil(),"body":{"embeds":[{"title":"Hello"}]},"dry_run":true});
    let mut mcp = Mcp::new(None, None);
    mcp.call(1, "send_message", body.clone()).await;
    let result = mcp.recv().await;
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(
        data(&result)["messages"][0]["body"]["embeds"][0]["title"],
        "Hello"
    );
    let text: Value =
        serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["result"]["structuredContent"]);
    mcp.call(2,"edit_message",json!({"channel":Uuid::nil(),"message":Uuid::nil(),"body_file":"/private/secret","dry_run":true})).await;
    assert_eq!(mcp.recv().await["result"]["isError"], true);
    mcp.call(
        3,
        "send_message",
        json!({"channel":Uuid::nil(),"body":{"content":"x"},"content":"y","dry_run":true}),
    )
    .await;
    assert_eq!(mcp.recv().await["result"]["isError"], true);
    mcp.close().await;
    let path = config();
    let file = temp("json");
    std::fs::write(&file, body.to_string()).unwrap();
    let (ok, cli) = cli(
        &path,
        None,
        &[
            "tools",
            "call",
            "send_message",
            "--arguments-file",
            file.to_str().unwrap(),
        ],
    )
    .await;
    assert!(ok);
    assert_eq!(cli["data"], *data(&result));
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn receipts_survive_processes_reject_future_ids_and_rotate_after_partial_ack() {
    let (base, _rx, server) = mock(vec![response(
        200,
        json!({"events":[event(11,"x"),event(12,"x")],"last":13}),
    )])
    .await;
    let cfg = config();
    let cursor = temp("cursor");
    let p = cursor.to_str().unwrap();
    let (ok, first) = cli(
        &cfg,
        Some(&base),
        &["events", "poll", "--after", "10", "--cursor-file", p],
    )
    .await;
    assert!(ok);
    let receipt = first["data"]["batch_id"].as_str().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(format!("{p}.pending.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let (ok, _) = cli(
        &cfg,
        Some(&base),
        &[
            "events",
            "ack",
            "14",
            "--cursor-file",
            p,
            "--batch-id",
            receipt,
        ],
    )
    .await;
    assert!(!ok);
    assert!(!cursor.exists());
    let (ok, _) = cli(
        &cfg,
        Some(&base),
        &[
            "events",
            "ack",
            "11",
            "--cursor-file",
            p,
            "--batch-id",
            receipt,
        ],
    )
    .await;
    assert!(ok);
    let (ok, replayed) = cli(&cfg, Some(&base), &["events", "poll", "--cursor-file", p]).await;
    assert!(ok);
    assert_eq!(replayed["data"]["events"].as_array().unwrap().len(), 1);
    assert_eq!(replayed["data"]["events"][0]["id"], 12);
    let (ok, _) = cli(
        &cfg,
        Some(&base),
        &[
            "events",
            "ack",
            "13",
            "--cursor-file",
            p,
            "--batch-id",
            receipt,
        ],
    )
    .await;
    assert!(!ok);
    let (ok, _) = cli(
        &cfg,
        Some(&base),
        &[
            "events",
            "ack",
            "13",
            "--cursor-file",
            p,
            "--batch-id",
            replayed["data"]["batch_id"].as_str().unwrap(),
        ],
    )
    .await;
    assert!(ok);
    assert_eq!(std::fs::read_to_string(p).unwrap(), "13");
    server.await.unwrap();
    cleanup(&cursor);
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn concurrent_polls_replay_one_batch_and_mcp_cannot_force_ack() {
    let (base, _rx, server) = mock(vec![response(
        200,
        json!({"events":[event(5,"x")],"last":5}),
    )])
    .await;
    let cursor = temp("cursor");
    let mut mcp = Mcp::new(Some(&base), Some(&cursor));
    mcp.call(1, "poll_events", json!({"after":4})).await;
    mcp.call(2, "poll_events", json!({"after":4})).await;
    let a = mcp.recv().await;
    let b = mcp.recv().await;
    assert_eq!(data(&a)["batch_id"], data(&b)["batch_id"]);
    mcp.call(3, "ack_events", json!({"id":500,"force":true}))
        .await;
    assert_eq!(mcp.recv().await["result"]["isError"], true);
    mcp.call(
        4,
        "ack_events",
        json!({"id":5,"batch_id":data(&a)["batch_id"]}),
    )
    .await;
    assert_eq!(mcp.recv().await["result"]["isError"], false);
    mcp.close().await;
    assert_eq!(std::fs::read_to_string(&cursor).unwrap(), "5");
    server.await.unwrap();
    cleanup(&cursor);
}
#[tokio::test]
async fn long_poll_does_not_block_ping_and_can_be_cancelled() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let _ = socket.read(&mut bytes).await;
        ready_tx.send(()).unwrap();
        signal.notified().await;
        let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 22\r\nConnection: close\r\n\r\n{\"events\":[],\"last\":1}").await;
    });
    let mut mcp = Mcp::new(Some(&base), None);
    mcp.call(1, "poll_events", json!({"after":1,"wait":25}))
        .await;
    ready_rx.await.unwrap();
    mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"ping"}))
        .await;
    assert_eq!(mcp.recv().await["id"], 2);
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}))
        .await;
    mcp.send(json!({"jsonrpc":"2.0","id":3,"method":"ping"}))
        .await;
    assert_eq!(mcp.recv().await["id"], 3);
    release.notify_one();
    mcp.close().await;
    task.await.unwrap();
}
#[tokio::test]
async fn partial_frames_are_preserved_while_other_requests_finish() {
    let mut mcp = Mcp::new(None, None);
    mcp.call(1, "doctor", json!({"offline":true})).await;
    mcp.input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,")
        .await
        .unwrap();
    assert_eq!(mcp.recv().await["id"], 1);
    mcp.input
        .write_all(b"\"method\":\"ping\"}\n")
        .await
        .unwrap();
    assert_eq!(mcp.recv().await["id"], 2);
    mcp.close().await;
}
#[tokio::test]
async fn oversized_frames_are_discarded_without_poisoning_next_request() {
    let mut mcp = Mcp::new(None, None);
    mcp.input.write_all(&vec![b'x'; 270000]).await.unwrap();
    mcp.input.write_all(b"\n").await.unwrap();
    mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        .await;
    assert_eq!(mcp.recv().await["error"]["code"], -32700);
    assert_eq!(mcp.recv().await["id"], 1);
    mcp.close().await;
}
#[tokio::test]
async fn bounded_history_keeps_continuation_at_the_returned_boundary() {
    let messages:Vec<_>=(0..6).map(|i|json!({"id":Uuid::from_u128(i+1),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"created_at":format!("2026-10-29T00:00:0{i}Z"),"content":"x".repeat(300)})).collect();
    let (base, _rx, server) = mock(vec![response(
        200,
        json!({"messages":messages,"authors":[]}),
    )])
    .await;
    let cfg = config();
    let (ok, reply) = cli(
        &cfg,
        Some(&base),
        &[
            "messages",
            "list",
            &Uuid::nil().to_string(),
            "--max-items",
            "2",
            "--fields",
            "content",
            "--max-bytes",
            "1400",
        ],
    )
    .await;
    assert!(ok);
    assert!(serde_json::to_vec(&reply).unwrap().len() <= 1400);
    assert_eq!(reply["data"]["returned_count"], 2);
    assert_eq!(
        reply["data"]["messages"][0]["id"],
        Uuid::from_u128(5).to_string()
    );
    assert_eq!(reply["data"]["next_cursor"], "2026-10-29T00:00:04Z");
    assert_eq!(reply["data"]["truncated"], true);
    server.await.unwrap();
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn split_write_failure_reports_confirmed_ids_and_unknown_failed_outcome() {
    let id = Uuid::new_v4();
    let (base, _rx, server) = mock(vec![
        response(200, json!({"message":{"id":id}})),
        response(502, json!({"error":"bad gateway"})),
    ])
    .await;
    let cfg = config();
    let text = "x".repeat(2500);
    let (ok, result) = cli(
        &cfg,
        Some(&base),
        &[
            "messages",
            "send",
            &Uuid::nil().to_string(),
            "--content",
            &text,
            "--split",
        ],
    )
    .await;
    assert!(!ok);
    assert_eq!(result["error"]["kind"], "partial_write");
    assert_eq!(
        result["error"]["completed_steps"][0]["message"]["id"],
        id.to_string()
    );
    assert_eq!(result["error"]["failed_step"], 2);
    assert_eq!(result["error"]["outcome"], "unknown");
    assert_eq!(result["error"]["retry_safety"], "do_not_replay_workflow");
    server.await.unwrap();
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn endpoint_schema_includes_only_reachable_components() {
    let cfg = config();
    let (ok, result) = cli(
        &cfg,
        None,
        &[
            "schema",
            "--method",
            "POST",
            "--path",
            "/channels/{id}/messages",
        ],
    )
    .await;
    assert!(ok);
    assert_eq!(result["data"]["method"], "POST");
    assert!(result["data"]["operation"]["requestBody"].is_object());
    assert!(serde_json::to_vec(&result).unwrap().len() < 177000);
    std::fs::remove_file(cfg).unwrap();
}

#[tokio::test]
async fn receipt_binding_survives_ack_and_crash_reconciliation_prunes_committed_events() {
    let (base, _rx, server) = mock(vec![response(
        200,
        json!({"events":[event(11,"x"),event(12,"x")],"last":12}),
    )])
    .await;
    let cfg = config();
    let cursor = temp("cursor");
    let path = cursor.to_str().unwrap();
    assert!(
        cli(
            &cfg,
            Some(&base),
            &["events", "poll", "--after", "10", "--cursor-file", path]
        )
        .await
        .0
    );
    // Simulate a crash after the numeric checkpoint advanced but before receipt update.
    std::fs::write(&cursor, "11").unwrap();
    let (ok, replay) = cli(
        &cfg,
        Some(&base),
        &["events", "poll", "--cursor-file", path],
    )
    .await;
    assert!(ok);
    assert_eq!(replay["data"]["events"].as_array().unwrap().len(), 1);
    assert_eq!(replay["data"]["events"][0]["id"], 12);
    assert!(
        cli(
            &cfg,
            Some(&base),
            &["events", "ack", "12", "--cursor-file", path]
        )
        .await
        .0
    );
    let out = Command::from(command(&cfg, Some(&base)))
        .env("YAPPER_BOT_TOKEN", "other-bot")
        .args(["events", "poll", "--cursor-file", path])
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("different bot"));
    server.await.unwrap();
    cleanup(&cursor);
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn timestamp_groups_are_not_silently_split_by_output_limits() {
    let messages:Vec<_>=(1..=3).map(|i|json!({"id":Uuid::from_u128(i),"author_id":Uuid::nil(),"channel_id":Uuid::nil(),"created_at":"2026-10-29T00:00:00Z","content":"same timestamp"})).collect();
    let (base, _rx, server) = mock(vec![response(
        200,
        json!({"messages":messages,"authors":[]}),
    )])
    .await;
    let cfg = config();
    let (ok, value) = cli(
        &cfg,
        Some(&base),
        &[
            "messages",
            "list",
            &Uuid::nil().to_string(),
            "--max-items",
            "2",
        ],
    )
    .await;
    assert!(!ok);
    assert_eq!(value["error"]["field"], "max_items");
    server.await.unwrap();
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn list_offset_and_member_continuation_match_returned_items() {
    let (base,_rx,server)=mock(vec![response(200,json!({"servers":[{"id":"a"},{"id":"b"},{"id":"c"}]})),response(200,json!({"members":[{"id":Uuid::from_u128(1),"username":"a"},{"id":Uuid::from_u128(2),"username":"b"}]}))]).await;
    let cfg = config();
    let (ok, a) = cli(
        &cfg,
        Some(&base),
        &["servers", "list", "--offset", "1", "--max-items", "1"],
    )
    .await;
    assert!(ok);
    assert_eq!(a["data"]["servers"][0]["id"], "b");
    assert_eq!(a["data"]["next_offset"], 2);
    let (ok, b) = cli(
        &cfg,
        Some(&base),
        &[
            "members",
            "list",
            &Uuid::nil().to_string(),
            "--max-items",
            "1",
        ],
    )
    .await;
    assert!(ok);
    assert_eq!(b["data"]["next_cursor"], Uuid::from_u128(1).to_string());
    assert_eq!(b["data"]["truncated"], true);
    server.await.unwrap();
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn cancellation_does_not_abandon_submitted_writes() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 8192];
        let _ = socket.read(&mut bytes).await;
        ready_tx.send(()).unwrap();
        signal.notified().await;
        let body = "{\"message\":{\"id\":\"sent\"}}";
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let mut mcp = Mcp::new(Some(&base), None);
    mcp.call(
        1,
        "send_message",
        json!({"channel":Uuid::nil(),"content":"requested"}),
    )
    .await;
    ready_rx.await.unwrap();
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}))
        .await;
    mcp.send(json!({"jsonrpc":"2.0","id":2,"method":"ping"}))
        .await;
    assert_eq!(mcp.recv().await["id"], 2);
    release.notify_one();
    let sent = mcp.recv().await;
    assert_eq!(sent["id"], 1);
    assert_eq!(data(&sent)["messages"][0]["message"]["id"], "sent");
    mcp.close().await;
    task.await.unwrap();
}

#[tokio::test]
async fn read_failures_are_safe_reads_and_tiny_budgets_do_not_hide_results() {
    let (base, _rx, server) = mock(vec![
        response(502, json!({"error":"gateway"})),
        response(
            200,
            json!({"servers":[{"id":"a","description":"x".repeat(1000)}]}),
        ),
    ])
    .await;
    let cfg = config();
    let (ok, error) = cli(&cfg, Some(&base), &["servers", "list"]).await;
    assert!(!ok);
    assert_eq!(error["error"]["retry_safety"], "safe_read");
    let (ok, error) = cli(
        &cfg,
        Some(&base),
        &["servers", "list", "--max-bytes", "256"],
    )
    .await;
    assert!(!ok);
    assert_eq!(error["error"]["field"], "max_bytes");
    server.await.unwrap();
    std::fs::remove_file(cfg).unwrap();
}
#[tokio::test]
async fn repeated_acknowledgement_is_idempotent_without_invalidating_current_receipt() {
    let (base, _rx, server) = mock(vec![response(
        200,
        json!({"events":[event(2,"x"),event(3,"x")],"last":3}),
    )])
    .await;
    let cfg = config();
    let cursor = temp("cursor");
    let path = cursor.to_str().unwrap();
    let (_, poll) = cli(
        &cfg,
        Some(&base),
        &["events", "poll", "--after", "1", "--cursor-file", path],
    )
    .await;
    let receipt = poll["data"]["batch_id"].as_str().unwrap();
    assert!(
        cli(
            &cfg,
            Some(&base),
            &[
                "events",
                "ack",
                "2",
                "--batch-id",
                receipt,
                "--cursor-file",
                path
            ]
        )
        .await
        .0
    );
    let (ok, repeated) = cli(
        &cfg,
        Some(&base),
        &[
            "events",
            "ack",
            "2",
            "--batch-id",
            receipt,
            "--cursor-file",
            path,
        ],
    )
    .await;
    assert!(ok);
    assert_eq!(repeated["data"]["already_committed"], true);
    let (ok, pending) = cli(
        &cfg,
        Some(&base),
        &["events", "poll", "--cursor-file", path],
    )
    .await;
    assert!(ok);
    assert_eq!(pending["data"]["events"][0]["id"], 3);
    server.await.unwrap();
    cleanup(&cursor);
    std::fs::remove_file(cfg).unwrap();
}

#[tokio::test]
async fn mcp_can_preview_long_split_messages_with_the_same_shared_validation() {
    let mut mcp = Mcp::new(None, None);
    mcp.call(
        1,
        "send_message",
        json!({"channel":Uuid::nil(),"content":"x".repeat(2500),"split":true,"dry_run":true}),
    )
    .await;
    let value = mcp.recv().await;
    assert_eq!(value["result"]["isError"], false);
    assert_eq!(data(&value)["messages"].as_array().unwrap().len(), 2);
    mcp.call(
        2,
        "send_message",
        json!({"channel":Uuid::nil(),"content":"x".repeat(2500),"dry_run":true}),
    )
    .await;
    assert_eq!(mcp.recv().await["result"]["isError"], true);
    mcp.close().await;
}
#[tokio::test]
async fn mcp_admission_limit_preserves_control_responses() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (ready, mut seen) = tokio::sync::mpsc::channel(16);
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let server = tokio::spawn(async move {
        let mut workers = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let ready = ready.clone();
            let signal = signal.clone();
            workers.spawn(async move {
                let mut buf = [0; 4096];
                let _ = socket.read(&mut buf).await;
                ready.send(()).await.unwrap();
                signal.notified().await;
            });
        }
        while workers.join_next().await.is_some() {}
    });
    let mut mcp = Mcp::new(Some(&base), None);
    for id in 1..=16 {
        mcp.call(id, "list_servers", json!({})).await;
    }
    for _ in 0..16 {
        tokio::time::timeout(Duration::from_secs(4), seen.recv())
            .await
            .unwrap()
            .unwrap();
    }
    mcp.call(17, "list_servers", json!({})).await;
    let rejected = mcp.recv().await;
    assert_eq!(rejected["id"], 17);
    assert_eq!(rejected["error"]["code"], -32000);
    mcp.send(json!({"jsonrpc":"2.0","id":18,"method":"ping"}))
        .await;
    assert_eq!(mcp.recv().await["id"], 18);
    mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        .await;
    assert_eq!(mcp.recv().await["error"]["code"], -32600);
    for id in 1..=16 {
        mcp.send(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}),
        )
        .await;
    }
    mcp.close().await;
    release.notify_waiters();
    server.await.unwrap();
}

#[tokio::test]
async fn cursor_locks_prevent_cross_process_mutation_and_release_on_close() {
    use fs2::FileExt;
    let cfg = config();
    let cursor = temp("cursor");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(format!("{}.lock", cursor.display()))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let (ok, value) = cli(
        &cfg,
        Some("http://127.0.0.1:1"),
        &[
            "events",
            "ack",
            "5",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--force",
        ],
    )
    .await;
    assert!(!ok);
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("another consumer"));
    assert!(!cursor.exists());
    drop(lock);
    let (ok, _) = cli(
        &cfg,
        None,
        &[
            "events",
            "ack",
            "5",
            "--cursor-file",
            cursor.to_str().unwrap(),
            "--force",
        ],
    )
    .await;
    assert!(ok);
    assert_eq!(std::fs::read_to_string(&cursor).unwrap(), "5");
    cleanup(&cursor);
    std::fs::remove_file(cfg).unwrap();
}
