//! YAPPER_BOT_TOKEN=... cargo run --features gateway --example gateway_api_requests
use rappin::{gateway::GatewayRequestConfig, Bot, Error, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| Error::Config("set YAPPER_BOT_TOKEN".into()))?;
    let bot = Bot::builder(token)
        .configure(|settings| {
            settings.transport.gateway_url = Some((rappin::gateway::DEFAULT_GATEWAY).into());
            settings.transport.mode = rappin::TransportMode::GatewayPreferred;
        })
        .command(
            "ping",
            "Reply through the selected API transport",
            |ctx| async move {
                ctx.reply("pong").await?;
                let status = ctx.client.gateway_request_status();
                eprintln!(
                    "JSON transport: {:?}; gateway requests enabled: {}",
                    status.selected, status.capability
                );
                Ok(())
            },
        )
        .build()
        .expect("valid bot configuration");
    bot.client()
        .configure_gateway_requests(Some(GatewayRequestConfig::default()))?;
    // HTTPS is selected before ready.requests=true and for uploads/events.
    // An ambiguous submitted write is returned as unknown; no HTTPS replay occurs.
    bot.run().await
}
