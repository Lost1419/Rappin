use std::time::Duration;
use uuid::Uuid;

use rappin::cache::Cache;
use rappin::cooldown::{Cooldown, CooldownType};
use rappin::models::channel::{Channel, ChannelKind};
use rappin::models::component::Component;
use rappin::models::embed::Embed;
use rappin::models::server::Server;
use rappin::models::user::BotProfile;
use rappin::oauth2::OAuth2Client;
use rappin::paginator::{ConfirmDialog, Paginator};
use rappin::webhook::WebhookClient;

#[tokio::test]
async fn test_cache_servers_and_channels() {
    let cache = Cache::new();

    // Verify initial empty state
    assert!(cache.get_me().await.is_none());
    assert!(cache.get_servers().await.is_empty());
    assert!(cache.get_channels().await.is_empty());

    // Test BotProfile caching
    let profile = BotProfile {
        id: Uuid::new_v4(),
        username: "testbot".to_string(),
        display_name: Some("Test Bot".to_string()),
        avatar_path: None,
        banner_path: None,
        bio: Some("A test bot".to_string()),
    };
    cache.set_me(profile.clone()).await;
    let cached_profile = cache.get_me().await.expect("expected cached profile");
    assert_eq!(cached_profile.username, "testbot");

    // Test Server caching
    let s1 = Server {
        id: Uuid::new_v4(),
        name: "Rust Hangout".to_string(),
        icon_path: None,
        owner_id: Uuid::new_v4(),
        member_count: Some(42),
    };
    let s2 = Server {
        id: Uuid::new_v4(),
        name: "Yapper Lounge".to_string(),
        icon_path: None,
        owner_id: Uuid::new_v4(),
        member_count: Some(10),
    };
    cache.update_servers(vec![s1.clone(), s2.clone()]).await;

    assert_eq!(cache.get_servers().await.len(), 2);
    let fetched = cache.get_server(&s1.id).await.expect("found s1");
    assert_eq!(fetched.name, "Rust Hangout");

    // Test Channel caching
    let ch1 = Channel {
        id: Uuid::new_v4(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        server_id: Some(s1.id),
        position: Some(0),
        category: None,
        category_id: None,
        private: Some(false),
        read_only: Some(false),
        slowmode: Some(0),
    };
    let ch2 = Channel {
        id: Uuid::new_v4(),
        name: "voice-chat".to_string(),
        kind: ChannelKind::Voice,
        server_id: Some(s1.id),
        position: Some(1),
        category: None,
        category_id: None,
        private: Some(false),
        read_only: Some(false),
        slowmode: Some(0),
    };
    let ch3 = Channel {
        id: Uuid::new_v4(),
        name: "lobby".to_string(),
        kind: ChannelKind::Text,
        server_id: Some(s2.id),
        position: Some(0),
        category: None,
        category_id: None,
        private: Some(false),
        read_only: Some(false),
        slowmode: Some(0),
    };
    cache
        .update_channels(vec![ch1.clone(), ch2.clone(), ch3.clone()])
        .await;

    assert_eq!(cache.get_channels().await.len(), 3);
    let s1_channels = cache.get_channels_for_server(&s1.id).await;
    assert_eq!(s1_channels.len(), 2);
    let s2_channels = cache.get_channels_for_server(&s2.id).await;
    assert_eq!(s2_channels.len(), 1);

    // Test removal
    cache.remove_channel(&ch1.id).await;
    assert_eq!(cache.get_channels().await.len(), 2);
    assert!(cache.get_channel(&ch1.id).await.is_none());

    cache.remove_server(&s1.id).await;
    assert_eq!(cache.get_servers().await.len(), 1);
    assert!(cache.get_server(&s1.id).await.is_none());
}

#[tokio::test]
async fn test_cooldown_rate_limiting() {
    let cd = Cooldown::user(Duration::from_millis(100));
    assert_eq!(cd.kind(), CooldownType::User);
    assert_eq!(cd.duration(), Duration::from_millis(100));

    // First check should pass
    let remaining = cd.check_key("user_123").await;
    assert!(remaining.is_none());

    // Immediate second check should be rate limited
    let remaining = cd.check_key("user_123").await;
    assert!(remaining.is_some());
    let rem = remaining.unwrap();
    assert!(rem <= Duration::from_millis(100) && rem > Duration::ZERO);

    // Different user should pass
    assert!(cd.check_key("user_456").await.is_none());

    // Resetting key allows user_123 through again
    cd.reset_key("user_123").await;
    assert!(cd.check_key("user_123").await.is_none());
}

#[tokio::test]
async fn test_paginator_buttons_and_parsing() {
    let embeds = vec![
        Embed::new().title("Page 1").description("First page"),
        Embed::new().title("Page 2").description("Second page"),
        Embed::new().title("Page 3").description("Third page"),
    ];

    let paginator = Paginator::new(embeds).with_id("test_pg");
    assert_eq!(paginator.total_pages(), 3);
    assert_eq!(paginator.id(), "test_pg");

    // Page 0
    let row0 = paginator.build_buttons(0);
    assert_eq!(row0.components.len(), 3);
    // Prev disabled
    assert!(row0.components[0].disabled);
    // Indicator
    assert_eq!(row0.components[1].label.as_deref(), Some("1/3"));
    assert!(row0.components[1].disabled);
    // Next enabled
    assert!(!row0.components[2].disabled);
    assert_eq!(row0.components[2].custom_id.as_deref(), Some("test_pg:1"));

    // Page 1
    let row1 = paginator.build_buttons(1);
    assert!(!row1.components[0].disabled);
    assert_eq!(row1.components[0].custom_id.as_deref(), Some("test_pg:0"));
    assert_eq!(row1.components[1].label.as_deref(), Some("2/3"));
    assert!(!row1.components[2].disabled);
    assert_eq!(row1.components[2].custom_id.as_deref(), Some("test_pg:2"));

    // Page 2 (last page)
    let row2 = paginator.build_buttons(2);
    assert!(!row2.components[0].disabled);
    assert_eq!(row2.components[1].label.as_deref(), Some("3/3"));
    assert!(row2.components[2].disabled);

    // Custom ID parsing
    assert_eq!(paginator.parse_page_from_custom_id("test_pg:2"), Some(2));
    assert_eq!(paginator.parse_page_from_custom_id("test_pg:0"), Some(0));
    assert_eq!(paginator.parse_page_from_custom_id("test_pg:noop"), None);
    assert_eq!(paginator.parse_page_from_custom_id("other:1"), None);

    // Message building
    let msg = paginator.build_message(0);
    let embeds = msg.embeds.expect("embeds present");
    assert_eq!(embeds.len(), 1);
    assert_eq!(embeds[0].title.as_deref(), Some("Page 1"));

    let components = msg.components.expect("components present");
    assert_eq!(components.len(), 1);
}

#[test]
fn test_confirm_dialog() {
    let dialog = ConfirmDialog::new("Are you sure you want to delete this channel?")
        .with_id("test_cnf")
        .with_labels("Yes, Delete", "Cancel");

    assert_eq!(dialog.confirm_custom_id(), "test_cnf:yes");
    assert_eq!(dialog.cancel_custom_id(), "test_cnf:no");

    let msg = dialog.build_message();
    assert_eq!(
        msg.content.as_deref(),
        Some("Are you sure you want to delete this channel?")
    );
    let components = msg.components.expect("components present");
    assert_eq!(components.len(), 1);
    match &components[0] {
        Component::Row(row) => {
            assert_eq!(row.components.len(), 2);
            assert_eq!(row.components[0].label.as_deref(), Some("Yes, Delete"));
            assert_eq!(row.components[0].custom_id.as_deref(), Some("test_cnf:yes"));
            assert_eq!(row.components[1].label.as_deref(), Some("Cancel"));
            assert_eq!(row.components[1].custom_id.as_deref(), Some("test_cnf:no"));
        }
        _ => panic!("Expected Component::Row"),
    }
}

#[test]
fn test_oauth2_authorization_url() {
    let client = OAuth2Client::new(
        "client_123",
        "secret_456",
        "https://example.com/oauth/callback",
    );

    let auth_url = client.authorization_url(&["identify", "servers"], Some("state_xyz"));
    assert!(auth_url.starts_with("https://www.yapin.gg/oauth/authorize?"));
    assert!(auth_url.contains("client_id=client_123"));
    assert!(auth_url.contains("redirect_uri=https%3A%2F%2Fexample.com%2Foauth%2Fcallback"));
    assert!(auth_url.contains("response_type=code"));
    assert!(auth_url.contains("scope=identify+servers"));
    assert!(auth_url.contains("state=state_xyz"));
}

#[test]
fn test_webhook_client_construct() {
    let _ = WebhookClient::new("https://www.yapin.gg/api/bot/v1/webhooks/dummy");
}
