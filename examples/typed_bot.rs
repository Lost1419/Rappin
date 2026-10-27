//! Run with YAPPER_BOT_TOKEN set: cargo run --example typed_bot
use rappin::prelude::*;
use std::time::Duration;

struct Roll {
    sides: BoundedU32<2, 100>,
    label: Option<String>,
}
impl FromArgs for Roll {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self {
            sides: args.required("sides")?,
            label: args.optional("label")?,
        })
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![
            ArgumentInfo::required("sides", "2 to 100 sides"),
            ArgumentInfo::optional("label", "Quote labels with spaces"),
        ]
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
    let bot = Bot::builder(token)
        .prefix("!")
        .mention_prefix(true)
        .handler_concurrency(8)
        .pending_batches(4)
        // Authorization checks still refresh by default; lookup caching is an explicit choice.
        .lookup_cache(CachePolicy::new(256, Duration::from_secs(30)))
        .command_spec(
            CommandSpec::<Roll>::new("dice roll", "Roll a die").alias("roll"),
            |ctx, args| async move {
                let value = uuid::Uuid::new_v4().as_u128() % u128::from(args.sides.0) + 1;
                ctx.reply(format!(
                    "{}: {value} on d{}",
                    args.label.as_deref().unwrap_or("Roll"),
                    args.sides.0
                ))
                .await?;
                Ok(())
            },
        )
        .command_spec(
            CommandSpec::<()>::new("owner", "Check bot ownership").check(Check::owner_only()),
            |ctx, ()| async move {
                ctx.reply("You own this bot.").await?;
                Ok(())
            },
        )
        .command("question", "Ask for a reply", |ctx| async move {
            let reply = ctx.replies().timeout(Duration::from_secs(30)).subscribe()?;
            ctx.reply("What should I call you?").await?;
            match reply.wait().await {
                Ok(answer) => {
                    ctx.reply(format!(
                        "Hello, {}",
                        answer.content.as_deref().unwrap_or("friend")
                    ))
                    .await?;
                }
                Err(WaitError::Timeout) => {
                    ctx.reply("Time is up.").await?;
                }
                Err(WaitError::Closed) => (), // Graceful shutdown cancels active waiters.
                Err(error) => return Err(error.into()),
            }
            Ok(())
        })
        .help_command("help")
        .build();
    let handle = bot.handle();
    let monitor = tokio::spawn(async move {
        let mut status = handle.subscribe_transport();
        while status.changed().await.is_ok() {
            eprintln!("Transport: {:?}", *status.borrow_and_update());
        }
    });
    let result = bot.run().await;
    monitor.abort();
    result
}
