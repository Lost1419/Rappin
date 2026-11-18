//! Use one status handler for prefix and slash commands.
use rappin::{ApiExt, AvailabilityPolicy, Bot, CommandContext, Result};

async fn status(ctx: CommandContext) -> Result<()> {
    ctx.parse::<()>()?;
    let me = ctx.get_me().await?;
    ctx.reply(format!("{} is ready", me.username)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    Bot::from_env()?
        .configure(|settings| {
            settings.transport.max_write_retries = 0;
            settings.transport.read_availability = Some(AvailabilityPolicy::default());
        })
        .command("status", "Show bot status", status)
        .build()?
        .run()
        .await
}
