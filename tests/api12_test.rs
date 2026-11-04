mod support;
use rappin::{Client, CreateEmoji, CreateMessage, UploadFile, YappinError};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use uuid::Uuid;
fn message(id: u128) -> Value {
    json!({"id":Uuid::from_u128(id),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"content":"file","created_at":"2026-11-02T00:00:00Z","attachment_path":"test/a.txt","attachment_name":"a.txt","attachment_size":3})
}
fn version() -> Value {
    json!({"api":"1.2.0","build":"fixture","sdk":{"js":"1.5.0","python":"1.5.0"},"packages":{"npm":"yapin","pypi":"yapin"}})
}

#[tokio::test]
async fn public_discovery_omits_credentials_and_observes_version_headers() {
    let (base, mut calls, http) = support::mock(vec![
        (
            200,
            vec![
                ("X-Yapin-API-Version", "1.2.0"),
                ("X-Yapin-Build", "fixture"),
            ],
            version().to_string(),
        ),
        support::response(200, json!({"api":"1.2.0","entries":[]})),
    ])
    .await;
    let client = Client::builder("yb_api12_private").base_url(base).build();
    assert_eq!(client.get_api_version().await.unwrap().api, "1.2.0");
    let last = client.last_request().unwrap();
    assert_eq!(last.api_version.as_deref(), Some("1.2.0"));
    assert_eq!(last.api_build.as_deref(), Some("fixture"));
    assert!(client
        .get_api_changelog(1)
        .await
        .unwrap()
        .entries
        .is_empty());
    for _ in 0..2 {
        let request = calls.recv().await.unwrap().to_lowercase();
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("yb_api12_private"));
    }
    assert!(client.get_api_changelog(0).await.is_err());
    http.await.unwrap();
}

#[tokio::test]
async fn multipart_success_and_partial_success_keep_every_receipt_without_replay() {
    let (base, mut calls, http) = support::mock(vec![
        support::response(
            201,
            json!({"message":message(1),"messages":[message(1),message(2)]}),
        ),
        support::response(
            201,
            json!({"message":message(3),"messages":[message(3)],"error":"second file failed"}),
        ),
    ])
    .await;
    let client = Client::builder("yb_api12_private").base_url(base).build();
    let files = [
        UploadFile::new("a.txt", b"one".to_vec()),
        UploadFile::new("SPOILER_b.png", b"two".to_vec()),
    ];
    let sent = client
        .send_files(Uuid::nil(), &CreateMessage::new("hello"), &files)
        .await
        .unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].attachment_name.as_deref(), Some("a.txt"));
    assert_eq!(sent[0].attachment_size, Some(3));
    let error = client
        .send_files(Uuid::nil(), &CreateMessage::default(), &files)
        .await
        .unwrap_err();
    assert!(
        matches!(&error,YappinError::PartialWrite{completed,failed_step:2,..} if completed.len()==1)
    );
    assert_eq!(
        error.to_remote().details["retry_safety"],
        "do_not_replay_workflow"
    );
    assert_eq!(error.to_remote().details["outcome"], "unknown");
    for _ in 0..2 {
        let request = calls.recv().await.unwrap();
        assert!(request.contains("multipart/form-data"));
        assert!(request.contains("name=\"payload_json\""));
        assert!(request.contains("name=\"files[0]\""));
        assert!(request.contains("filename=\"SPOILER_b.png\""));
    }
    assert_eq!(client.request_metrics().attempts, 2);
    http.await.unwrap();
}

#[test]
fn uploads_and_emoji_validation_are_bounded_and_do_not_take_paths() {
    for name in ["../token.txt", "/home/token.txt", "bad\r\n.txt", "bad.exe"] {
        assert!(UploadFile::new(name, b"a".to_vec()).validate().is_err());
    }
    assert!(rappin::validate_uploads(&[]).is_err());
    assert!(rappin::validate_uploads(&vec![UploadFile::new("a.txt", vec![1]); 5]).is_err());
    assert!(rappin::validate_uploads(&[
        UploadFile::new("a.txt", vec![1; 7 * 1024 * 1024]),
        UploadFile::new("b.txt", vec![2; 6 * 1024 * 1024])
    ])
    .is_err());
    for (name, image) in [
        ("x", "YQ=="),
        ("bad-name", "YQ=="),
        ("ok", "not base64"),
        ("ok", "data:image/jpeg;base64,YQ=="),
    ] {
        assert!(CreateEmoji::new(name, image).validate().is_err());
    }
    assert!(CreateEmoji::new("valid_name", "YQ==").validate().is_ok());
}

#[tokio::test]
async fn emoji_pending_and_bot_webhook_authentication_remain_distinct() {
    let webhook = Uuid::from_u128(8);
    let (base, mut calls, http) = support::mock(vec![
        support::response(201, json!({"emoji":null,"pending":true})),
        support::response(200, json!({"message":message(9)})),
    ])
    .await;
    let client = Client::builder("yb_api12_private").base_url(base).build();
    let created = client
        .create_emoji(Uuid::nil(), &CreateEmoji::new("party", "YQ=="))
        .await
        .unwrap();
    assert!(created.pending && created.emoji.is_none());
    let body = rappin::webhook::WebhookMessage {
        content: Some("hello".into()),
        allowed_mentions: Some(json!({"parse":[]})),
        ..Default::default()
    };
    client.post_bot_webhook(webhook, &body).await.unwrap();
    let create = calls.recv().await.unwrap();
    assert!(create.starts_with("POST /servers/"));
    let post = calls.recv().await.unwrap();
    assert!(post.starts_with(&format!("POST /webhooks/{webhook} ")));
    assert!(post
        .to_lowercase()
        .contains("authorization: bot yb_api12_private"));
    assert!(post.contains("allowed_mentions"));
    http.await.unwrap();
}

#[tokio::test]
async fn retry_wait_budget_covers_shared_deadlines_and_retains_advice() {
    let (base, mut calls, http) = support::mock(vec![(
        429,
        vec![("Retry-After", "0.2")],
        json!({"error":"slow down","global":true}).to_string(),
    )])
    .await;
    let client = Client::builder("fixture")
        .base_url(base)
        .max_retry_wait(Duration::from_millis(20))
        .build();
    let error = client
        .send_message(Uuid::nil(), &CreateMessage::new("first"))
        .await
        .unwrap_err();
    assert!(matches!(error, YappinError::RetryWaitExceeded { .. }));
    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.retry_after(), Some(Duration::from_millis(200)));
    let now = tokio::time::Instant::now();
    let error = client
        .send_message(Uuid::from_u128(1), &CreateMessage::new("second"))
        .await
        .unwrap_err();
    assert!(matches!(error, YappinError::RetryWaitExceeded { .. }));
    assert!(now.elapsed() < Duration::from_millis(150));
    assert_eq!(client.request_metrics().attempts, 1);
    assert!(calls.recv().await.unwrap().contains("first"));
    assert!(calls.try_recv().is_err());
    http.await.unwrap();
}

#[tokio::test]
async fn bounded_retry_reuses_multipart_fields_and_never_retries_ambiguous_writes() {
    let (base, mut calls, http) = support::mock(vec![
        (
            503,
            vec![("Retry-After", "0.001")],
            json!({"error":"busy"}).to_string(),
        ),
        support::response(201, json!({"message":message(1)})),
        support::response(502, json!({"error":"unknown"})),
    ])
    .await;
    let client = Client::builder("fixture").base_url(base).build();
    let files = [UploadFile::new("a.txt", b"payload".to_vec())];
    client
        .send_files(Uuid::nil(), &CreateMessage::default(), &files)
        .await
        .unwrap();
    assert_eq!(
        client
            .send_files(Uuid::nil(), &CreateMessage::default(), &files)
            .await
            .unwrap_err()
            .status_code(),
        Some(502)
    );
    for _ in 0..3 {
        assert!(calls.recv().await.unwrap().contains("payload"));
    }
    assert_eq!(client.request_metrics().attempts, 3);
    http.await.unwrap();
}

struct DmHandler(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl rappin::EventHandler for DmHandler {
    async fn on_dm(&self, ctx: &rappin::MessageContext) -> rappin::Result<()> {
        assert!(ctx.event.dm && ctx.event.is_dm);
        assert!(ctx.command.is_none());
        assert_eq!(ctx.event.attachments[0].name.as_deref(), Some("a.txt"));
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn on_message(&self, _: &rappin::MessageContext) {
        panic!("DM must not enter server messages");
    }
    async fn on_reaction_add(&self, _: &Client, event: &rappin::ReactionEvent) {
        assert!(event.is_dm);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn typed_dm_collectors_and_callbacks_never_dispatch_server_commands() {
    let count = Arc::new(AtomicUsize::new(0));
    let commands = Arc::new(AtomicUsize::new(0));
    let called = commands.clone();
    let bot = rappin::Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .event_handler(DmHandler(count.clone()))
        .command("danger", "test", move |_| {
            let called = called.clone();
            async move {
                called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let waiter = bot
        .client()
        .events()
        .collect::<rappin::DmMessageEvent>()
        .author(Uuid::nil())
        .channel(Uuid::nil())
        .timeout(Duration::from_secs(1))
        .limit(1)
        .subscribe()
        .unwrap();
    let dm: rappin::Event=serde_json::from_value(json!({"id":1,"type":"dm_message","data":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"server_id":null,"author_id":Uuid::nil(),"content":"!danger","command":"danger","attachments":[{"path":"a","name":"a.txt","size":1}]}})).unwrap();
    assert!(matches!(
        dm.parse_data().unwrap(),
        rappin::EventData::DmMessage(_)
    ));
    bot.process_event(dm).await.unwrap();
    assert_eq!(
        waiter.wait().await.unwrap().content.as_deref(),
        Some("!danger")
    );
    let reaction=serde_json::from_value(json!({"id":2,"type":"reaction_add","data":{"channel_id":Uuid::nil(),"message_id":Uuid::nil(),"user_id":Uuid::nil(),"emoji":"x","server_id":null,"is_dm":true,"dm_channel_id":Uuid::nil()}})).unwrap();
    bot.process_event(reaction).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(commands.load(Ordering::SeqCst), 0);
}

#[test]
fn catalogue_uses_upstream_schemas_for_all_new_operations_and_redacts_previews() {
    use rappin::capabilities::*;
    assert_eq!(bundled_schema()["info"]["version"], "1.3.0");
    assert_eq!(catalogue().len(), 57);
    for name in [
        "list_emojis",
        "create_emoji",
        "edit_emoji",
        "delete_emoji",
        "list_bot_webhooks",
        "list_channel_webhooks",
        "create_bot_webhook",
        "delete_bot_webhook",
        "post_bot_webhook",
        "send_files",
        "get_api_version",
        "get_api_changelog",
    ] {
        assert!(capability(name).is_some(), "{name}");
    }
    let call = capability("send_files")
        .unwrap()
        .validate(
            &json!({"channel":Uuid::nil(),"files":[{"name":"a.txt","data":"YQ=="}],"dry_run":true}),
        )
        .unwrap();
    assert_eq!(call.api_request().unwrap().files[0].data.as_ref(), b"a");
    assert!(!call.preview().data.to_string().contains("YQ=="));
    assert!(capability("create_emoji")
        .unwrap()
        .validate(&json!({"id":Uuid::nil(),"body":{"name":"bad-name","image":"YQ=="}}))
        .is_err());
}

#[tokio::test]
async fn proxy_metadata_survives_serialization_but_is_not_resent_in_typed_payloads() {
    let embed:rappin::Embed=serde_json::from_value(json!({"image":{"url":"https://example.com/p.png","proxy_url":"https://files.yapper.website/proxy/a"},"author":{"name":"a","icon_url":"https://example.com/a.png","proxy_icon_url":"https://files.yapper.website/proxy/b"}})).unwrap();
    assert_eq!(
        embed.image.as_ref().unwrap().proxy_url.as_deref(),
        Some("https://files.yapper.website/proxy/a")
    );
    assert!(serde_json::to_value(&embed).unwrap()["image"]["proxy_url"].is_string());
    let (base, mut requests, http) = support::mock(vec![
        support::response(201, json!({"message":message(1)})),
        support::response(200, json!({"message":message(2)})),
    ])
    .await;
    let client = Client::builder("fixture").base_url(base).build();
    client
        .send_message(
            Uuid::nil(),
            &CreateMessage::new("embed").embed(embed.clone()),
        )
        .await
        .unwrap();
    let body = rappin::webhook::WebhookMessage {
        content: Some("proxy".into()),
        embeds: vec![embed],
        ..Default::default()
    };
    client.post_bot_webhook(Uuid::nil(), &body).await.unwrap();
    for _ in 0..2 {
        let request = requests.recv().await.unwrap();
        assert!(!request.contains("proxy_url"));
        assert!(!request.contains("proxy_icon_url"));
        assert!(request.contains("https://example.com"));
    }
    http.await.unwrap();
}

#[test]
fn missing_per_file_confirmation_is_an_unknown_partial_write() {
    let response: rappin::UploadedMessages =
        serde_json::from_value(json!({"message":message(1)})).unwrap();
    let error = response.into_result_for(2).unwrap_err();
    assert!(matches!(error, YappinError::PartialWrite { .. }));
    assert_eq!(error.to_json()["outcome"], "unknown");
}

struct FailedDm;
#[async_trait::async_trait]
impl rappin::EventHandler for FailedDm {
    async fn on_dm(&self, _: &rappin::MessageContext) -> rappin::Result<()> {
        Err(YappinError::RateLimited { retry_after: 2.0 })
    }
}
#[tokio::test]
async fn failed_dm_processing_retains_original_cause_and_cursor_barrier() {
    let path = std::env::temp_dir().join(format!("rappin-dm-cursor-{}.json", Uuid::new_v4()));
    let cursor = rappin::EventCursor::new(&path);
    cursor.save(1).await.unwrap();
    let dm = json!({"id":2,"type":"dm_message","data":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"server_id":null,"content":"hello"}});
    let (base, _calls, http) = support::routed_mock(vec![
        (
            "GET /me".into(),
            vec![support::response(200, support::profile())],
        ),
        (
            "GET /events".into(),
            vec![support::response(
                200,
                json!({"events":[dm,support::event(3,"later")],"last":3}),
            )],
        ),
    ])
    .await;
    let bot = rappin::Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .handler_concurrency(1)
        .cursor_file(&path)
        .event_handler(FailedDm)
        .command("later", "must not run", |_| async {
            panic!("failed DM cannot be checkpointed implicitly")
        })
        .build();
    let error = bot.run().await.unwrap_err();
    assert_eq!(error.status_code(), Some(429));
    assert_eq!(error.retry_after(), Some(Duration::from_secs(2)));
    let report = error.to_json();
    assert_eq!(report["report"]["committed_cursor"], 1);
    assert_eq!(report["report"]["failed_events"][0]["event_id"], 2);
    assert_eq!(report["report"]["unprocessed_events"], json!([3]));
    assert_eq!(cursor.load().await.unwrap(), Some(1));
    http.await.unwrap();
    let _ = std::fs::remove_file(path);
}
