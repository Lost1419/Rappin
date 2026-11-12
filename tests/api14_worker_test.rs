#![cfg(feature = "hot-reload")]
mod support;
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    *,
};
use serde_json::json;
use uuid::Uuid;

struct Project(std::path::PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_resource_listener_uses_shared_api_and_preserves_partial_failure() {
    let project =
        Project(std::env::temp_dir().join(format!("rappin-api14-worker-{}", Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    let sdk = serde_json::to_string(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(
        project.0.join("Cargo.toml"),
        format!(
            r#"
[package]
name="api14-resource-worker"
version="0.0.0"
edition="2021"
[workspace]
[dependencies]
rappin={{package="rappin-rs",path={sdk},features=["hot-reload"]}}
tokio={{version="1",features=["macros","rt-multi-thread"]}}
serde_json="1"
"#
        ),
    )
    .unwrap();
    std::fs::write(project.0.join("src/main.rs"),r#"
use rappin::{hot_reload::{WorkerCog,Listener,CogEvent},ResourceApiExt,ResourceEventKind,PermissionTarget,PermissionOverride,Result,YappinError};
#[tokio::main]
async fn main()->Result<()> {
    assert!(std::env::var_os("YAPPER_BOT_TOKEN").is_none());
    WorkerCog::new("api14")
        .listen(Listener::ResourceEvent,|ctx|async move {
            let CogEvent::ResourceEvent{event}=&ctx.event else {panic!("wrong event kind")};
            assert_eq!(event.data["type"],"voice");
            if event.kind==ResourceEventKind::ChannelDelete { return Ok(()); }
            if event.kind==ResourceEventKind::ChannelUpdate {
                return Err(YappinError::PartialWrite {
                    completed:vec![serde_json::json!({"resource":event.id})],failed_step:2,
                    source:Box::new(YappinError::Api{status:504,message:"uncertain fixture write".into(),retry_after:Some(0.5)})
                });
            }
            let actor="00000000-0000-0000-0000-000000000004".parse().unwrap();
            let access=ctx.get_member_permissions(event.server_id,actor,Some(event.id)).await?;
            assert_eq!(access.permission("view_channel"),Some(true));
            let object=ctx.resolve_id(event.id).await?;
            assert_eq!(object.object,"channel");
            ctx.set_channel_override(event.id,PermissionTarget::Everyone,&PermissionOverride{allow:vec!["use_activities".into()],deny:vec![]}).await?;
            Ok(())
        }).run().await
}
"#).unwrap();
    let server = Uuid::from_u128(1);
    let channel = Uuid::from_u128(3);
    let user = Uuid::from_u128(4);
    let (base,mut calls,http)=support::mock(vec![
        support::response(200,json!({"permissions":{"object":"permissions","server_id":server,"user_id":user,"channel_id":channel,"owner":false,"administrator":false,"permissions":{"view_channel":true}}})),
        support::response(200,json!({"id":channel,"object":"channel","server_id":server,"channel_id":channel})),
        support::response(200,json!({"override":null})),
    ]).await;
    let bot = Bot::builder("fixture-parent-secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let controller = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "api14-resource-worker").offline(true),
    )
    .await
    .unwrap();
    for (event_id, kind) in [
        (1, "channel_create"),
        (2, "channel_update"),
        (3, "channel_delete"),
    ] {
        let event=serde_json::from_value(json!({"id":event_id,"type":kind,"data":{"id":channel,"server_id":server,"object":"channel","type":"voice"}})).unwrap();
        let result = bot.process_event(event).await;
        if event_id != 2 {
            result.unwrap();
        } else {
            let failure = result.unwrap_err().to_json();
            assert_eq!(failure["kind"], "partial_write");
            assert_eq!(failure["completed_steps"].as_array().unwrap().len(), 1);
            assert_eq!(failure["outcome"], "unknown");
        }
    }
    assert!(calls
        .recv()
        .await
        .unwrap()
        .starts_with(&format!("GET /channels/{channel}/permissions/{user}")));
    assert!(calls
        .recv()
        .await
        .unwrap()
        .starts_with(&format!("GET /resolve/{channel}")));
    let write = calls.recv().await.unwrap();
    assert!(write.starts_with(&format!("PUT /channels/{channel}/overrides/everyone")));
    assert!(write.contains("use_activities"));
    assert!(calls.try_recv().is_err());
    controller.unload().await.unwrap();
    http.await.unwrap();
}
