//! Run through HotReload with interaction_commands(true) on the parent.
//! Configure read recovery and mutation retry budgets on the parent's builder.
use rappin::{hot_reload::WorkerCog, ApiExt, CommandContext, Result};

async fn status(ctx: CommandContext) -> Result<()> {
    ctx.parse::<()>()?;
    let me = ctx.api().get_me().await?;
    ctx.reply(format!("{} is ready", me.username)).await
}

#[tokio::main]
async fn main() -> Result<()> {
    WorkerCog::new("status")
        .command_shared("status", "Show bot status", status)
        .run()
        .await
}
