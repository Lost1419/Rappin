//! One status handler for prefix/message and interaction-based slash commands.
use rappin::{ApiExt, AvailabilityPolicy, Bot, CommandContext, Result, YappinError};

async fn status(ctx: CommandContext) -> Result<()> {
    ctx.parse::<()>()?;
    let me = ctx.api().get_me().await?;
    ctx.reply(format!("{} is ready", me.username)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
    let recovery = AvailabilityPolicy::default();
    Bot::builder(token)
        .max_write_retries(0)
        .read_availability_policy(recovery.clone())
        .command_shared("status", "Show bot status", status)
        .build()
        .run_supervised(recovery)
        .await
}
