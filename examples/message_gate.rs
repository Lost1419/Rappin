//! Run with YAPPER_BOT_TOKEN and BOT_ALLOWED_USER (a user UUID).
//! This demonstrates an application-owned allowlist, not platform ACL inference.
use rappin::prelude::*;
use uuid::Uuid;

struct NaturalLanguage;
#[async_trait::async_trait]
impl EventHandler for NaturalLanguage {
    async fn on_message_result(&self, ctx: &MessageContext) -> Result<MessageDisposition> {
        if ctx.command.is_none() && ctx.content().eq_ignore_ascii_case("hello") {
            ctx.reply("Hello! Your message passed the application gate.")
                .await?;
            return Ok(MessageDisposition::Handled);
        }
        Ok(MessageDisposition::Continue)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| YappinError::Config("set YAPPER_BOT_TOKEN".into()))?;
    let allowed: Uuid = std::env::var("BOT_ALLOWED_USER")
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| YappinError::Config("set BOT_ALLOWED_USER to a user UUID".into()))?;
    Bot::builder(token)
        .message_gate(move |ctx| async move {
            if ctx.author_id() != allowed {
                // A failed reply returns Err and retains the checkpoint barrier.
                ctx.reply("This bot is restricted by its application allowlist.")
                    .await?;
                return Ok(MessageDisposition::Handled);
            }
            Ok(MessageDisposition::Continue)
        })
        .event_handler(NaturalLanguage)
        .on_message_failure(|_, failure| async move {
            eprintln!(
                "event {} {:?}/{}: {}",
                failure.event_id,
                failure.stage,
                failure.handler,
                failure.error.to_json()
            );
            // Typed rejections receive a checked reply; all other errors stop intake.
            Ok(FailureDisposition::Default)
        })
        .command("ping", "Check the bot", |ctx| async move {
            ctx.reply("pong").await?;
            Ok(())
        })
        .build()
        .run()
        .await
}
