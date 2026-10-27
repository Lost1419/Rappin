//! A separately compiled cog: edit the reply while hot_reload_bot stays running.
use rappin::{hot_reload::WorkerCog, prelude::*};
struct Roll(BoundedU32<2, 100>);
impl FromArgs for Roll {
    fn parse(args: &mut Arguments) -> std::result::Result<Self, CommandError> {
        Ok(Self(args.required("sides")?))
    }
    fn describe() -> Vec<ArgumentInfo> {
        vec![ArgumentInfo::required("sides", "2 to 100 sides")]
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    WorkerCog::new("fun")
        .command("ping", "Check the reloadable cog", |ctx| async move {
            ctx.reply("Pong from the reloadable Rust cog!").await?;
            Ok(())
        })
        .command_typed(
            "roll",
            "Roll a die in a worker",
            |ctx, Roll(sides)| async move {
                let value = uuid::Uuid::new_v4().as_u128() % u128::from(sides.0) + 1;
                ctx.reply(format!("Rolled {value} on d{}", sides.0)).await
            },
        )
        .command("question", "Ask for a reply", |ctx| async move {
            let answer = ctx
                .prompt_for_reply(
                    "What should I call you?",
                    std::time::Duration::from_secs(30),
                )
                .await?;
            ctx.reply(format!(
                "Hello, {}",
                answer.content.as_deref().unwrap_or("friend")
            ))
            .await
        })
        .button("hot:hello", |ctx| async move {
            ctx.ephemeral("Hello from a reloadable cog").await
        })
        .run()
        .await
}
