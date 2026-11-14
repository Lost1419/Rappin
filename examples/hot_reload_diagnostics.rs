//! Offline candidate validation: cargo run --features hot-reload --example
//! hot_reload_diagnostics -- path/to/Cargo.toml worker_binary
//! Runs trusted Cargo/build scripts/macros and, on success, the worker executable.
use rappin::{
    hot_reload::{HotReload, WorkerProject},
    Bot, Error, Result,
};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let manifest = args
        .next()
        .ok_or_else(|| Error::Config("provide a manifest path and binary name".into()))?;
    let binary = args
        .next()
        .ok_or_else(|| Error::Config("provide a binary name".into()))?;
    // No authentication or network requests: sync is disabled and run() is not called.
    let bot = Bot::builder("unused_offline_credential")
        .sync_commands(false)
        .log(false)
        .build()
        .expect("valid bot configuration");
    match HotReload::load(
        bot.handle(),
        WorkerProject::new(manifest, binary).offline(true),
    )
    .await
    {
        Ok(controller) => {
            println!(
                "{}",
                serde_json::json!({"installed":true,"generation":controller.generation()})
            );
            controller.unload().await?;
        }
        Err(error) => {
            if let Some(report) = error.hot_reload_failure() {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                return Err(error);
            }
            std::process::exit(1);
        }
    }
    Ok(())
}
