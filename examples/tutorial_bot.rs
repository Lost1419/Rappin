//! First commands, derived arguments, buttons, shared state and a native cog.
//! Run with YAPPER_BOT_TOKEN and features gateway,derive.
use rappin::{
    bot::{PersistenceConfig, TransportConfig},
    prelude::*,
    CogBuilder,
};
use std::sync::Arc;

struct Settings {
    greeting: String,
}

#[derive(FromArgs)]
struct Greet {
    /// Person to greet. Quote names containing spaces.
    name: String,
    /// Optional greeting suffix.
    #[arg(rest, max = 80)]
    note: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let bot = Bot::from_env()?
        .transport(TransportConfig::gateway())
        .configure(|settings| settings.persistence = PersistenceConfig::file("./state/events.json"))
        .state(Arc::new(Settings {
            greeting: "Hello".into(),
        }))
        .help_command("help")
        .command("ping", "Check the bot", |ctx| async move {
            ctx.reply("pong").await?;
            Ok(())
        })
        .command_spec(
            CommandSpec::<Greet>::new("greet", "Greet someone"),
            |ctx, args| async move {
                let settings = ctx.state::<Settings>()?;
                ctx.reply(format!(
                    "{}, {}! {}",
                    settings.greeting,
                    args.name,
                    args.note.unwrap_or_default()
                ))
                .await?;
                Ok(())
            },
        )
        .command("menu", "Show a button", |ctx| async move {
            ctx.reply_with(
                CreateMessage::new("Choose a fruit")
                    .component(Component::row([Button::primary("Apple", "fruit:apple")])),
            )
            .await?;
            Ok(())
        })
        .button("fruit:", |ctx| async move {
            ctx.ephemeral("You chose Apple").await?;
            Ok(())
        })
        .command_spec(
            CommandSpec::<()>::new("owner", "Check bot ownership").check(Check::owner_only()),
            |ctx, ()| async move {
                ctx.reply("You own this bot").await?;
                Ok(())
            },
        )
        .cog(
            CogBuilder::new("utilities")
                .command_spec(
                    CommandSpec::<()>::new("status", "Check the cog"),
                    |ctx, ()| async move {
                        ctx.reply("Ready").await?;
                        Ok(())
                    },
                )
                .build(),
        )
        .build()?;
    bot.run().await
}
