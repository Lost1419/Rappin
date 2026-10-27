//! Demonstrates replacing native cog instances using already compiled code.
//! For edited Rust source without restarting the bot, see hot_reload_bot.rs.
//!
//! To run:
//! ```sh
//! export YAPPER_BOT_TOKEN="your_bot_token_here"
//! cargo run --example cogs_bot
//! ```

use std::env;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use rappin::prelude::*;

/// A Cog grouping fun commands with internal state.
struct FunCog {
    joke_count: Arc<AtomicU32>,
}

impl FunCog {
    pub fn new() -> Self {
        Self {
            joke_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

#[async_trait]
impl Cog for FunCog {
    fn name(&self) -> &str {
        "fun"
    }

    async fn on_load(&self, _client: &Client) -> Result<()> {
        println!("[FunCog] Loaded!");
        Ok(())
    }

    async fn on_unload(&self, _client: &Client) -> Result<()> {
        println!("[FunCog] Unloaded!");
        Ok(())
    }

    fn commands(&self) -> Vec<CogCommand> {
        let count = self.joke_count.clone();

        vec![
            CogCommand::new("joke", "Tells a funny programmer joke", move |ctx| {
                let count = count.clone();
                async move {
                    let total = count.fetch_add(1, Ordering::SeqCst) + 1;
                    ctx.reply(format!(
                        "Why do programmers prefer dark mode? Because light attracts bugs! (Joke #{total})"
                    ))
                    .await?;
                    Ok(())
                }
            }),
            CogCommand::new("flip", "Flip a coin", |ctx| async move {
                let outcome = if rand_bool() {
                    "Heads! 🪙"
                } else {
                    "Tails! 🪙"
                };
                ctx.reply(outcome).await?;
                Ok(())
            }),
        ]
    }
}

fn rand_bool() -> bool {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        & 1)
        == 0
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let token =
        env::var("YAPPER_BOT_TOKEN").unwrap_or_else(|_| "PASTE_YOUR_BOT_TOKEN_HERE".to_string());

    let bot = Bot::builder(token)
        .prefix("!")
        .prefix("/")
        // 1. Initial cog registered at startup
        .cog(FunCog::new())
        // 2. An admin command to dynamically reload or load cogs at runtime!
        .command(
            "reload",
            "Reload a cog dynamically while running",
            |ctx| async move {
                let owner = ctx
                    .cache()
                    .get_me()
                    .await
                    .and_then(|me| me.bot)
                    .map(|bot| bot.owner_id);
                if owner != Some(ctx.author_id()) {
                    ctx.reply("Only the bot owner can reload cogs.").await?;
                    return Ok(());
                }
                let cog_name = ctx.argv.first().map(|s| s.as_str()).unwrap_or("fun");

                if let Some(bot_handle) = ctx.bot() {
                    if cog_name == "fun" {
                        bot_handle.reload_cog(FunCog::new()).await?;
                        ctx.reply(format!(
                            "✅ Successfully reloaded `{cog_name}` cog in real-time!"
                        ))
                        .await?;
                    } else {
                        ctx.reply(format!("Unknown cog `{cog_name}`")).await?;
                    }
                }
                Ok(())
            },
        )
        .command("cogs", "List currently active cogs", |ctx| async move {
            if let Some(bot_handle) = ctx.bot() {
                let list = bot_handle.list_cogs().await;
                ctx.reply(format!("Active cogs: `{}`", list.join(", ")))
                    .await?;
            }
            Ok(())
        })
        .build();

    println!("Starting bot with Cogs support... Type /joke, /flip, or /reload fun");
    bot.run().await?;
    Ok(())
}
