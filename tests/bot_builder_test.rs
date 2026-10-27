use rappin::models::MessageEvent;
use rappin::prelude::*;

#[tokio::test]
async fn test_bot_builder_configuration() {
    let bot = Bot::builder("yb_dummy_token_123")
        .base_url("https://custom.yapin.gg/api/bot/v1")
        .prefix("!")
        .prefix("yap!")
        .ignore_bots(false)
        .sync_commands(false)
        .command("ping", "Check ping", |_ctx| async move { Ok(()) })
        .command("roll", "Roll dice", |_ctx| async move { Ok(()) })
        .button("vote:", |_ctx| async move { Ok(()) })
        .on_command_error(|_ctx, _err| async move {})
        .on_button_error(|_ctx, _err| async move {})
        .build();

    assert_eq!(
        bot.client().base_url(),
        "https://custom.yapin.gg/api/bot/v1"
    );
}

#[test]
fn test_message_context_argv_splitting() {
    let raw = serde_json::json!({
        "id": "11111111-1111-1111-1111-111111111111",
        "channel_id": "22222222-2222-2222-2222-222222222222",
        "author_id": "33333333-3333-3333-3333-333333333333",
        "content": "!roll 20 advantage"
    });

    let msg: MessageEvent = serde_json::from_value(raw).unwrap();
    let client = std::sync::Arc::new(Client::new("token"));
    let ctx = MessageContext::new(
        client,
        msg,
        Some("roll".to_string()),
        "20 advantage".to_string(),
        None,
    );

    assert_eq!(ctx.command.as_deref(), Some("roll"));
    assert_eq!(ctx.args, "20 advantage");
    assert_eq!(ctx.argv, vec!["20", "advantage"]);
    assert_eq!(
        ctx.author_mention(),
        "<@33333333-3333-3333-3333-333333333333>"
    );
}
