//! Minimal WebSocket event bot. Run with:
//! `YAPPER_BOT_TOKEN=... cargo run --features gateway --example gateway_bot`

use rappin::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| Error::Config("set YAPPER_BOT_TOKEN".into()))?;

    let bot = Bot::builder(token)
        .configure(|settings| {
            settings.transport.gateway_url = Some((rappin::gateway::DEFAULT_GATEWAY).into());
            settings.transport.mode = rappin::TransportMode::GatewayPreferred;
        })
        .command("ping", "Check if the bot is responding", |ctx| async move {
            ctx.reply("pong").await?;
            Ok(())
        })
        .build()
        .expect("valid bot configuration");

    bot.run().await
}
