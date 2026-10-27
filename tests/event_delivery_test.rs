mod support;
use rappin::*;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use support::*;
fn cursor() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("rappin-test-{}.json", uuid::Uuid::new_v4()))
}
#[tokio::test]
async fn consumer_replays_until_explicit_ack_and_restarts_from_disk() {
    let path = cursor();
    EventCursor::new(&path).save(10).await.unwrap();
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"events":[event(11,"ping")],"last":11})),
        response(200, json!({"events":[],"last":12})),
    ])
    .await;
    let client = Client::builder("secret").base_url(base).build();
    let mut consumer = EventConsumer::new(client.clone()).cursor_file(&path);
    assert_eq!(consumer.poll(0).await.unwrap().events.len(), 1);
    assert_eq!(consumer.poll(0).await.unwrap().events.len(), 1);
    assert!(consumer.acknowledge(99).await.is_err());
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(10));
    consumer.acknowledge(11).await.unwrap();
    let mut restarted = EventConsumer::new(client).cursor_file(&path);
    restarted.poll(0).await.unwrap();
    restarted.acknowledge(12).await.unwrap();
    assert!(requests.recv().await.unwrap().contains("after=10"));
    assert!(requests.recv().await.unwrap().contains("after=11"));
    task.await.unwrap();
    tokio::fs::remove_file(path).await.unwrap();
}
struct Unload;
#[async_trait::async_trait]
impl Cog for Unload {
    fn name(&self) -> &str {
        "unload"
    }
    async fn on_message(&self, ctx: &MessageContext) {
        ctx.bot().unwrap().remove_cog("unload").await.unwrap();
    }
}
#[tokio::test]
async fn cog_can_remove_itself_from_callback_without_deadlock() {
    let (base, _requests, task) = mock(vec![response(200, profile())]).await;
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build();
    bot.add_cog(Unload).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        bot.process_event(serde_json::from_value(event(1, "none")).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!bot.handle().has_cog("unload").await);
    task.await.unwrap();
}
struct StopAfter {
    count: Arc<AtomicUsize>,
    total: usize,
}
#[async_trait::async_trait]
impl EventHandler for StopAfter {
    async fn on_message(&self, ctx: &MessageContext) {
        if self.count.fetch_add(1, Ordering::SeqCst) + 1 == self.total {
            ctx.bot().unwrap().stop();
        }
    }
}
#[tokio::test]
async fn bot_bounds_work_and_checkpoints_after_all_handlers_finish() {
    let path = cursor();
    EventCursor::new(&path).save(1).await.unwrap();
    let events: Vec<_> = (2..=6).map(|i| event(i, "work")).collect();
    let (base, _requests, task) = mock(vec![
        response(200, profile()),
        response(200, json!({"events":events,"last":6})),
    ])
    .await;
    let active = Arc::new(AtomicUsize::new(0));
    let max = Arc::new(AtomicUsize::new(0));
    let a = active.clone();
    let m = max.clone();
    let p = path.clone();
    let bot = Bot::builder("secret")
        .base_url(base)
        .log(false)
        .sync_commands(false)
        .cursor_file(&path)
        .handler_concurrency(2)
        .event_handler(StopAfter {
            count: Arc::new(AtomicUsize::new(0)),
            total: 5,
        })
        .command("work", "work", move |_ctx| {
            let a = a.clone();
            let m = m.clone();
            let p = p.clone();
            async move {
                let current = a.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(current, Ordering::SeqCst);
                assert_eq!(EventCursor::new(p).load().await.unwrap(), Some(1));
                tokio::time::sleep(Duration::from_millis(10)).await;
                a.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build();
    tokio::time::timeout(Duration::from_secs(3), bot.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(max.load(Ordering::SeqCst), 2);
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(6));
    task.await.unwrap();
    tokio::fs::remove_file(path).await.unwrap();
}
#[tokio::test]
async fn failed_handler_does_not_commit_batch() {
    let path = cursor();
    EventCursor::new(&path).save(1).await.unwrap();
    let (base, _requests, task) = mock(vec![
        response(200, profile()),
        response(200, json!({"events":[event(2,"fail")],"last":2})),
    ])
    .await;
    let bot = Bot::builder("secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .cursor_file(&path)
        .command("fail", "fails", |_| async {
            Err(YappinError::Config("failed".into()))
        })
        .build();
    assert!(bot.run().await.is_err());
    assert_eq!(EventCursor::new(&path).load().await.unwrap(), Some(1));
    task.await.unwrap();
    tokio::fs::remove_file(path).await.unwrap();
}
