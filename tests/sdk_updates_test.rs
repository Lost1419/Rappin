mod support;
use rappin::*;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use support::*;
use uuid::Uuid;

#[tokio::test]
async fn upload_and_json_retries_preserve_payloads_and_custom_origins() {
    let (base, mut requests, task) = mock(vec![
        (
            503,
            vec![("Retry-After", "0")],
            "{\"error\":\"busy\"}".into(),
        ),
        response(200, json!({})),
        (
            503,
            vec![("Retry-After", "0")],
            "{\"error\":\"busy\"}".into(),
        ),
        response(200, json!({})),
        response(502, json!({"error":"bad gateway"})),
    ])
    .await;
    let client = Client::builder("secret").base_url(&base).build();
    client
        .request_binary::<serde_json::Value>(
            reqwest::Method::PUT,
            "/me/avatar",
            b"image-data".to_vec(),
            "image/png",
        )
        .await
        .unwrap();
    client
        .request_json::<serde_json::Value, _>(
            reqwest::Method::PUT,
            "/commands",
            Some(&json!({"commands":[]})),
        )
        .await
        .unwrap();
    for ending in [
        "image-data",
        "image-data",
        "{\"commands\":[]}",
        "{\"commands\":[]}",
    ] {
        assert!(requests.recv().await.unwrap().ends_with(ending));
    }
    assert_eq!(
        client
            .request_json::<serde_json::Value, ()>(reqwest::Method::GET, "/me", None)
            .await
            .unwrap_err()
            .status_code(),
        Some(502)
    );
    assert_eq!(client.active_base_url(), base);
    task.await.unwrap();
}

#[derive(Default)]
struct AppCache {
    members: tokio::sync::Mutex<HashMap<(Uuid, Uuid), MemberDetail>>,
    role_reads: AtomicUsize,
    invalidations: AtomicUsize,
}
#[async_trait::async_trait]
impl LookupCache for AppCache {
    async fn get_member(&self, s: Uuid, u: Uuid) -> Option<MemberDetail> {
        self.members.lock().await.get(&(s, u)).cloned()
    }
    async fn get_roles(&self, _: Uuid) -> Option<Vec<Role>> {
        self.role_reads.fetch_add(1, Ordering::SeqCst);
        Some(vec![])
    }
    async fn store_member(&self, s: Uuid, u: Uuid, m: &MemberDetail) {
        self.members.lock().await.insert((s, u), m.clone());
    }
    async fn store_roles(&self, _: Uuid, _: &[Role]) {}
    async fn invalidate_member(&self, s: Uuid, u: Uuid) {
        self.members.lock().await.remove(&(s, u));
    }
    async fn invalidate_server(&self, s: Uuid) {
        self.members
            .lock()
            .await
            .retain(|(server, _), _| *server != s);
        self.invalidations.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn application_cache_serves_lookups_but_refresh_and_mutations_reach_api() {
    let member = json!({"id":Uuid::nil(),"username":"member","roles":[],"nickname":null});
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"member":member})),
        response(200, json!({"member":member})),
        response(200, json!({"ok":true})),
        response(200, json!({"member":member})),
    ])
    .await;
    let backend = Arc::new(AppCache::default());
    let client = Client::builder("secret")
        .base_url(base)
        .lookup_cache_backend(backend.clone())
        .build();
    let id = Uuid::nil();
    client
        .fetch_member(id, id, Freshness::PreferCache)
        .await
        .unwrap(); // miss, populate
    client
        .fetch_member(id, id, Freshness::PreferCache)
        .await
        .unwrap(); // backend hit
    client
        .fetch_member(id, id, Freshness::Refresh)
        .await
        .unwrap();
    assert!(client
        .fetch_roles(id, Freshness::PreferCache)
        .await
        .unwrap()
        .is_empty());
    client.set_nickname(id, id, "test").await.unwrap();
    assert_eq!(backend.invalidations.load(Ordering::SeqCst), 1);
    client
        .fetch_member(id, id, Freshness::PreferCache)
        .await
        .unwrap(); // invalidated
    for method in ["GET", "GET", "PATCH", "GET"] {
        assert!(requests.recv().await.unwrap().starts_with(method));
    }
    assert_eq!(backend.role_reads.load(Ordering::SeqCst), 1);
    task.await.unwrap();
}

fn raw(content: &str) -> Event {
    serde_json::from_value(json!({"id":1,"type":"message","data":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"content":content}})).unwrap()
}
#[tokio::test]
async fn database_prefixes_stay_dynamic_and_cog_root_index_updates() {
    let (base, _requests, task) = mock(vec![response(200, profile())]).await;
    let prefix = Arc::new(tokio::sync::RwLock::new("first!".to_owned()));
    let calls = Arc::new(AtomicUsize::new(0));
    let hits = Arc::new(AtomicUsize::new(0));
    let (p, c, h) = (prefix.clone(), calls.clone(), hits.clone());
    let backend = Arc::new(AppCache::default());
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .lookup_cache_backend(backend.clone())
        .prefixes_with(move |_| {
            let p = p.clone();
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(vec![String::new(), p.read().await.clone()])
            }
        })
        .command_spec(CommandSpec::<()>::new("tool run", "Run"), move |_, ()| {
            let h = h.clone();
            async move {
                h.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();
    bot.process_event(raw("first!tool run")).await.unwrap();
    *prefix.write().await = "second!".into();
    bot.process_event(raw("first!tool run")).await.unwrap();
    bot.process_event(raw("second!tool run")).await.unwrap();
    let h = hits.clone();
    bot.add_cog(
        CogBuilder::new("extra")
            .command("other", "Other", move |_| {
                let h = h.clone();
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build(),
    )
    .await
    .unwrap();
    bot.process_event(raw("second!other")).await.unwrap();
    bot.handle().remove_cog("extra").await.unwrap();
    bot.process_event(raw("second!other")).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert!(bot
        .client()
        .fetch_roles(Uuid::nil(), Freshness::PreferCache)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(backend.role_reads.load(Ordering::SeqCst), 1);
    task.await.unwrap();
}

#[tokio::test]
async fn static_prefixes_prefer_longest_and_keep_mentions() {
    let (base, _requests, task) = mock(vec![response(200, profile())]).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .prefix("!")
        .prefix("!!")
        .prefix("")
        .mention_prefix(true)
        .command("ping", "Ping", move |_| {
            let h = h.clone();
            async move {
                h.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();
    for content in [
        "!!ping",
        "!ping",
        "<@11111111-1111-4111-8111-111111111111> ping",
    ] {
        bot.process_event(raw(content)).await.unwrap();
    }
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    task.await.unwrap();
}

#[tokio::test]
async fn server_expiry_blocks_callback_without_network_and_is_not_extended() {
    let client = Client::new("not-used");
    let deadline = chrono::Utc::now() - chrono::Duration::seconds(1);
    let interaction: InteractionEvent = serde_json::from_value(
        json!({"id":"id","token":"secret","type":"button","expires_at":deadline}),
    )
    .unwrap();
    assert_eq!(
        client.interaction_state(&interaction).await.unwrap().phase,
        ResponsePhase::Expired
    );
    assert!(client
        .respond_interaction("id", "secret", &InteractionCallback::ack())
        .await
        .is_err());
    let mut extended = interaction.clone();
    extended.expires_at = Some(chrono::Utc::now() + chrono::Duration::seconds(900));
    assert_eq!(
        client.interaction_state(&extended).await.unwrap().phase,
        ResponsePhase::Expired
    );
    let ctx = InteractionContext::new(Arc::new(client), interaction, None);
    assert!(ctx.ephemeral("too late").await.is_err());
}

#[test]
fn typed_events_preserve_documented_metadata() {
    let id = Uuid::nil();
    let mut event = raw("hello");
    event.data["channel_name"] = json!("general");
    event.data["author"] = json!({"username":"Alice"});
    event.data["attachment"] = json!("image.png");
    let EventData::Message(message) = event.parse_data().unwrap() else {
        panic!()
    };
    assert_eq!(message.channel_name.as_deref(), Some("general"));
    assert!(message.author.is_some());
    let reaction:Event=serde_json::from_value(json!({"type":"reaction_add","data":{"channel_id":id,"message_id":id,"user_id":id,"emoji":"🦀","username":"Alice","message_author_id":id}})).unwrap();
    let EventData::ReactionAdd(reaction) = reaction.parse_data().unwrap() else {
        panic!()
    };
    assert_eq!(reaction.username.as_deref(), Some("Alice"));
    assert_eq!(reaction.message_author_id, Some(id));
}

#[tokio::test]
async fn webhook_inspection_uses_get_preserves_queries_and_excludes_credentials() {
    let (base, mut requests, task) = mock(vec![
        response(
            200,
            json!({"id":"hook","name":"CI","type":1,"channel_id":Uuid::nil(),"token":"hidden"}),
        ),
        response(401, json!({"message":"reflected hidden"})),
    ])
    .await;
    let hook = WebhookClient::new(format!("{base}/webhook/hook/hidden?wait=true&thread_id=42"));
    let info = hook.get_info().await.unwrap();
    assert_eq!(info.name.as_deref(), Some("CI"));
    assert!(!format!("{info:?}").contains("hidden"));
    assert!(serde_json::to_value(info).unwrap().get("token").is_none());
    assert!(requests
        .recv()
        .await
        .unwrap()
        .starts_with("GET /webhook/hook/hidden?thread_id=42 "));
    let error = hook.get_info().await.unwrap_err();
    assert_eq!(error.status_code(), Some(401));
    assert!(!error.to_string().contains("hidden"));
    task.await.unwrap();
}

#[tokio::test]
async fn bio_accepts_guide_limit_and_preserves_server_rejection() {
    let (base, _requests, task) = mock(vec![
        response(200, profile()),
        response(400, json!({"error":"bio too long"})),
    ])
    .await;
    let client = Client::builder("secret").base_url(base).build();
    let request = UpdateBotProfile {
        display_name: None,
        bio: Some("🦀".repeat(300)),
    };
    client.update_me(&request).await.unwrap();
    assert_eq!(
        client.update_me(&request).await.unwrap_err().status_code(),
        Some(400)
    );
    assert!(client
        .update_me(&UpdateBotProfile {
            bio: Some("x".repeat(301)),
            ..request
        })
        .await
        .is_err());
    task.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn empty_poll_acknowledgements_do_not_replace_unchanged_checkpoint() {
    use std::os::unix::fs::MetadataExt;
    let path = std::env::temp_dir().join(format!("rappin-idle-{}", Uuid::new_v4()));
    let cursor = EventCursor::new(&path);
    cursor.save(7).await.unwrap();
    let inode = tokio::fs::metadata(&path).await.unwrap().ino();
    let (base, _requests, task) = mock(vec![
        response(200, json!({"events":[],"last":7})),
        response(200, json!({"events":[],"last":8})),
    ])
    .await;
    let mut consumer =
        EventConsumer::new(Client::builder("secret").base_url(base).build()).cursor_file(&path);
    consumer.poll(0).await.unwrap();
    consumer.acknowledge(7).await.unwrap();
    assert_eq!(tokio::fs::metadata(&path).await.unwrap().ino(), inode);
    consumer.poll(0).await.unwrap();
    consumer.acknowledge(8).await.unwrap();
    assert_eq!(cursor.load().await.unwrap(), Some(8));
    assert_ne!(tokio::fs::metadata(&path).await.unwrap().ino(), inode);
    tokio::fs::remove_file(path).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn server_deadline_overrides_inferred_age_and_old_events_expire() {
    let client = Client::new("not-used");
    let now = chrono::Utc::now();
    let with_deadline:InteractionEvent=serde_json::from_value(json!({"id":"server","token":"test","created_at":now-chrono::Duration::hours(1),"expires_at":now+chrono::Duration::minutes(1)})).unwrap();
    assert_eq!(
        client
            .interaction_state(&with_deadline)
            .await
            .unwrap()
            .phase,
        ResponsePhase::Unanswered
    );
    let old: InteractionEvent = serde_json::from_value(
        json!({"id":"old","token":"test","created_at":now-chrono::Duration::minutes(16)}),
    )
    .unwrap();
    assert_eq!(
        client.interaction_state(&old).await.unwrap().phase,
        ResponsePhase::Expired
    );
}

#[tokio::test]
async fn member_events_invalidate_the_application_backend() {
    let backend = Arc::new(AppCache::default());
    let member: MemberDetail =
        serde_json::from_value(json!({"id":Uuid::nil(),"username":"member"})).unwrap();
    backend
        .store_member(Uuid::nil(), Uuid::nil(), &member)
        .await;
    let bot = Bot::builder("not-used")
        .lookup_cache_backend(backend.clone())
        .sync_commands(false)
        .log(false)
        .build();
    bot.client()
        .cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    bot.process_event(serde_json::from_value(json!({"id":1,"type":"member_leave","data":{"server_id":Uuid::nil(),"user_id":Uuid::nil(),"username":"member"}})).unwrap()).await.unwrap();
    assert!(backend.get_member(Uuid::nil(), Uuid::nil()).await.is_none());
}
