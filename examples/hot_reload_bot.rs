//! Run: cargo run --features hot-reload --example hot_reload_bot
//! Edit examples/hot_cog_worker.rs; only that worker is rebuilt and replaced.
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    prelude::*,
};
use std::time::Duration;
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
    let bot = Bot::builder(token)
        .cursor_file("./state/hot-reload-events.json")
        .build();
    let cog = HotReload::load(
        bot.handle(),
        WorkerProject::example("Cargo.toml", "hot_cog_worker").features(["hot-reload"]),
    )
    .await?;
    let _watch = cog.watch(Duration::from_millis(500)).await?;
    eprintln!("Watching Rust cog source; the parent bot stays running during rebuilds.");
    bot.run().await
}
