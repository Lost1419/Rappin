//! A separately compiled cog: edit the reply while hot_reload_bot stays running.
use rappin::{hot_reload::WorkerCog, Result};
#[tokio::main]
async fn main() -> Result<()> {
    WorkerCog::new("fun")
        .command("ping", "Check the reloadable cog", |ctx| async move {
            ctx.reply("Pong from the reloadable Rust cog!").await?;
            Ok(())
        })
        .button("hot:hello", |ctx| async move {
            ctx.ephemeral("Hello from a reloadable cog").await
        })
        .run()
        .await
}
