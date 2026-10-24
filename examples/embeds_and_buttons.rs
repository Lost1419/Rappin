//! Demonstrates creating rich embeds, button rows, and Components V2 layouts.
//!
//! To run:
//! ```sh
//! export YAPPER_BOT_TOKEN="your_bot_token_here"
//! cargo run --example embeds_and_buttons
//! ```

use rappin::prelude::*;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let token =
        env::var("YAPPER_BOT_TOKEN").unwrap_or_else(|_| "PASTE_YOUR_BOT_TOKEN_HERE".to_string());

    let bot = Bot::builder(token)
        .command("card", "Send a rich embed card", |ctx| async move {
            let embed = Embed::new()
                .title("Server Statistics")
                .description("Here is the latest snapshot for this server.")
                .color(EmbedColor::GREEN)
                .field("Online Members", "42", true)
                .field("Bot Version", "0.1.0 (Rust)", true)
                .footer(EmbedFooter::new("Powered by yappin-rs"))
                .timestamp(chrono::Utc::now());

            let row = ActionRow::new(vec![
                Button::primary("Refresh", "stat:refresh").with_emoji("🔄"),
                Button::link("Dashboard", "https://www.yapin.gg"),
            ]);

            let msg = CreateMessage::reply("📊 Here are your stats:", ctx.message_id())
                .embed(embed)
                .action_row(row);

            ctx.reply_with(msg).await?;
            Ok(())
        })
        .command(
            "v2",
            "Send a modern Components V2 layout",
            |ctx| async move {
                let layout = Container::new()
                    .accent(EmbedColor::BLUE)
                    .add(TextDisplay::new(
                        "# Announcement\nWelcome to our community!",
                    ))
                    .add(Separator::divider())
                    .add(
                        Section::new("Join our verified members pool by clicking below:")
                            .accessory(Button::success("Verify Identity", "verify:now")),
                    );

                ctx.send_layout(vec![layout]).await?;
                Ok(())
            },
        )
        .button("stat:refresh", |ctx| async move {
            ctx.ephemeral("Stats refreshed just now!").await?;
            Ok(())
        })
        .button("verify:now", |ctx| async move {
            ctx.ephemeral("Verification submitted! An admin will review shortly.")
                .await?;
            Ok(())
        })
        .build();

    println!("Bot started! Try typing /card or /v2");
    bot.run().await?;
    Ok(())
}
