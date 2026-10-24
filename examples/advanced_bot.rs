use rappin::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let token = std::env::var("YAPPER_BOT_TOKEN").unwrap_or_else(|_| "yb_sample_token".to_string());

    let bot = Bot::builder(token)
        .prefix("!")
        .prefix("/")
        // Command with automatic per-user rate limit (10s)
        .command_with_cooldown(
            "daily",
            "Claim your daily reward (10s cooldown demo)",
            Cooldown::user(Duration::from_secs(10)),
            |ctx| async move {
                ctx.reply("🎁 You claimed your daily reward! Come back later.")
                    .await?;
                Ok(())
            },
        )
        // Command showcasing the interactive Paginator
        .command(
            "help_pages",
            "Browse help pages with navigation buttons",
            |ctx| async move {
                let pages = vec![
                    Embed::new()
                        .title("📘 Help Menu — Page 1/3")
                        .description("Welcome to the Bot guide! Use buttons below to navigate.")
                        .color(EmbedColor::BLUE)
                        .field(
                            "Commands",
                            "`!daily`, `!help_pages`, `!servers`, `!prompt`",
                            false,
                        ),
                    Embed::new()
                        .title("⚙️ Help Menu — Page 2/3")
                        .description("Bot configuration and cache options.")
                        .color(EmbedColor::PURPLE)
                        .field(
                            "Cache",
                            "Bot automatically caches server and channel data.",
                            false,
                        ),
                    Embed::new()
                        .title("🎉 Help Menu — Page 3/3")
                        .description("Thank you for using rappin!")
                        .color(EmbedColor::GREEN)
                        .field("Links", "[Website](https://www.yapin.gg)", false),
                ];

                ctx.send_paginated(pages).await?;
                Ok(())
            },
        )
        // Command showing interactive ConfirmDialog
        .command(
            "prompt",
            "Try an interactive confirmation dialog",
            |ctx| async move {
                ctx.send_confirm("Do you accept the server rules?").await?;
                Ok(())
            },
        )
        // Command demonstrating instant in-memory cache lookup
        .command(
            "servers",
            "List cached servers without HTTP overhead",
            |ctx| async move {
                let cached = ctx.cache().get_servers().await;
                if cached.is_empty() {
                    ctx.reply("No servers cached yet. Fetching from API...")
                        .await?;
                    let servers = ctx.client.get_servers().await?;
                    ctx.reply(format!("Fetched & cached {} server(s)!", servers.len()))
                        .await?;
                } else {
                    let names: Vec<String> = cached.iter().map(|s| s.name.clone()).collect();
                    ctx.reply(format!("Cached server list: {}", names.join(", ")))
                        .await?;
                }
                Ok(())
            },
        )
        // Paginator button click router
        .button("pg_", |ctx| async move {
            // For production bots, you can maintain the paginator state or parse target page
            if let Some(target_page) = ctx
                .custom_id()
                .split(':')
                .nth(1)
                .and_then(|p| p.parse::<usize>().ok())
            {
                ctx.update(format!("Navigated to page {target_page}"))
                    .await?;
            }
            Ok(())
        })
        // ConfirmDialog button click router
        .button("cnf_", |ctx| async move {
            if ctx.custom_id().ends_with(":yes") {
                ctx.update("✅ You confirmed!").await?;
            } else {
                ctx.update("❌ You cancelled!").await?;
            }
            Ok(())
        })
        .build();

    println!("Starting advanced bot... Press Ctrl+C to exit.");
    bot.run().await?;
    Ok(())
}
