mod support;
use rappin::*;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use support::*;
use uuid::Uuid;

#[tokio::test]
async fn new_endpoints_preserve_partial_payloads_and_decode_responses() {
    let id = Uuid::nil();
    let role =
        json!({"id":id,"name":"Helper","mentionable":false,"permissions":{"manage_messages":true}});
    let event = json!({"id":id,"title":"Meetup","starts_at":"2026-11-01T12:00:00Z","location":"Somewhere","repeat":"weekly","status":"scheduled","interested_count":2});
    let (base, mut requests, task) = mock(vec![
        response(200,json!({"role":role})), response(200,json!({"role":role})), response(200,json!({"ok":true})), response(200,json!({"ok":true})),
        response(200,json!({"overrides":[{"role_id":null,"allow":["send_messages"],"deny":[]}]})),
        response(200,json!({"ok":true})), response(200,json!({"ok":true})), response(200,json!({"ok":true})), response(200,json!({"ok":true})),
        response(200,json!({"server_id":id,"member_count":7,"online_count":3})),
        response(200,json!({"invites":[{"code":"hi","url":"https://www.yapin.gg/invite/hi","uses":2,"max_uses":0,"expires_at":null,"vanity":false}]})),
        response(200,json!({"events":[event]})), response(200,json!({"event":event})),
    ]).await;
    let client = Client::builder("fixture-secret").base_url(base).build();
    let mut create = CreateRole::new("Helper");
    create.permissions = Some([("manage_messages".into(), true)].into());
    assert_eq!(
        client.create_role(id, &create).await.unwrap().mentionable,
        Some(false)
    );
    client
        .edit_role(
            id,
            id,
            &EditRole {
                mentionable: Some(false),
                permissions: Some([("send_messages".into(), false)].into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    client.delete_role(id, id).await.unwrap();
    client
        .set_everyone_permissions(id, &[("send_messages".into(), false)].into())
        .await
        .unwrap();
    assert_eq!(
        client.get_channel_permissions(id).await.unwrap()[0].role_id,
        None
    );
    let perms = PermissionOverride {
        allow: vec!["send_messages".into()],
        deny: vec![],
    };
    client
        .set_channel_permissions(id, PermissionTarget::Everyone, &perms)
        .await
        .unwrap();
    client
        .clear_channel_permissions(id, PermissionTarget::Role(id))
        .await
        .unwrap();
    client
        .set_category_permissions(id, id, PermissionTarget::Role(id), &perms)
        .await
        .unwrap();
    client
        .clear_category_permissions(id, id, PermissionTarget::Everyone)
        .await
        .unwrap();
    assert_eq!(client.get_member_counts(id).await.unwrap().online_count, 3);
    assert_eq!(client.get_invites(id).await.unwrap()[0].uses, 2);
    assert_eq!(
        client.get_server_events(id).await.unwrap()[0].interested_count,
        2
    );
    let mut create = CreateServerEvent::new("Meetup", "2026-11-01T12:00:00Z".parse().unwrap());
    create.location = Some("Somewhere".into());
    create.repeat = Some(EventRepeat::Weekly);
    client.create_server_event(id, &create).await.unwrap();
    let expected = [
        format!("POST /servers/{id}/roles "),
        format!("PATCH /servers/{id}/roles/{id} "),
        format!("DELETE /servers/{id}/roles/{id} "),
        format!("PATCH /servers/{id}/roles/everyone "),
        format!("GET /channels/{id}/permissions "),
        format!("PUT /channels/{id}/permissions/everyone "),
        format!("DELETE /channels/{id}/permissions/{id} "),
        format!("PUT /servers/{id}/categories/{id}/permissions/{id} "),
        format!("DELETE /servers/{id}/categories/{id}/permissions/everyone "),
        format!("GET /servers/{id}/counts "),
        format!("GET /servers/{id}/invites "),
        format!("GET /servers/{id}/events "),
        format!("POST /servers/{id}/events "),
    ];
    for (i, path) in expected.iter().enumerate() {
        let request = requests.recv().await.unwrap();
        assert!(request.starts_with(path), "{request}");
        if i == 1 {
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(
                body,
                json!({"mentionable":false,"permissions":{"send_messages":false}})
            );
        }
        if i == 12 {
            assert!(request.contains("\"repeat\":\"weekly\""));
            assert!(!request.contains("channel_id"));
        }
    }
    task.await.unwrap();
}

#[tokio::test]
async fn mutations_invalidate_role_and_member_snapshots_even_on_ambiguous_error() {
    let id = Uuid::nil();
    let (base, mut requests, task) = mock(vec![
        response(200, json!({"roles":[]})),
        response(
            200,
            json!({"member":{"id":id,"username":"member","roles":[]}}),
        ),
        response(503, json!({"error":"unavailable"})),
    ])
    .await;
    let client = Client::builder("fixture-secret")
        .base_url(base)
        .lookup_cache(CachePolicy::new(4, Duration::from_secs(60)))
        .build();
    client.get_roles(id).await.unwrap();
    client.get_member(id, id).await.unwrap();
    assert!(client.cache().get_roles(id).await.is_some());
    assert!(client.cache().get_member(id, id).await.is_some());
    assert_eq!(
        client.delete_role(id, id).await.unwrap_err().status_code(),
        Some(503)
    );
    assert!(client.cache().get_roles(id).await.is_none());
    assert!(client.cache().get_member(id, id).await.is_none());
    for _ in 0..3 {
        requests.recv().await.unwrap();
    }
    task.await.unwrap();
}

#[derive(Default)]
struct Invites(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl EventHandler for Invites {
    async fn on_invite_create(&self, _: &Client, event: &InviteEvent) {
        assert_eq!(event.invite.code, "test");
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    async fn on_invite_delete(&self, _: &Client, _: &InviteEvent) {
        self.0.fetch_add(10, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl Cog for Invites {
    fn name(&self) -> &str {
        "invites"
    }
    async fn on_invite_create(&self, _: &Client, _: &InviteEvent) {
        self.0.fetch_add(100, Ordering::SeqCst);
    }
    async fn on_invite_delete(&self, _: &Client, _: &InviteEvent) {
        self.0.fetch_add(1000, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn invites_dispatch_to_handlers_cogs_and_collectors_with_unknown_fallback() {
    let count = Arc::new(AtomicUsize::new(0));
    let bot = Bot::builder("fixture-secret")
        .log(false)
        .sync_commands(false)
        .event_handler(Invites(count.clone()))
        .build();
    bot.cache()
        .set_me(serde_json::from_value(profile()).unwrap())
        .await;
    bot.handle().add_cog(Invites(count.clone())).await.unwrap();
    let mut collector = bot
        .client()
        .events()
        .collect::<InviteEvent>()
        .subscribe()
        .unwrap();
    for (id, kind) in [(1, "invite_create"), (2, "invite_delete")] {
        let event: Event = serde_json::from_value(
            json!({"id":id,"type":kind,"data":{"server_id":Uuid::nil(),"code":"test","uses":0}}),
        )
        .unwrap();
        bot.process_event(event).await.unwrap();
        assert_eq!(collector.next().await.unwrap().unwrap().invite.code, "test");
    }
    assert_eq!(count.load(Ordering::SeqCst), 1111);
    let event: Event =
        serde_json::from_value(json!({"type":"invite_create","data":{"code":"broken"}})).unwrap();
    assert!(matches!(
        event.parse_data().unwrap(),
        EventData::Unknown { .. }
    ));
}

#[test]
fn metadata_is_optional_and_scheduled_event_validation_is_local() {
    let member:MemberDetail=serde_json::from_value(json!({"id":Uuid::nil(),"username":"member","invite_code":"hello","inviter_id":Uuid::nil()})).unwrap();
    assert_eq!(member.invite_code.as_deref(), Some("hello"));
    let join: MemberJoinEvent = serde_json::from_value(
        json!({"user_id":Uuid::nil(),"server_id":Uuid::nil(),"invite_code":"hello"}),
    )
    .unwrap();
    assert_eq!(join.invite_code.as_deref(), Some("hello"));
    let server: Server = serde_json::from_value(
        json!({"id":Uuid::nil(),"owner_id":Uuid::nil(),"name":"test","online_count":5}),
    )
    .unwrap();
    assert_eq!(server.online_count, Some(5));
    let mut event = CreateServerEvent::new("Test", "2026-11-01T12:00:00Z".parse().unwrap());
    assert!(event.validate().is_err());
    event.location = Some("Outside".into());
    assert!(event.validate().is_ok());
    event.ends_at = Some(event.starts_at);
    assert!(event.validate().is_err());
    event.ends_at = None;
    event.channel_id = Some(Uuid::nil());
    assert!(event.validate().is_err());
    assert!("../../me".parse::<PermissionTarget>().is_err());
}

#[tokio::test]
async fn permission_checks_use_new_dictionary_and_preserve_explicit_false() {
    let id = Uuid::nil();
    for allowed in [true, false] {
        let (base,mut requests,task)=mock(vec![
            response(200,json!({"server":{"id":id,"name":"test","owner_id":Uuid::new_v4()}})),
            response(200,json!({"member":{"id":id,"username":"member","roles":[id]}})),
            response(200,json!({"roles":[{"id":id,"name":"helper","manage_messages":!allowed,"permissions":{"manage_messages":allowed}}]})),
        ]).await;
        let client = Arc::new(Client::builder("fixture-secret").base_url(base).build());
        let message =
            serde_json::from_value(json!({"id":id,"channel_id":id,"server_id":id,"author_id":id}))
                .unwrap();
        let ctx = MessageContext::new(client, message, None, String::new(), None);
        assert_eq!(
            Check::permissions([Permission::manage_messages()])
                .check(&ctx)
                .await
                .is_ok(),
            allowed
        );
        for _ in 0..3 {
            requests.recv().await.unwrap();
        }
        task.await.unwrap();
    }
}
