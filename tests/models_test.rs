use chrono::Utc;
use serde_json::json;
use uuid::Uuid;
use rappin::prelude::*;
use rappin::models::*;

#[test]
fn test_user_deserialization_and_helpers() {
    let raw = json!({
        "id": "11111111-2222-3333-4444-555555555555",
        "username": "alice",
        "display_name": "Alice Wonderland",
        "avatar_path": "avatars/alice.png",
        "badges": ["verified", "supporter"]
    });

    let user: User = serde_json::from_value(raw).unwrap();
    assert_eq!(user.username, "alice");
    assert_eq!(user.display_name.as_deref(), Some("Alice Wonderland"));
    assert_eq!(
        user.avatar_url().unwrap(),
        "https://files.yapper.website/avatars/avatars/alice.png"
    );
    assert_eq!(
        user.mention(),
        "<@11111111-2222-3333-4444-555555555555>"
    );
}

#[test]
fn test_bot_profile_and_status() {
    let raw = json!({
        "id": "22222222-3333-4444-5555-666666666666",
        "username": "yappin_bot",
        "display_name": "Yappin Bot",
        "avatar_path": "/icon.svg",
        "banner_path": "banners/banner.png",
        "bio": "A friendly bot"
    });

    let profile: BotProfile = serde_json::from_value(raw).unwrap();
    assert_eq!(profile.username, "yappin_bot");
    assert_eq!(
        profile.avatar_url().unwrap(),
        "https://www.yapin.gg/icon.svg"
    );
    assert_eq!(
        profile.banner_url().unwrap(),
        "https://files.yapper.website/banners/banners/banner.png"
    );

    let status_req = StatusUpdateRequest::new(BotStatus::Online).with_activity("Helping users");
    let serialized = serde_json::to_value(status_req).unwrap();
    assert_eq!(serialized["status"], "online");
    assert_eq!(serialized["activity"], "Helping users");
}

#[test]
fn test_embed_builder_and_serialization() {
    let embed = Embed::new()
        .title("Test Card")
        .description("Hello [world](https://example.com)!")
        .color(EmbedColor::BLUE)
        .field("First", "Value 1", true)
        .field("Second", "Value 2", false)
        .footer(EmbedFooter::new("Footer Text"))
        .author(EmbedAuthor::new("Antigravity"))
        .image("https://example.com/image.png")
        .thumbnail("https://example.com/thumb.png")
        .timestamp(Utc::now());

    let val = serde_json::to_value(&embed).unwrap();
    assert_eq!(val["title"], "Test Card");
    assert_eq!(val["description"], "Hello [world](https://example.com)!");
    assert_eq!(val["color"], 0x5865F2);
    assert_eq!(val["fields"].as_array().unwrap().len(), 2);
    assert_eq!(val["footer"]["text"], "Footer Text");
    assert_eq!(val["author"]["name"], "Antigravity");
    assert_eq!(val["image"]["url"], "https://example.com/image.png");
}

#[test]
fn test_buttons_and_action_row() {
    let btn1 = Button::success("Vote Yes", "vote:yes").with_emoji("👍");
    let btn2 = Button::danger("Vote No", "vote:no").with_emoji("👎");
    let btn3 = Button::link("Documentation", "https://www.yapin.gg/developers");

    let row = ActionRow::new(vec![btn1, btn2, btn3]);
    let val = serde_json::to_value(&row).unwrap();

    assert_eq!(val["type"], "row");
    let comps = val["components"].as_array().unwrap();
    assert_eq!(comps.len(), 3);
    assert_eq!(comps[0]["style"], "success");
    assert_eq!(comps[0]["custom_id"], "vote:yes");
    assert_eq!(comps[0]["emoji"], "👍");
    assert_eq!(comps[1]["style"], "danger");
    assert_eq!(comps[2]["style"], "link");
    assert_eq!(comps[2]["url"], "https://www.yapin.gg/developers");
}

#[test]
fn test_components_v2_layout() {
    let layout = Container::new()
        .accent(0x5865F2)
        .add(TextDisplay::new("# Welcome to Yapin'"))
        .add(
            Section::new("Click to verify your account")
                .accessory(Button::primary("Verify", "verify:account")),
        )
        .add(Separator::divider())
        .add(FileDisplay::new("https://example.com/rules.pdf"));

    let val = serde_json::to_value(&layout).unwrap();
    assert_eq!(val["type"], 17);
    assert_eq!(val["accent_color"], 0x5865F2);

    let parts = val["components"].as_array().unwrap();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0]["type"], 10);
    assert_eq!(parts[0]["content"], "# Welcome to Yapin'");
    assert_eq!(parts[1]["type"], 9);
    assert_eq!(parts[2]["type"], 14);
    assert_eq!(parts[3]["type"], 13);
}

#[test]
fn test_create_and_edit_message_builders() {
    let _user_id = Uuid::new_v4();
    let msg_id = Uuid::new_v4();

    let create = CreateMessage::reply("Hey there!", msg_id)
        .embed(Embed::new().title("Notice"))
        .action_row(ActionRow::new(vec![Button::secondary("Dismiss", "dismiss")]));

    let val = serde_json::to_value(&create).unwrap();
    assert_eq!(val["content"], "Hey there!");
    assert_eq!(val["reply_to"], msg_id.to_string());
    assert_eq!(val["embeds"].as_array().unwrap().len(), 1);
    assert_eq!(val["components"].as_array().unwrap().len(), 1);

    let edit = EditMessage::new().content("Updated text").clear_embeds();
    let edit_val = serde_json::to_value(&edit).unwrap();
    assert_eq!(edit_val["content"], "Updated text");
    assert!(edit_val["embeds"].is_null());
}

#[test]
fn test_event_parsing() {
    let raw_msg_event = json!({
        "id": 105,
        "type": "message",
        "data": {
            "id": "33333333-4444-5555-6666-777777777777",
            "channel_id": "44444444-5555-6666-7777-888888888888",
            "author_id": "55555555-6666-7777-8888-999999999999",
            "content": "/ping test arg",
            "command": "ping",
            "args": "test arg",
            "author_bot": false,
            "created_at": "2026-10-24T12:00:00Z"
        }
    });

    let ev: Event = serde_json::from_value(raw_msg_event).unwrap();
    assert_eq!(ev.id, 105);
    assert_eq!(ev.kind, "message");

    let parsed = ev.parse_data().unwrap();
    match parsed {
        EventData::Message(m) => {
            assert_eq!(m.command.as_deref(), Some("ping"));
            assert_eq!(m.args.as_deref(), Some("test arg"));
            assert_eq!(m.author_bot, Some(false));
        }
        _ => panic!("Expected MessageEvent"),
    }

    let raw_inter_event = json!({
        "id": 106,
        "type": "interaction",
        "data": {
            "id": "inter_abc123",
            "token": "tok_xyz789",
            "custom_id": "vote:apples",
            "user_id": "55555555-6666-7777-8888-999999999999",
            "channel_id": "44444444-5555-6666-7777-888888888888"
        }
    });

    let ev2: Event = serde_json::from_value(raw_inter_event).unwrap();
    match ev2.parse_data().unwrap() {
        EventData::Interaction(i) => {
            assert_eq!(i.id, "inter_abc123");
            assert_eq!(i.token, "tok_xyz789");
            assert_eq!(i.custom_id.as_deref(), Some("vote:apples"));
        }
        _ => panic!("Expected InteractionEvent"),
    }
}

#[test]
fn test_voice_ticket_deserialization() {
    let raw = json!({
        "url": "wss://livekit.yapin.gg",
        "token": "jwt.token.here",
        "e2ee": true,
        "voice_muted": false,
        "channel_id": "66666666-7777-8888-9999-000000000000",
        "server_id": "77777777-8888-9999-0000-111111111111",
        "expires_in": 60
    });

    let ticket: VoiceTicket = serde_json::from_value(raw).unwrap();
    assert_eq!(ticket.url, "wss://livekit.yapin.gg");
    assert!(ticket.e2ee);
    assert_eq!(ticket.expires_in, Some(60));
}
