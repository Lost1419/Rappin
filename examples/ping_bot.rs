//! A complete example of a Yapin' bot built with `rappin`.
//!
//! To run:
//! ```sh
//! export YAPPER_BOT_TOKEN="your_bot_token_here"
//! cargo run --example ping_bot
//! ```

use std::env;
use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Enable tracing/logging
    tracing_subscriber::fmt::init();

    let token = env::var("YAPPER_BOT_TOKEN")
        .unwrap_or_else(|_| "PASTE_YOUR_BOT_TOKEN_HERE".to_string());

    let bot = Bot::builder(token)
        .prefix("!")
        .prefix("/")
        .command("ping", "Check if the bot is alive", |ctx| async move {
            ctx.reply("pong 🏓").await?;
            Ok(())
        })
        .command("roll", "Roll a die, like /roll 20", |ctx| async move {
            let sides = ctx
                .argv
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(6)
                .clamp(2, 1000);

            // Simple pseudo-random die roll
            let roll = 1 + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
                % sides);

            ctx.reply(format!("🎲 Rolled: {roll} (d{sides})")).await?;
            Ok(())
        })
        .command("menu", "Show an interactive menu with buttons", |ctx| async move {
            let row = ActionRow::new(vec![
                Button::success("Apples", "vote:apples").with_emoji("🍎"),
                Button::primary("Oranges", "vote:oranges").with_emoji("🍊"),
                Button::danger("Cancel", "vote:cancel"),
                Button::link("Yapin' Devs", "https://www.yapin.gg/developers"),
            ]);

            let msg = CreateMessage::reply("Which fruit do you prefer?", ctx.message_id())
                .action_row(row);

            ctx.reply_with(msg).await?;
            Ok(())
        })
        .button("vote:", |ctx| async move {
            let choice = ctx.custom_id().strip_prefix("vote:").unwrap_or("unknown");
            if choice == "cancel" {
                ctx.update("Selection cancelled.").await?;
            } else {
                ctx.ephemeral(format!("You voted for **{choice}**!")).await?;
            }
            Ok(())
        })
        .build();

    println!("Starting Yapin' bot... Press Ctrl+C to exit.");
    bot.run().await?;
    Ok(())
}

