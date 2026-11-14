//! Run with YAPPER_BOT_TOKEN set and `cargo run --example api14_resources`.
//! Computed permission reads require the platform's API 1.4 test rollout.
use rappin::*;

struct Resources;
#[async_trait::async_trait]
impl EventHandler for Resources {
    async fn on_resource_event(&self, _client: &Client, event: &ResourceEvent) -> Result<()> {
        println!(
            "{} {} in {}",
            event.kind.as_str(),
            event.id,
            event.server_id
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| Error::Config("set YAPPER_BOT_TOKEN".into()))?;
    Bot::builder(token)
        .event_handler(Resources)
        .command(
            "permissions",
            "Inspect your computed channel access",
            |ctx| async move {
                let server = ctx
                    .server_id()
                    .ok_or_else(|| CommandError::Denied("Use this command in a server".into()))?;
                let access = ctx
                    .get_member_permissions(
                        server,
                        ctx.author_id()
                            .ok_or_else(|| CommandError::Denied("Requester required".into()))?,
                        ctx.channel_id(),
                    )
                    .await?;
                ctx.reply(format!(
                    "View channel {:?}, manage messages {:?}",
                    access.permission("view_channel"),
                    access.permission("manage_messages")
                ))
                .await?;
                Ok(())
            },
        )
        .build()
        .expect("valid bot configuration")
        .run()
        .await
}
