//! Minimal WebSocket event bot. Run with:
//! `YAPPER_BOT_TOKEN=... cargo run --features gateway --example gateway_bot`

use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;

    let bot = Bot::builder(token)
        .gateway(rappin::gateway::DEFAULT_GATEWAY)
        .command("ping", "Check if the bot is responding", |ctx| async move {
            ctx.reply("pong").await?;
            Ok(())
        })
        .build();

    bot.run().await
}
