//! Load this example through HotReload. The parent owns the API transport and retry policies.
use rappin::{ApiExt, CommandContext, Result, WorkerCog};

async fn status(ctx: CommandContext) -> Result<()> {
    ctx.parse::<()>()?;
    let me = ctx.get_me().await?;
    ctx.reply(format!("{} is ready", me.username)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    WorkerCog::new("status")
        .command("status", "Show bot status", status)
        .run()
        .await
}
