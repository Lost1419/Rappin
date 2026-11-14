#![cfg(feature = "hot-reload")]
mod support;
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    Bot,
};
use serde_json::{json, Value};
use uuid::Uuid;
struct Project(std::path::PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn message(id: u128) -> Value {
    json!({"id":Uuid::from_u128(id),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"content":"ok","created_at":"2026-11-02T00:00:00Z"})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_chunked_upload_public_metadata_and_dm_reactions_use_parent_transport() {
    let directory = std::env::temp_dir().join(format!("rappin-api12-worker-{}", Uuid::new_v4()));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    let project = Project(directory);
    let sdk = serde_json::to_string(env!("CARGO_MANIFEST_DIR")).unwrap();
    std::fs::write(project.0.join("Cargo.toml"),format!("[package]\nname=\"api12-worker\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin={{package=\"rappin-rs\",path={sdk},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\n")).unwrap();
    std::fs::write(project.0.join("src/main.rs"),r#"
use rappin::{hot_reload::{WorkerCog,Listener,CogEvent},ApiExt,CreateMessage,UploadFile,Result};
#[tokio::main] async fn main()->Result<()> {
 assert!(std::env::var_os("YAPPER_BOT_TOKEN").is_none());
 WorkerCog::new("api12")
 .command("upload","test",|ctx|async move {
   assert_eq!(ctx.get_api_version().await?.api,"1.2.0");
   let files=[UploadFile::new("large.txt",vec![b'x';1_300_000])];
   let messages=ctx.send_files(ctx.channel_id().unwrap(),&CreateMessage::new("large"),&files).await?;
   assert_eq!(messages.len(),1);Ok(())
 })
 .listen(Listener::DmMessage,|ctx|async move {assert!(matches!(ctx.event,CogEvent::DmMessage{..}));assert!(ctx.event.message().unwrap().is_dm);ctx.say("dm reply").await?;Ok(())})
 .listen(Listener::ReactionAdd,|ctx|async move {if let CogEvent::ReactionAdd{event}=&ctx.event{assert!(event.is_dm);}else{panic!("wrong event");}ctx.say("reaction reply").await?;Ok(())})
 .run().await
}
"#).unwrap();
    let version = json!({"api":"1.2.0","build":"fixture","sdk":{"js":"1.5.0","python":"1.5.0"},"packages":{"npm":"yapin","pypi":"yapin"}});
    let (base, mut calls, http) = support::mock(vec![
        support::response(200, version),
        support::response(201, json!({"message":message(1),"messages":[message(1)]})),
        support::response(201, json!({"message":message(2)})),
        support::response(201, json!({"message":message(3)})),
    ])
    .await;
    let bot = Bot::builder("yb_api12_parent_secret")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build()
        .expect("valid bot configuration");
    bot.cache()
        .set_me(serde_json::from_value(support::profile()).unwrap())
        .await;
    let controller = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "api12-worker").offline(true),
    )
    .await
    .unwrap();
    bot.process_event(serde_json::from_value(support::event(1, "upload")).unwrap())
        .await
        .unwrap();
    bot.process_event(serde_json::from_value(json!({"id":2,"type":"dm_message","data":{"id":Uuid::from_u128(100),"channel_id":Uuid::nil(),"server_id":null,"author_id":Uuid::nil(),"content":"hello"}})).unwrap()).await.unwrap();
    bot.process_event(serde_json::from_value(json!({"id":3,"type":"reaction_add","data":{"channel_id":Uuid::nil(),"message_id":Uuid::from_u128(100),"user_id":Uuid::nil(),"emoji":"x","server_id":null,"is_dm":true}})).unwrap()).await.unwrap();
    let public = calls.recv().await.unwrap();
    assert!(public.starts_with("GET /version "));
    assert!(!public.to_lowercase().contains("authorization:"));
    let upload = calls.recv().await.unwrap();
    assert!(upload.contains("large.txt"));
    assert!(upload.contains(&"x".repeat(100_000)));
    assert!(upload
        .to_lowercase()
        .contains("authorization: bot yb_api12_parent_secret"));
    assert!(calls.recv().await.unwrap().contains("dm reply"));
    assert!(calls.recv().await.unwrap().contains("reaction reply"));
    assert_eq!(controller.generation(), 1);
    controller.unload().await.unwrap();
    http.await.unwrap();
}
