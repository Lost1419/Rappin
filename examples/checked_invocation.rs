//! Run with YAPPER_BOT_TOKEN set; send !plan in a server channel.
use rappin::{Bot, Check, CommandSpec, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| rappin::Error::Config("set YAPPER_BOT_TOKEN".into()))?;
    Bot::builder(token)
        .command_spec(
            CommandSpec::<()>::new("greet", "Send a greeting").check(Check::server_only()),
            |ctx, ()| async move {
                ctx.reply("Hello!").await?;
                Ok(())
            },
        )
        .command("plan", "Invoke an installed command", |ctx| async move {
            let message = ctx
                .message()
                .ok_or_else(|| rappin::CommandError::Denied("Send !plan as a message".into()))?;
            let bot = ctx.bot().expect("framework context");
            let prepared = bot.prepare_invocation(message, "greet", "").await?;
            // Consumer policy decides whether these deferred checks are acceptable.
            // Preparation never promises a transaction or reserves authorization.
            eprintln!("deferred checks: {:?}", prepared.report().deferred);
            bot.invoke_prepared(prepared).await.into_result()
        })
        .build()
        .expect("valid bot configuration")
        .run()
        .await
}
