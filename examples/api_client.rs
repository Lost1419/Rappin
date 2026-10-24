//! Demonstrates using the low-level and high-level REST API client directly.
//!
//! To run:
//! ```sh
//! export YAPPER_BOT_TOKEN="your_bot_token_here"
//! cargo run --example api_client
//! ```

use std::env;
use rappin::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let token = env::var("YAPPER_BOT_TOKEN")
        .unwrap_or_else(|_| "PASTE_YOUR_BOT_TOKEN_HERE".to_string());

    let client = Client::new(token);

    // 1. Fetch authenticated bot profile
    println!("Fetching bot profile...");
    let me = client.get_me().await?;
    println!("Bot Name: {} (ID: {})", me.username, me.id);
    if let Some(avatar) = me.avatar_url() {
        println!("Avatar CDN URL: {}", avatar);
    }

    // 2. Set bot status and custom activity
    println!("Updating bot status to online...");
    client
        .set_status(BotStatus::Online, Some("Exploring the universe 🚀"))
        .await?;

    // 3. List all servers the bot is in
    println!("Listing servers...");
    let servers = client.get_servers().await?;
    println!("The bot is in {} server(s):", servers.len());
    for s in servers {
        println!("- {} (ID: {}, Owner: {})", s.name, s.id, s.owner_id);

        // List text channels in this server
        let resp = client.get_server_channels(s.id, None).await?;
        for ch in resp.channels {
            println!("    Channel #{} ({:?})", ch.name, ch.kind);
        }
    }

    Ok(())
}

