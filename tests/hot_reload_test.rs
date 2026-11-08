#![cfg(feature = "hot-reload")]
mod support;
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    Bot, Client, Cog, CogBuilder, Command, Result, YappinError,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

struct Project(PathBuf);
impl Project {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("rappin-hot-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(root.join("src")).await.unwrap();
        tokio::fs::write(root.join("Cargo.toml"),"[package]\nname=\"fixture-worker\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[workspace]\n").await.unwrap();
        Self(root)
    }
    fn spec(&self) -> WorkerProject {
        WorkerProject::new(self.0.join("Cargo.toml"), "fixture-worker").offline(true)
    }
    async fn code(&self, version: &str, commands: &[&str], delay: u64, marker: Option<&Path>) {
        self.code_with_listeners(version, commands, delay, marker, &[])
            .await;
    }
    async fn code_with_listeners(
        &self,
        version: &str,
        commands: &[&str],
        delay: u64,
        marker: Option<&Path>,
        listeners: &[&str],
    ) {
        let ready = json!({"op":"ready","protocol":1,"manifest":{"name":"fixture","commands":commands.iter().map(|n|Command::new(*n,version)).collect::<Vec<_>>(),"buttons":[format!("{version}:")],"listeners":listeners}});
        let api = json!({"op":"api","method":"POST","path":"/channels/00000000-0000-0000-0000-000000000000/messages","body":{"content":version}});
        let source=r#"
use std::io::{BufRead,Write};
fn main(){
    for key in ["YAPPER_BOT_TOKEN","CARGO_REGISTRY_TOKEN","NODE_OPTIONS"] {assert!(std::env::var_os(key).is_none(),"inherited credential environment");}
    println!("{}",__READY__);std::io::stdout().flush().unwrap();
    let stdin=std::io::stdin();let mut lines=stdin.lock().lines();
    let marker:&str=__MARKER__;
    while let Some(line)=lines.next(){
        let line=line.unwrap();
        if line.contains("\"op\":\"shutdown\""){if !marker.is_empty(){std::fs::write(format!("{}.stopped",marker),"stopped").unwrap();}break;}
        assert!(line.contains("\"op\":\"invoke\""));
        if !marker.is_empty(){std::fs::write(marker,"started").unwrap();}
        if !marker.is_empty(){
            let release=format!("{}.release",marker);
            while !std::path::Path::new(&release).exists(){
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }else{
            std::thread::sleep(std::time::Duration::from_millis(__DELAY__));
        }
        println!("{}",__API__);std::io::stdout().flush().unwrap();
        let result=lines.next().unwrap().unwrap();assert!(result.contains("\"op\":\"api_result\""));assert!(!result.contains("yb_private_fixture"));
        println!("{{\"op\":\"done\",\"error\":null}}");std::io::stdout().flush().unwrap();
    }
}
"#.replace("__READY__",&serde_json::to_string(&ready.to_string()).unwrap())
    .replace("__API__",&serde_json::to_string(&api.to_string()).unwrap())
    .replace("__DELAY__",&delay.to_string()).replace("__MARKER__",&serde_json::to_string(&marker.map(|p|p.to_string_lossy().into_owned()).unwrap_or_default()).unwrap());
        tokio::fs::write(self.0.join("src/main.rs"), source)
            .await
            .unwrap();
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn me() -> rappin::BotProfile {
    serde_json::from_value(support::profile()).unwrap()
}
fn event(command: &str) -> rappin::Event {
    serde_json::from_value(support::event(2, command)).unwrap()
}
async fn wait_for(mut predicate: impl AsyncPredicate) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !predicate.check().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
trait AsyncPredicate {
    async fn check(&mut self) -> bool;
}
struct FileExists(PathBuf);
impl AsyncPredicate for FileExists {
    async fn check(&mut self) -> bool {
        self.0.exists()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edited_rust_is_rebuilt_while_inflight_old_code_finishes_and_failed_build_keeps_new() {
    let project = Project::new().await;
    let marker = project.0.join("started");
    project.code("old", &["ping"], 5000, Some(&marker)).await;
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({"echo":"yb_private_fixture"})),
        support::response(200, json!({})),
        support::response(200, json!({})),
        support::response(200, json!({})),
    ])
    .await;
    let bot = Arc::new(
        Bot::builder("yb_private_fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .build(),
    );
    bot.cache().set_me(me()).await;
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let old_pid = reload.process_id().await;
    let old = tokio::spawn({
        let bot = bot.clone();
        async move { bot.process_event(event("ping")).await }
    });
    wait_for(FileExists(marker.clone())).await;
    project.code("new", &["ping", "added"], 0, None).await;
    reload.reload().await.unwrap();
    assert!(
        !old.is_finished(),
        "old request must still be running during swap"
    );
    assert_ne!(old_pid, reload.process_id().await);
    bot.process_event(event("added")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"new\""));
    // Keep the old generation blocked until the new route proves it can reply.
    // Releasing both first permits either process's HTTP request to arrive first.
    assert!(!old.is_finished());
    tokio::fs::write(format!("{}.release", marker.display()), "release")
        .await
        .unwrap();
    old.await.unwrap().unwrap();
    wait_for(FileExists(PathBuf::from(format!(
        "{}.stopped",
        marker.display()
    ))))
    .await;
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"old\""));
    let new_pid = reload.process_id().await;
    tokio::fs::write(project.0.join("src/main.rs"), "this is not valid Rust")
        .await
        .unwrap();
    let error = reload.reload().await.unwrap_err();
    let failure = error.hot_reload_failure().unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Compilation);
    assert!(!failure.candidate_started);
    assert!(!failure.routes_published);
    assert!(!failure.diagnostics.is_empty());
    assert_eq!(reload.process_id().await, new_pid);
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"new\""));
    // A compiling worker with a malformed manifest is also rejected before publication.
    tokio::fs::write(
        project.0.join("src/main.rs"),
        "fn main(){println!(\"not protocol JSON\");}",
    )
    .await
    .unwrap();
    let failure = reload
        .reload()
        .await
        .unwrap_err()
        .hot_reload_failure()
        .unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Protocol);
    assert!(failure.candidate_started);
    assert!(!failure.routes_published);
    assert_eq!(reload.generation(), 2);
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"new\""));
    http.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watcher_reloads_source_and_registration_conflicts_preserve_existing_routes() {
    let project = Project::new().await;
    project.code("one", &["ping"], 0, None).await;
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({})),
        support::response(200, json!({})),
        support::response(200, json!({})),
    ])
    .await;
    let bot = Bot::builder("yb_fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .command("reserved", "native", |_| async { Ok(()) })
        .build();
    bot.cache().set_me(me()).await;
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let watch = reload.watch(Duration::from_millis(30)).await.unwrap();
    project.code("two", &["ping", "added"], 0, None).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        while reload.generation() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    bot.process_event(event("added")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"two\""));
    tokio::fs::write(project.0.join("src/main.rs"), "invalid Rust")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while watch.last_error().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        watch.status().reload_failure.unwrap().phase,
        rappin::ReloadPhase::Compilation
    );
    assert_eq!(reload.generation(), 2);
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"two\""));
    project.code("three", &["ping", "added"], 0, None).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        while reload.generation() < 3 || watch.last_error().is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    drop(watch);
    project.code("conflict", &["reserved"], 0, None).await;
    let failure = reload
        .reload()
        .await
        .unwrap_err()
        .hot_reload_failure()
        .unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Registration);
    assert!(failure.candidate_started);
    assert!(!failure.publication_attempted);
    assert_eq!(reload.generation(), 3);
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"three\""));
    http.await.unwrap();
}

struct BadCog;

async fn watch_status_until(
    watch: &rappin::hot_reload::WatchGuard,
    predicate: impl Fn(rappin::hot_reload::WatchStatus) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !predicate(watch.status()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovered_scans_clear_only_scan_errors_and_missing_sources_keep_active_routes() {
    let project = Project::new().await;
    project.code("stable", &["ping"], 0, None).await;
    let source = project.0.join("src/main.rs");
    let original = tokio::fs::read(&source).await.unwrap();
    let shared = project.0.join("shared.toml");
    tokio::fs::write(&shared, "shared = true\n").await.unwrap();
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({})),
        support::response(200, json!({})),
    ])
    .await;
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache().set_me(me()).await;
    let reload = HotReload::load(bot.handle(), project.spec().watch_path(&shared))
        .await
        .unwrap();
    let pid = reload.process_id().await;
    let watch = reload.watch(Duration::from_millis(20)).await.unwrap();

    tokio::fs::remove_file(&shared).await.unwrap();
    watch_status_until(&watch, |s| s.scan_error.is_some()).await;
    let error = watch.last_error().unwrap();
    assert!(
        error.contains("resolve watch root") && error.contains("shared.toml"),
        "{error}"
    );
    tokio::fs::write(&shared, "shared = true\n").await.unwrap();
    watch_status_until(&watch, |s| s.scan_error.is_none()).await;
    assert!(watch.last_error().is_none());
    assert_eq!(
        reload.generation(),
        1,
        "unchanged scan recovery must not rebuild"
    );

    // A required Rust source removed before enumeration changes the digest;
    // Cargo must reject it instead of accepting a truncated source inventory.
    tokio::fs::remove_file(&source).await.unwrap();
    watch_status_until(&watch, |s| s.reload_error.is_some()).await;
    assert_eq!(reload.process_id().await, pid);
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"stable\""));
    let failed_candidate = watch.status().reload_error.unwrap();

    tokio::fs::remove_file(&shared).await.unwrap();
    watch_status_until(&watch, |s| s.scan_error.is_some()).await;
    assert_eq!(
        watch.status().reload_error.as_ref(),
        Some(&failed_candidate)
    );
    tokio::fs::write(&shared, "shared = true\n").await.unwrap();
    watch_status_until(&watch, |s| s.scan_error.is_none()).await;
    assert_eq!(watch.last_error().as_ref(), Some(&failed_candidate));
    assert_eq!(reload.generation(), 1);

    let replacement = source.with_extension("new");
    tokio::fs::write(&replacement, original).await.unwrap();
    tokio::fs::rename(replacement, &source).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while reload.generation() < 2 || watch.last_error().is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"stable\""));
    http.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_removes_old_buttons_and_listeners_and_installs_new_ones() {
    let project = Project::new().await;
    project
        .code_with_listeners("one", &["ping"], 0, None, &["member_join"])
        .await;
    let (base, mut requests, http) =
        support::mock((0..4).map(|_| support::response(200, json!({}))).collect()).await;
    let bot = Bot::builder("yb_fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache().set_me(me()).await;
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let button = |prefix: &str| {
        serde_json::from_value(json!({"id":20,"type":"interaction","data":{"id":"interaction","token":"callback","custom_id":format!("{prefix}:hello")}})).unwrap()
    };
    let member = |kind: &str| {
        serde_json::from_value(json!({"id":21,"type":kind,"data":{"user_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil()}})).unwrap()
    };
    bot.process_event(button("one")).await.unwrap();
    bot.process_event(member("member_join")).await.unwrap();
    for _ in 0..2 {
        assert!(requests
            .recv()
            .await
            .unwrap()
            .contains("\"content\":\"one\""));
    }
    project
        .code_with_listeners("two", &["added"], 0, None, &["member_leave"])
        .await;
    reload.reload().await.unwrap();
    bot.process_event(button("one")).await.unwrap();
    bot.process_event(member("member_join")).await.unwrap();
    assert!(
        requests.try_recv().is_err(),
        "removed routes were still dispatched"
    );
    bot.process_event(button("two")).await.unwrap();
    bot.process_event(member("member_leave")).await.unwrap();
    for _ in 0..2 {
        assert!(requests
            .recv()
            .await
            .unwrap()
            .contains("\"content\":\"two\""));
    }
    http.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timed_out_exchange_cannot_be_reused_and_reload_recovers_the_cog() {
    let project = Project::new().await;
    project.code("slow", &["ping"], 1000, None).await;
    let (base, mut requests, http) = support::mock(vec![support::response(200, json!({}))]).await;
    let bot = Bot::builder("yb_fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache().set_me(me()).await;
    let mut spec = project.spec();
    spec.handler_timeout = Duration::from_millis(100);
    let reload = HotReload::load(bot.handle(), spec).await.unwrap();
    let error = bot.process_event(event("ping")).await.unwrap_err();
    assert!(error.to_string().contains("timed out"));
    let error = bot.process_event(event("ping")).await.unwrap_err();
    assert!(error.to_string().contains("no longer usable"));
    project.code("recovered", &["ping"], 0, None).await;
    reload.reload().await.unwrap();
    bot.process_event(event("ping")).await.unwrap();
    assert!(requests
        .recv()
        .await
        .unwrap()
        .contains("\"content\":\"recovered\""));
    http.await.unwrap();
}

#[async_trait::async_trait]
impl Cog for BadCog {
    fn name(&self) -> &str {
        "native"
    }
    async fn on_load(&self, _: &Client) -> Result<()> {
        Err(YappinError::Config("candidate failed".into()))
    }
}
#[tokio::test]
async fn native_cog_load_failure_also_preserves_old_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let bot = Bot::builder("yb_fixture")
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache().set_me(me()).await;
    bot.add_cog(
        CogBuilder::new("native")
            .command("ping", "old", move |_| {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .build(),
    )
    .await
    .unwrap();
    assert!(bot.reload_cog(BadCog).await.is_err());
    bot.process_event(event("ping")).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_rust_worker_sdk_builds_and_proxies_replies_without_receiving_a_token() {
    let reply = json!({"message":{"id":uuid::Uuid::nil(),"channel_id":uuid::Uuid::nil(),"author_id":uuid::Uuid::nil(),"created_at":"2026-10-27T00:00:00Z","content":"reply"}});
    let (base, mut requests, http) = support::mock(
        (0..5)
            .map(|_| support::response(200, reply.clone()))
            .collect(),
    )
    .await;
    let bot = Arc::new(
        Bot::builder("yb_host_credential_fixture")
            .base_url(base)
            .sync_commands(false)
            .log(false)
            .build(),
    );
    bot.cache().set_me(me()).await;
    let project = WorkerProject::example(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        "hot_cog_worker",
    )
    .features(["hot-reload"])
    .offline(true);
    let reload = HotReload::load(bot.handle(), project).await.unwrap();
    bot.process_event(event("ping")).await.unwrap();
    let request = requests.recv().await.unwrap();
    assert!(request
        .to_lowercase()
        .contains("authorization: bot yb_host_credential_fixture"));
    assert!(request.contains("Pong from the reloadable Rust cog!"));
    assert!(bot.handle().help("roll").await.contains("/roll <sides>"));
    let mut roll = event("roll");
    roll.data["args"] = json!("20");
    bot.process_event(roll).await.unwrap();
    assert!(requests.recv().await.unwrap().contains("on d20"));
    let mut invalid = event("roll");
    invalid.data["args"] = json!("101");
    bot.process_event(invalid).await.unwrap();
    assert!(requests.recv().await.unwrap().contains("invalid sides"));
    let question = tokio::spawn({
        let bot = bot.clone();
        async move { bot.process_event(event("question")).await }
    });
    let prompt = tokio::time::timeout(Duration::from_secs(3), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(prompt.contains("What should I call you?"));
    assert_eq!(bot.events().subscriber_count(), 1);
    let mut answer = event("unused");
    answer.data["content"] = json!("Chris");
    bot.events().publish(answer);
    tokio::time::timeout(Duration::from_secs(3), question)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(requests.recv().await.unwrap().contains("Hello, Chris"));
    assert_eq!(bot.events().subscriber_count(), 0);
    assert_eq!(reload.generation(), 1);
    http.await.unwrap();
}

struct ReadyGate {
    ready: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_worker_shares_api_errors_permissions_and_runtime_schemas() {
    let project = Project::new().await;
    let manifest=format!("[package]\nname=\"fixture-worker\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin-rs={{path={:?},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\nserde_json=\"1\"\nuuid=\"1\"\n",env!("CARGO_MANIFEST_DIR"));
    tokio::fs::write(project.0.join("Cargo.toml"), manifest)
        .await
        .unwrap();
    // Reuse the existing worker build cache; no acceptance bot is packaged.
    #[cfg(unix)]
    {
        tokio::fs::create_dir_all(project.0.join("target"))
            .await
            .unwrap();
        // Cargo cannot use a dangling target-dir link on a fresh checkout.
        tokio::fs::create_dir_all(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rappin-hot"))
            .await
            .unwrap();
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rappin-hot"),
            project.0.join("target/rappin-hot"),
        )
        .unwrap();
    }
    let source = r#"
use rappin::*;
use serde_json::{json,Value};
use uuid::Uuid;
async fn common(api:&(impl Api+?Sized),path:&str)->Result<Value> {
    api.api_json::<_,()>("GET".parse().unwrap(),path,None).await
}
#[tokio::main]
async fn main()->Result<()> {
    let schema:CommandSchema=serde_json::from_value(json!({"name":"dynamic","description":"Runtime","arguments":[{"name":"n","kind":{"type":"integer","min":1,"max":10}}]}))?;
    WorkerCog::new("fixture")
        .command("probe","Probe",|ctx|async move {
            let error=common(&ctx,"/denied").await.unwrap_err();
            assert!(error.is_forbidden());assert_eq!(error.to_json()["kind"],"api");assert!(!error.to_string().contains("yb_host_secret"));
            let error=common(&ctx,"/rate").await.unwrap_err();assert!(error.is_rate_limit());assert_eq!(error.retry_after(),Some(std::time::Duration::from_secs(120)));
            let permissions=ctx.evaluate_permissions(Uuid::nil(),Uuid::nil(),Freshness::Refresh).await?;
            assert!(permissions.administrator);assert!(permissions.allows(&Permission::new("future_permission")));
            assert!(ctx.evaluate_permissions(Uuid::nil(),Uuid::nil(),Freshness::PreferCache).await?.administrator);
            let error=ctx.wait_for_event(EventFilter::default(),std::time::Duration::from_millis(10)).await.unwrap_err();
            assert_eq!(error.to_json()["kind"],"event_wait");
            Err(YappinError::Api{status:418,message:"worker failure".into(),retry_after:None})
        })
        .dynamic_command(schema,|ctx,values|async move {ctx.say(format!("dynamic {}",values["n"])).await?;Ok(())})?
        .run().await
}
"#;
    tokio::fs::write(project.0.join("src/main.rs"), source)
        .await
        .unwrap();
    let id = uuid::Uuid::nil();
    let message = json!({"message":{"id":id,"channel_id":id,"author_id":id,"created_at":"2026-10-31T00:00:00Z","content":"reply"}});
    let (base, mut requests, http) = support::mock(vec![
        support::response(403, json!({"error":"yb_host_secret denied"})),
        (
            429,
            vec![("Retry-After", "120")],
            "{\"error\":\"wait\"}".into(),
        ),
        support::response(
            200,
            json!({"server":{"id":id,"name":"test","owner_id":uuid::Uuid::from_u128(9)}}),
        ),
        support::response(
            200,
            json!({"member":{"id":id,"username":"actor","roles":[id]}}),
        ),
        support::response(
            200,
            json!({"roles":[{"id":id,"name":"admin","permissions":{"administrator":true}}]}),
        ),
        support::response(
            200,
            json!({"server":{"id":id,"name":"test","owner_id":uuid::Uuid::from_u128(9)}}),
        ),
        support::response(200, message.clone()),
        support::response(200, message),
    ])
    .await;
    let bot = Bot::builder("yb_host_secret")
        .base_url(base)
        .lookup_cache(rappin::CachePolicy::new(8, Duration::from_secs(60)))
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache().set_me(me()).await;
    let _reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let error = bot.process_event(event("probe")).await.unwrap_err();
    assert_eq!(error.status_code(), Some(418));
    assert_eq!(error.to_json()["kind"], "api");
    let mut valid = event("dynamic");
    valid.data["args"] = json!("3");
    bot.process_event(valid).await.unwrap();
    let mut invalid = event("dynamic");
    invalid.data["args"] = json!("11");
    bot.process_event(invalid).await.unwrap();
    for _ in 0..8 {
        let request = requests.recv().await.unwrap();
        assert!(request
            .to_lowercase()
            .contains("authorization: bot yb_host_secret"));
    }
    http.await.unwrap();
}
#[async_trait::async_trait]
impl rappin::EventHandler for ReadyGate {
    async fn on_ready(&self, _: &Client, _: &rappin::BotProfile) {
        self.ready.notify_one();
        self.resume.notified().await;
    }
    async fn on_message(&self, ctx: &rappin::MessageContext) {
        ctx.bot().unwrap().stop();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_slash_sync_preserves_old_worker_and_main_bot_still_handles_events() {
    let project = Project::new().await;
    project.code("one", &["ping"], 0, None).await;
    let (base, mut requests, http) = support::routed_mock(vec![
        (
            "GET /me".into(),
            vec![support::response(200, support::profile())],
        ),
        (
            "PUT /commands".into(),
            vec![
                support::response(200, json!({"commands":[Command::new("ping","one")]})),
                support::response(503, json!({"error":"command sync unavailable"})),
            ],
        ),
        (
            "GET /events".into(),
            vec![
                support::response(200, json!({"events":[],"last":1})),
                support::response(200, json!({"events":[support::event(2,"ping")],"last":2})),
            ],
        ),
        (
            "POST /channels/00000000-0000-0000-0000-000000000000/messages".into(),
            vec![support::response(200, json!({}))],
        ),
    ])
    .await;
    let ready = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let bot = Bot::builder("yb_fixture")
        .base_url(base)
        .log(false)
        .event_handler(ReadyGate {
            ready: ready.clone(),
            resume: resume.clone(),
        })
        .build();
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let old = reload.process_id().await;
    let runner = tokio::spawn(bot.run());
    ready.notified().await;
    project.code("two", &["ping", "new"], 0, None).await;
    let error = reload.reload().await.unwrap_err();
    let failure = error.hot_reload_failure().unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Publication);
    assert!(failure.candidate_started);
    assert!(failure.publication_attempted);
    assert!(!failure.routes_published);
    assert_eq!(error.status_code(), Some(503));
    assert_eq!(failure.source.unwrap().details["outcome"], "unknown");
    assert_eq!(reload.process_id().await, old);
    assert_eq!(reload.generation(), 1);
    resume.notify_one();
    tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut recorded = vec![];
    while let Ok(request) = requests.try_recv() {
        recorded.push(request);
    }
    assert!(recorded
        .iter()
        .any(|request| request.contains("\"content\":\"one\"")));
    http.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn polling_and_unrelated_native_commands_continue_during_recompilation() {
    let project = Project::new().await;
    project.code("one", &["ping"], 0, None).await;
    let mut responses = vec![
        support::response(200, support::profile()),
        support::response(200, json!({"events":[],"last":1})),
    ];
    for id in 2..1002 {
        responses.push(support::response(
            200,
            json!({"events":[support::event(id,"native")],"last":id}),
        ));
    }
    let (base, mut requests, http) = support::mock(responses).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let count = calls.clone();
    let done = finished.clone();
    let bot = Bot::builder("yb_fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .command("native", "parent handler", move |ctx| {
            let count = count.clone();
            let done = done.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                if done.load(Ordering::SeqCst) {
                    ctx.bot().unwrap().stop();
                }
                Ok(())
            }
        })
        .build();
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let runner = tokio::spawn(bot.run());
    tokio::time::timeout(Duration::from_secs(10), async {
        while calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let before = calls.load(Ordering::SeqCst);
    project.code("two", &["ping", "added"], 0, None).await;
    reload.reload().await.unwrap();
    assert!(
        calls.load(Ordering::SeqCst) > before,
        "main bot stopped dispatching while compiling"
    );
    finished.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut logins = 0;
    while let Ok(request) = requests.try_recv() {
        if request.starts_with("GET /me ") {
            logins += 1;
        }
    }
    assert_eq!(logins, 1, "parent bot reconnected instead of staying alive");
    http.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_inherits_no_credentials_and_unload_prevents_future_reloads() {
    if std::env::var_os("RAPPIN_HOT_ENV_TEST").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worker_inherits_no_credentials_and_unload_prevents_future_reloads",
                "--nocapture",
            ])
            .env("RAPPIN_HOT_ENV_TEST", "1")
            .env("YAPPER_BOT_TOKEN", "yb_environment_fixture")
            .env("CARGO_REGISTRY_TOKEN", "cargo_environment_fixture")
            .env("NODE_OPTIONS", "untrusted-startup-hook")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let project = Project::new().await;
    project.code("one", &["ping"], 0, None).await;
    let bot = Bot::builder("yb_private_fixture")
        .sync_commands(false)
        .log(false)
        .build();
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let watch = reload.watch(Duration::from_millis(30)).await.unwrap();
    reload.unload().await.unwrap();
    assert!(!bot.handle().has_cog("fixture").await);
    assert!(reload.process_id().await.is_none());
    assert!(reload.reload().await.is_err());
    project.code("two", &["ping"], 0, None).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!bot.handle().has_cog("fixture").await);
    drop(watch);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compiler_codes_spans_and_build_script_effects_are_reported_without_source_or_environment()
{
    let project = Project::new().await;
    tokio::fs::write(
        project.0.join("build.rs"),
        r#"fn main() { std::fs::write("build-executed", "yes").unwrap(); }"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        project.0.join("src/main.rs"),
        "fn one(_: u8) {}\nfn main() { absent(); one(); ().missing_method(); }\n",
    )
    .await
    .unwrap();
    let bot = Bot::builder("yb_report_private")
        .sync_commands(false)
        .log(false)
        .build();
    let error = match HotReload::load(bot.handle(), project.spec()).await {
        Ok(_) => panic!("must not compile"),
        Err(error) => error,
    };
    let failure = error.hot_reload_failure().unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Compilation);
    assert!(
        !failure.candidate_started && !failure.routes_published && !failure.publication_attempted
    );
    assert!(!failure.timed_out);
    assert_ne!(failure.exit.as_ref().unwrap().code, Some(0));
    for code in ["E0425", "E0061", "E0599"] {
        let diagnostic = failure
            .diagnostics
            .iter()
            .find(|item| item.code.as_deref() == Some(code))
            .unwrap_or_else(|| panic!("missing {code}: {failure:?}"));
        assert!(diagnostic
            .spans
            .iter()
            .any(|span| span.file.as_deref() == Some("src/main.rs")
                && span.line_start == 2
                && span.is_primary));
        assert!(!diagnostic.message.is_empty());
    }
    assert!(
        project.0.join("build-executed").exists(),
        "compilation is not a side-effect-free sandbox"
    );
    let encoded = error.to_json().to_string();
    assert!(!encoded.contains(&project.0.to_string_lossy().to_string()));
    assert_eq!(error.to_remote().hot_reload_failure(), Some(failure));

    let home_variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = std::env::var(home_variable).unwrap();
    tokio::fs::write(
        project.0.join("src/main.rs"),
        format!("compile_error!(env!({home_variable:?}));\nfn main() {{}}\n"),
    )
    .await
    .unwrap();
    let error = match HotReload::load(bot.handle(), project.spec()).await {
        Ok(_) => panic!("must not compile"),
        Err(error) => error,
    };
    let text = error.to_json().to_string();
    let encoded_home = serde_json::to_string(&home).unwrap();
    assert!(!text.contains(&encoded_home[1..encoded_home.len() - 1]));
    assert!(text.contains("[redacted]"));
    assert!(!text.contains("compile_error!(env!"), "no source excerpts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn candidate_timeout_exit_and_protocol_phases_are_explicit() {
    let project = Project::new().await;
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .build();
    tokio::fs::write(
        project.0.join("src/main.rs"),
        "fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }",
    )
    .await
    .unwrap();
    let mut spec = project.spec();
    spec.build_timeout = Duration::ZERO;
    let error = match HotReload::load(bot.handle(), spec).await {
        Ok(_) => panic!("must time out"),
        Err(error) => error,
    };
    let failure = error.hot_reload_failure().unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Compilation);
    assert!(failure.timed_out);
    assert!(!failure.candidate_started);

    let mut spec = project.spec();
    spec.startup_timeout = Duration::from_millis(50);
    let error = match HotReload::load(bot.handle(), spec).await {
        Ok(_) => panic!("must time out"),
        Err(error) => error,
    };
    let failure = error.hot_reload_failure().unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Startup);
    assert!(failure.timed_out && failure.candidate_started);
    assert!(!failure.routes_published);

    tokio::fs::write(
        project.0.join("src/main.rs"),
        "fn main() { std::process::exit(17); }",
    )
    .await
    .unwrap();
    let error = match HotReload::load(bot.handle(), project.spec()).await {
        Ok(_) => panic!("must exit"),
        Err(error) => error,
    };
    let failure = error.hot_reload_failure().unwrap();
    assert_eq!(failure.phase, rappin::ReloadPhase::Startup);
    assert_eq!(failure.exit.as_ref().unwrap().code, Some(17));
    assert!(!failure.timed_out && failure.candidate_started);

    for (version, commands, phase) in [
        (
            99,
            vec![Command::new("ping", "test")],
            rappin::ReloadPhase::Protocol,
        ),
        (
            3,
            vec![Command::new("INVALID!", "test")],
            rappin::ReloadPhase::Registration,
        ),
    ] {
        let ready = json!({"op":"ready","protocol":version,"manifest":{"name":"fixture","commands":commands,"buttons":[],"listeners":[]}}).to_string();
        let source = format!(r#"fn main() {{ println!("{{}}",{ready:?}); }}"#);
        tokio::fs::write(project.0.join("src/main.rs"), source)
            .await
            .unwrap();
        let failure = match HotReload::load(bot.handle(), project.spec()).await {
            Ok(_) => panic!("invalid candidate"),
            Err(error) => error.hot_reload_failure().unwrap(),
        };
        assert_eq!(failure.phase, phase);
        assert!(failure.candidate_started && !failure.routes_published);
    }

    let missing = HotReload::from_executable(bot.handle(), project.0.join("missing")).await;
    let failure = match missing {
        Ok(_) => panic!("missing executable"),
        Err(error) => error.hot_reload_failure().unwrap(),
    };
    assert_eq!(failure.phase, rappin::ReloadPhase::Startup);
    assert!(!failure.candidate_started);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_candidate_failure_crosses_worker_and_callback_without_implicit_completion() {
    use rappin::{FailureClass, FailureDisposition, HotReloadFailure, ReloadPhase};
    let project = Project::new().await;
    let failure: HotReloadFailure = serde_json::from_value(json!({
        "phase":"compilation","message":"Cargo compilation failed", "diagnostics":[{"code":"E0425","level":"error","message":"cannot find value","spans":[{"file":"src/main.rs","line_start":2,"line_end":2,"column_start":1,"column_end":4,"is_primary":true}],"children":[]}],
        "truncated":false,"exit":{"code":101,"signal":null},"timed_out":false,"candidate_started":false,"publication_attempted":false,"routes_published":false,"source":null
    })).unwrap();
    let original = YappinError::HotReload {
        failure: Box::new(failure.clone()),
    };
    let ready = json!({"op":"ready","protocol":3,"manifest":{"name":"fixture","commands":[Command::new("proposal","test")],"buttons":[],"listeners":[]}}).to_string();
    let done =
        json!({"op":"done","error":original.to_string(),"structured_error":original.to_remote()})
            .to_string();
    let source = format!(
        r#"use std::io::{{BufRead,Write}};fn main() {{ println!("{{}}",{ready:?});std::io::stdout().flush().unwrap();for line in std::io::stdin().lock().lines() {{ if line.unwrap().contains("shutdown") {{break;}} println!("{{}}",{done:?}); std::io::stdout().flush().unwrap(); }} }}"#
    );
    tokio::fs::write(project.0.join("src/main.rs"), source)
        .await
        .unwrap();
    let observed = Arc::new(std::sync::Mutex::new(None));
    let saved = observed.clone();
    let bot = Bot::builder("fixture")
        .sync_commands(false)
        .log(false)
        .on_command_failure(move |_ctx, info| {
            let saved = saved.clone();
            async move {
                assert_eq!(info.class, FailureClass::Unexpected);
                *saved.lock().unwrap() = info.error.hot_reload_failure();
                Ok(FailureDisposition::Stop)
            }
        })
        .build();
    bot.cache().set_me(me()).await;
    let reload = HotReload::load(bot.handle(), project.spec()).await.unwrap();
    let error = bot.process_event(event("proposal")).await.unwrap_err();
    assert_eq!(
        error.hot_reload_failure().unwrap().phase,
        ReloadPhase::Compilation
    );
    assert_eq!(*observed.lock().unwrap(), Some(failure));
    reload.unload().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_role_policy_preserves_independent_checks_and_single_attempt_outcomes() {
    let project = Project::new().await;
    let manifest = format!("[package]\nname=\"role-policy-worker\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin-rs={{path={:?},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\nserde_json=\"1\"\nuuid=\"1\"\n", env!("CARGO_MANIFEST_DIR"));
    tokio::fs::write(project.0.join("Cargo.toml"), manifest)
        .await
        .unwrap();
    #[cfg(unix)]
    {
        tokio::fs::create_dir_all(project.0.join("target"))
            .await
            .unwrap();
        let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rappin-hot");
        tokio::fs::create_dir_all(&cache).await.unwrap();
        std::os::unix::fs::symlink(cache, project.0.join("target/rappin-hot")).unwrap();
    }
    let source = r#"
use rappin::{*,capabilities::*};
use serde_json::json;
use uuid::Uuid;
#[tokio::main]
async fn main()->Result<()> {
    WorkerCog::new("role-fixture").single_attempt_api()
        .command("delete", "Delete", |ctx| async move {
            let target=match ctx.args() { "protected"=>11,"missing"=>12,_=>10 };
            let operation=capability("delete_role").unwrap().validate(&json!({"id":Uuid::nil(),"role":Uuid::from_u128(target)}))?;
            let context=OperationContext::server(Uuid::nil(),Uuid::nil()).test_server_access(true);
            let policy=RoleHierarchyPolicy::new(&ctx).owner_admin_strict();
            let error=policy.execute_deletion(&operation,&context).await.unwrap_err();
            if target==10 {assert_eq!(error.to_json()["retry_safety"],"do_not_replay_workflow");}
            else {assert_eq!(error.to_json()["kind"],"operation_authorization");}
            Err(error)
        }).run().await
}
"#;
    tokio::fs::write(project.0.join("src/main.rs"), source)
        .await
        .unwrap();
    let requester = uuid::Uuid::nil();
    let bot_id = me().id;
    let roles = json!({"roles":[
        {"id":uuid::Uuid::from_u128(10),"name":"target","position":1,"auto":null,"perms":{}},
        {"id":uuid::Uuid::from_u128(11),"name":"protected","position":1,"auto":"everyone","perms":{}},
        {"id":uuid::Uuid::from_u128(12),"name":"missing","position":1,"perms":{}},
        {"id":uuid::Uuid::from_u128(20),"name":"requester","position":2,"auto":null,"perms":{"manage_roles":true}},
        {"id":uuid::Uuid::from_u128(30),"name":"bot","position":3,"auto":null,"perms":{"manage_roles":true}}
    ]});
    // One worker binary covers successful authorization followed by uncertain and
    // server-advised failures. No retry may occur in either process.
    let (base, mut requests, http) = support::routed_mock(vec![
        ("GET /me".into(), vec![support::response(200, support::profile()); 20]),
        (format!("GET /servers/{requester}"), vec![support::response(200,json!({"server":{"id":requester,"name":"test","owner_id":uuid::Uuid::from_u128(99)}})); 20]),
        (format!("GET /servers/{requester}/roles"), vec![support::response(200, roles); 30]),
        (format!("GET /servers/{requester}/members/{requester}"),vec![support::response(200,json!({"member":{"id":requester,"username":"requester","roles":[uuid::Uuid::from_u128(20)]}})); 20]),
        (format!("GET /servers/{requester}/members/{bot_id}"),vec![support::response(200,json!({"member":{"id":bot_id,"username":"bot","roles":[uuid::Uuid::from_u128(30)]}})); 20]),
        (format!("DELETE /servers/{requester}/roles/{}",uuid::Uuid::from_u128(10)),vec![
            support::response(502,json!({"error":"uncertain"})),
            (503,vec![("Retry-After","0.001")],json!({"error":"busy"}).to_string()),
        ]),
    ]).await;
    let callback = Arc::new(AtomicUsize::new(0));
    let c = callback.clone();
    let bot = Bot::builder("fixture")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .on_command_failure(move |_, failure| {
            let c = c.clone();
            async move {
                let index = c.fetch_add(1, Ordering::SeqCst);
                if index >= 2 {
                    let value = failure.error.to_json();
                    assert_eq!(value["kind"], "operation_authorization");
                    assert_eq!(
                        value["decision"],
                        if index == 2 { "denied" } else { "unknown" }
                    );
                    assert_eq!(value["outcome"], "not_applied");
                    return Ok(rappin::FailureDisposition::Handled);
                }
                assert_eq!(
                    failure.error.status,
                    Some(if index == 0 { 502 } else { 503 })
                );
                if index == 0 {
                    assert_eq!(failure.error.to_json()["outcome"], "unknown");
                } else {
                    assert_eq!(failure.error.retry_after, Some(0.001));
                }
                assert_eq!(
                    failure.error.to_json()["retry_safety"],
                    "do_not_replay_workflow"
                );
                // Test application explicitly records the failure as completed. No
                // mutation replay is implied by this callback disposition.
                Ok(rappin::FailureDisposition::Handled)
            }
        })
        .build();
    bot.cache().set_me(me()).await;
    let _reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "role-policy-worker").offline(true),
    )
    .await
    .unwrap();
    for args in ["", "", "protected", "missing"] {
        let mut invocation = event("delete");
        invocation.data["server_id"] = json!(requester);
        invocation.data["args"] = json!(args);
        bot.process_event(invocation).await.unwrap();
    }
    assert_eq!(callback.load(Ordering::SeqCst), 4);
    let seen: Vec<_> = std::iter::from_fn(|| requests.try_recv().ok()).collect();
    assert_eq!(seen.iter().filter(|r| r.starts_with("DELETE ")).count(), 2);
    assert!(seen
        .iter()
        .any(|r| r.starts_with(&format!("GET /servers/{requester}/members/{requester} "))));
    assert!(seen
        .iter()
        .any(|r| r.starts_with(&format!("GET /servers/{requester}/members/{bot_id} "))));
    http.abort();

    // Omitting the protocol opt-in fails before any DELETE is sent.
    // The opt-in check itself needs no network or token.
    let legacy = r#"
use rappin::*;
#[tokio::main]
async fn main()->Result<()> {
    WorkerCog::new("role-fixture").command("no-once","Check opt-in",|ctx|async move {
        let error=ctx.request_once("DELETE".parse().unwrap(),"/roles",None).await.unwrap_err();
        assert!(error.to_string().contains("single_attempt_api"));Ok(())
    }).command("no-ui","Check UI opt-in",|ctx|async move {
        let event:InteractionEvent=serde_json::from_value(serde_json::json!({"id":"fixture","token":"fixture","type":"button"}))?;
        let error=ctx.show_modal(&event,&Modal::new("f","F").input(TextInput::short("n","N"))).await.unwrap_err();
        assert!(error.to_string().contains("ui_api"));Ok(())
    }).run().await
}
"#;
    tokio::fs::write(project.0.join("src/main.rs"), legacy)
        .await
        .unwrap();
    let bot = Bot::builder("fixture")
        .base_url("http://127.0.0.1:1")
        .sync_commands(false)
        .log(false)
        .build();
    bot.cache().set_me(me()).await;
    let _reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "role-policy-worker").offline(true),
    )
    .await
    .unwrap();
    bot.process_event(event("no-once")).await.unwrap();
    bot.process_event(event("no-ui")).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_worker_preserves_values_callbacks_and_failure_causes() {
    let project = Project::new().await;
    let manifest=format!("[package]\nname=\"ui-api-worker\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nrappin-rs={{path={:?},features=[\"hot-reload\"]}}\ntokio={{version=\"1\",features=[\"macros\",\"rt-multi-thread\"]}}\nserde_json=\"1\"\n",env!("CARGO_MANIFEST_DIR"));
    tokio::fs::write(project.0.join("Cargo.toml"), manifest)
        .await
        .unwrap();
    #[cfg(unix)]
    {
        tokio::fs::create_dir_all(project.0.join("target"))
            .await
            .unwrap();
        let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/rappin-hot");
        tokio::fs::create_dir_all(&cache).await.unwrap();
        std::os::unix::fs::symlink(cache, project.0.join("target/rappin-hot")).unwrap();
    }
    let source = r#"
use rappin::*;
#[tokio::main]
async fn main()->Result<()> {
    WorkerCog::new("ui-worker").on_ui_interaction(|ctx|async move {
        let data=ctx.ui_interaction().expect("extended event");
        match data.interaction.kind.as_deref() {
            Some("select")=>{
                assert!(matches!(&data.values,Some(InteractionValues::Selected(v)) if v==&["rust"]));
                ctx.show_modal(&data.interaction,&Modal::new("apply","Apply").input(TextInput::short("name","Name"))).await
            },
            Some("modal_submit")=>{
                assert!(matches!(&data.values,Some(InteractionValues::Submitted(v)) if matches!(v.get("name"),Some(ModalValue::Text(t)) if t=="Rust")));
                Err(YappinError::Api{status:503,message:"structured worker failure".into(),retry_after:Some(2.0)})
            },
            Some("command")=>{
                assert_eq!(data.command.as_deref(),Some("apply"));
                ctx.respond_ui(&data.interaction,InteractionCallbackType::Reply,&UiMessageBody::new("Command received")).await
            },
            _=>panic!("unexpected event"),
        }
    }).run().await
}
"#;
    tokio::fs::write(project.0.join("src/main.rs"), source)
        .await
        .unwrap();
    let (base, mut requests, http) = support::mock(vec![
        support::response(200, json!({"ok":true})),
        support::response(200, json!({"ok":true})),
    ])
    .await;
    let callbacks = Arc::new(AtomicUsize::new(0));
    let c = callbacks.clone();
    let bot = Bot::builder("private_parent_token")
        .base_url(base)
        .sync_commands(false)
        .log(false)
        .interactive_events(true)
        .on_button_failure(move |_, failure| {
            let c = c.clone();
            async move {
                assert_eq!(failure.handler, "ui-worker");
                assert_eq!(failure.error.status, Some(503));
                assert_eq!(failure.error.retry_after, Some(2.0));
                c.fetch_add(1, Ordering::SeqCst);
                Ok(rappin::FailureDisposition::Handled)
            }
        })
        .build();
    bot.cache().set_me(me()).await;
    let _reload = HotReload::load(
        bot.handle(),
        WorkerProject::new(project.0.join("Cargo.toml"), "ui-api-worker").offline(true),
    )
    .await
    .unwrap();
    for kind in ["select", "modal_submit", "command"] {
        let mut data = json!({"id":uuid::Uuid::new_v4(),"token":"interaction_secret","type":kind,"custom_id":"apply","channel_id":uuid::Uuid::nil(),"server_id":uuid::Uuid::nil(),"user_id":uuid::Uuid::nil(),"expires_at":chrono::Utc::now()+chrono::Duration::minutes(15)});
        if kind == "select" {
            data["values"] = json!(["rust"]);
        }
        if kind == "modal_submit" {
            data["values"] = json!({"name":"Rust"});
        }
        if kind == "command" {
            data["command"] = json!("apply");
            data["args"] = json!("");
        }
        bot.process_event(serde_json::from_value(json!({"id":1,"type":if kind=="command" {"command"}else{"interaction"},"data":data})).unwrap()).await.unwrap();
    }
    assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    let first = requests.recv().await.unwrap();
    assert!(first.contains("\"type\":\"modal\""));
    assert!(first
        .to_lowercase()
        .contains("authorization: bot private_parent_token"));
    let second = requests.recv().await.unwrap();
    assert!(second.contains("Command received"));
    assert!(!second.contains("private_interaction_token"));
    http.await.unwrap();
}
