mod support;

use rappin::{Channel, ChannelKind, ChannelRow, Client};
use serde_json::{json, Value};
use uuid::Uuid;

fn channel() -> Value {
    json!({
        "id": Uuid::from_u128(1),
        "name": "general",
        "kind": "text",
        "type": "text",
        "server_id": Uuid::from_u128(2),
        "position": 3,
        "category": {"id": Uuid::from_u128(4), "name": "Main", "position": 0},
        "category_id": Uuid::from_u128(4),
        "private": false,
        "read_only": true,
        "slowmode": 5,
        "future_field": {"type": "voice"}
    })
}

#[test]
fn channel_models_accept_either_or_both_documented_type_fields() {
    for (name, expected) in [("text", ChannelKind::Text), ("voice", ChannelKind::Voice)] {
        for omitted in [None, Some("kind"), Some("type")] {
            let mut value = channel();
            value["kind"] = name.into();
            value["type"] = name.into();
            if let Some(key) = omitted {
                value.as_object_mut().unwrap().remove(key);
            }
            let channel: Channel = serde_json::from_value(value.clone()).unwrap();
            let row: ChannelRow = serde_json::from_value(value).unwrap();
            assert_eq!(channel.kind, expected);
            assert_eq!(row.kind, expected);
            assert_eq!(channel.name, "general");
            assert_eq!(channel.server_id, Some(Uuid::from_u128(2)));
            assert_eq!(channel.position, Some(3));
            assert_eq!(channel.category.as_ref().unwrap().name, "Main");
            assert_eq!(channel.category_id, Some(Uuid::from_u128(4)));
            assert_eq!(channel.read_only, Some(true));
            assert_eq!(channel.slowmode, Some(5));
            assert_eq!(row.name.as_deref(), Some("general"));
            assert_eq!(row.position, Some(3));
            assert_eq!(row.category_id, Some(Uuid::from_u128(4)));
            assert_eq!(row.private, Some(false));
            assert_eq!(row.read_only, Some(true));
            assert_eq!(row.slowmode, Some(5));
            let serialized = serde_json::to_value(&channel).unwrap();
            assert_eq!(serialized["kind"], name);
            assert!(serialized.get("type").is_none());
            assert_eq!(
                serde_json::from_value::<Channel>(serialized).unwrap(),
                channel
            );
            let serialized = serde_json::to_value(&row).unwrap();
            assert_eq!(serialized["kind"], name);
            assert!(serialized.get("type").is_none());
            assert_eq!(
                serde_json::from_value::<ChannelRow>(serialized).unwrap(),
                row
            );
        }
    }
}

#[test]
fn matching_type_fields_work_in_either_json_key_order() {
    for fields in [
        r#""kind":"text","type":"text""#,
        r#""type":"text","kind":"text""#,
    ] {
        let raw = format!(r#"{{"id":"{}","name":"general",{fields}}}"#, Uuid::nil());
        assert_eq!(
            serde_json::from_str::<Channel>(&raw).unwrap().kind,
            ChannelKind::Text
        );
        assert_eq!(
            serde_json::from_str::<ChannelRow>(&raw).unwrap().kind,
            ChannelKind::Text
        );
    }
}

#[test]
fn channel_models_reject_missing_invalid_or_conflicting_type_fields() {
    for fields in [
        json!({}),
        json!({"kind": "invalid"}),
        json!({"type": "invalid"}),
        json!({"kind": null}),
        json!({"type": null}),
        json!({"kind": null, "type": "text"}),
        json!({"kind": "text", "type": null}),
        json!({"kind": "text", "type": "voice"}),
        json!({"kind": "voice", "type": "text"}),
        json!({"kind": "text", "type": "invalid"}),
    ] {
        let mut value = json!({"id": Uuid::nil(), "name": "general"});
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert!(
            serde_json::from_value::<Channel>(value.clone()).is_err(),
            "{value}"
        );
        assert!(
            serde_json::from_value::<ChannelRow>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[tokio::test]
async fn typed_channel_endpoints_and_resolution_accept_live_response_shape() {
    let channel_id = Uuid::from_u128(1);
    let server_id = Uuid::from_u128(2);
    let listing = json!({"channels": [channel()], "categories": []});
    let (base, _requests, task) = support::mock(vec![
        support::response(200, listing.clone()),
        support::response(200, listing.clone()),
        support::response(200, json!({"channel": channel()})),
        support::response(200, listing.clone()),
        support::response(200, listing),
    ])
    .await;
    let client = Client::builder("yb_fixture").base_url(base).build();
    assert_eq!(
        client.get_channels(None).await.unwrap()[0].kind,
        ChannelKind::Text
    );
    assert_eq!(
        client
            .get_server_channels(server_id, None)
            .await
            .unwrap()
            .channels[0]
            .id,
        channel_id
    );
    assert_eq!(
        client.get_channel(channel_id).await.unwrap().kind,
        ChannelKind::Text
    );
    assert_eq!(
        client
            .resolve_channel(Some(server_id), "GENERAL")
            .await
            .unwrap()
            .id,
        channel_id
    );
    assert_eq!(
        client.resolve_channel(None, "general").await.unwrap().id,
        channel_id
    );
    assert_eq!(
        client
            .cache()
            .get_channel(&channel_id)
            .await
            .unwrap()
            .slowmode,
        Some(5)
    );
    task.await.unwrap();
}

#[cfg(feature = "cli")]
#[tokio::test]
async fn cli_and_mcp_resolve_channels_with_both_type_fields() {
    use std::process::Stdio;
    use tokio::{io::AsyncWriteExt, process::Command};

    let server_id = Uuid::from_u128(2);
    let listing = json!({"channels": [channel()], "categories": []});
    let (base, mut requests, task) = support::mock(vec![
        support::response(200, listing.clone()),
        support::response(200, listing),
    ])
    .await;
    let config = std::env::temp_dir().join(format!("rappin-channel-{}.toml", Uuid::new_v4()));
    tokio::fs::write(&config, "").await.unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rappin"));
        command
            .env("YAPPER_BOT_TOKEN", "yb_fixture")
            .env_remove("YAPPER_API_BASE")
            .arg("--config")
            .arg(&config)
            .args(["--base-url", &base]);
        command
    };
    let output = command()
        .args([
            "channels",
            "resolve",
            "general",
            "--server",
            &server_id.to_string(),
        ])
        .output()
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{result}");
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["id"], Uuid::from_u128(1).to_string());
    assert_eq!(result["data"]["kind"], "text");

    let mut child = command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"resolve_channel","arguments":{"name":"general","server":server_id}}}),
    ] {
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    drop(input);
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies[1]["result"]["isError"], false);
    let result: Value =
        serde_json::from_str(replies[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["data"]["id"], Uuid::from_u128(1).to_string());
    assert_eq!(result["data"]["kind"], "text");
    for _ in 0..2 {
        assert!(requests
            .recv()
            .await
            .unwrap()
            .starts_with(&format!("GET /servers/{server_id}/channels ")));
    }
    task.await.unwrap();
    tokio::fs::remove_file(config).await.unwrap();
}
