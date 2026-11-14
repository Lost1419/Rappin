use rappin::{
    bot::{PersistenceConfig, TransportConfig},
    prelude::*,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    Bot::from_env()?
        .transport(TransportConfig::gateway())
        .configure(|settings| {
            settings.persistence = PersistenceConfig::file("./state/events.json");
        })
        .help_command("help")
        .command("ping", "Check if the bot is alive", |ctx| async move {
            ctx.reply("pong 🦀").await?;
            Ok(())
        })
        .build()?
        .run()
        .await
}
