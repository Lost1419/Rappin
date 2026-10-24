use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use rappin::prelude::*;

struct CounterCog {
    loaded: Arc<AtomicBool>,
    unloaded: Arc<AtomicBool>,
}

#[async_trait]
impl Cog for CounterCog {
    fn name(&self) -> &str {
        "counter"
    }

    async fn on_load(&self, _client: &Client) -> Result<()> {
        self.loaded.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn on_unload(&self, _client: &Client) -> Result<()> {
        self.unloaded.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn commands(&self) -> Vec<CogCommand> {
        vec![CogCommand::new("count", "Count up", |_ctx| async move {
            Ok(())
        })]
    }

    fn buttons(&self) -> Vec<CogButton> {
        vec![CogButton::new("count:", |_ctx| async move { Ok(()) })]
    }
}

#[tokio::test]
async fn test_cog_lifecycle_and_dynamic_update() {
    let loaded = Arc::new(AtomicBool::new(false));
    let unloaded = Arc::new(AtomicBool::new(false));

    let bot = Bot::builder("yb_dummy_token").sync_commands(false).build();

    let handle = bot.handle();
    assert!(!handle.has_cog("counter").await);

    // 1. Add cog dynamically
    let cog = CounterCog {
        loaded: loaded.clone(),
        unloaded: unloaded.clone(),
    };

    handle.add_cog(cog).await.unwrap();
    assert!(handle.has_cog("counter").await);
    assert!(loaded.load(Ordering::SeqCst));
    assert_eq!(handle.list_cogs().await, vec!["counter"]);

    // 2. Remove cog dynamically
    handle.remove_cog("counter").await.unwrap();
    assert!(!handle.has_cog("counter").await);
    assert!(unloaded.load(Ordering::SeqCst));

    // 3. Test CogBuilder
    let dynamic_cog = CogBuilder::new("fun")
        .command("joke", "Tell a joke", |_ctx| async move { Ok(()) })
        .button("vote:", |_ctx| async move { Ok(()) })
        .build();

    handle.add_cog(dynamic_cog).await.unwrap();
    assert!(handle.has_cog("fun").await);

    // 4. Test Cog factory & reload_cog_by_name
    let reload_flag = Arc::new(AtomicBool::new(false));
    let flag_clone = reload_flag.clone();

    handle
        .register_cog_factory("fun", move || {
            flag_clone.store(true, Ordering::SeqCst);
            Box::new(
                CogBuilder::new("fun")
                    .command("joke2", "New joke", |_ctx| async move { Ok(()) })
                    .build(),
            )
        })
        .await;

    handle.reload_cog_by_name("fun").await.unwrap();
    assert!(handle.has_cog("fun").await);
    assert!(reload_flag.load(Ordering::SeqCst));
}
