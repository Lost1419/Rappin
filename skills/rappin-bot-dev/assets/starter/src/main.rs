use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .expect("Set YAPPER_BOT_TOKEN before starting the bot");
    let bot = Bot::builder(token)
        .command("ping", "Check if the bot is alive", |ctx| async move {
            ctx.reply("pong 🦀").await?;
            Ok(())
        })
        .build();
    bot.run().await
}
