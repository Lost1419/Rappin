//! API 1.3 selects and modals; platform feature availability is enforced upstream.
use rappin::{CreateMessage, InteractionValues, Modal, SelectMenu, SelectOption, TextInput, *};

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("YAPPER_BOT_TOKEN")
        .map_err(|_| Error::Config("YAPPER_BOT_TOKEN is required".into()))?;
    Bot::builder(token)
        .command("form", "Choose a topic and open a form", |ctx| async move {
            let menu = SelectMenu::new("form:topic")
                .option(SelectOption::new("Rust", "rust"))
                .option(SelectOption::new("Other", "other"));
            let body = CreateMessage::new("Choose a topic").component(menu.row());
            ctx.reply_with(body).await?;
            Ok(())
        })
        .on_select(|ctx| async move {
            if ctx.custom_id() != "form:topic" {
                return Ok(());
            }
            if let Some(InteractionValues::Selected(values)) = &ctx.values {
                let title = if values.iter().any(|v| v == "rust") {
                    "Rust feedback"
                } else {
                    "Feedback"
                };
                ctx.show_modal(
                    &Modal::new("form:submit", title)
                        .input(TextInput::paragraph("feedback", "Your feedback")),
                )
                .await?;
            }
            Ok(())
        })
        .on_modal(|ctx| async move {
            if ctx.custom_id() == "form:submit" {
                // Validate/store submitted values in application-owned storage
                // before reporting durable acceptance.
                ctx.ephemeral("Form received by the example.").await?;
            }
            Ok(())
        })
        .build()
        .expect("valid bot configuration")
        .run()
        .await
}
