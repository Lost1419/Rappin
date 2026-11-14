//! Offline, operator-only project generation. Never exposed as an MCP operation.
use clap::{Args, ValueEnum};
use rappin::{Error, Result};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum Template {
    Basic,
    Gateway,
    Worker,
}
#[derive(Args)]
pub(super) struct Options {
    /// New directory; its parent must exist. Existing directories are refused.
    pub destination: PathBuf,
    #[arg(long, value_enum, default_value = "gateway")]
    pub template: Template,
    /// Cargo package name; defaults to the destination's final component.
    #[arg(long)]
    pub name: Option<String>,
    /// Initial prefix; applications can replace it with their own resolver.
    #[arg(long, default_value = "!")]
    pub prefix: String,
}
fn invalid(message: &str) -> Error {
    Error::Config(message.into())
}
fn files(options: &Options) -> Result<BTreeMap<&'static str, String>> {
    let name = options
        .name
        .as_deref()
        .or_else(|| options.destination.file_name()?.to_str())
        .ok_or_else(|| invalid("provide an ASCII Cargo package name with --name"))?;
    if name.len() > 64
        || name.is_empty()
        || !name.as_bytes()[0].is_ascii_alphabetic()
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        || [
            "test",
            "build",
            "deps",
            "examples",
            "incremental",
            "self",
            "super",
            "crate",
            "std",
            "core",
            "alloc",
            "proc-macro",
        ]
        .contains(&name)
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&name.to_ascii_uppercase().as_str())
    {
        return Err(invalid("package name must start with an ASCII letter and contain at most 64 letters, digits, underscores or hyphens; reserved names are refused"));
    }
    if options.prefix.is_empty()
        || options.prefix.len() > 64
        || options.prefix.chars().any(char::is_control)
    {
        return Err(invalid(
            "prefix must contain 1..64 UTF-8 bytes without control characters",
        ));
    }
    let features = match options.template {
        Template::Basic => r#", features = ["derive"]"#,
        Template::Gateway => r#", features = ["gateway", "derive"]"#,
        Template::Worker => r#", features = ["gateway", "hot-reload", "derive"]"#,
    };
    let source = if env!("CARGO_PKG_VERSION").contains("-dev.") {
        ", git = \"https://github.com/Lost1419/Rappin\", branch = \"dev\""
    } else {
        ""
    };
    let manifest = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
rust-version = "{rust_version}"
default-run = "{name}"

[dependencies]
rappin = {{ package = "rappin-rs", version = "{version}"{source}{features} }}
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}
tracing-subscriber = "0.3"
"#,
        version = env!("CARGO_PKG_VERSION"),
        rust_version = env!("CARGO_PKG_RUST_VERSION"),
    );
    let mut main = String::from("use rappin::prelude::*;\nuse rappin::bot::{PersistenceConfig, TransportConfig};\n\n#[tokio::main]\nasync fn main() -> Result<()> {\n    tracing_subscriber::fmt::init();\n    let bot = Bot::from_env()?\n");
    main.push_str(&format!("        .prefixes([{:?}])\n", options.prefix));
    main.push_str("        .configure(|settings| settings.persistence = PersistenceConfig::file(\"./state/events.json\"))\n");
    if !matches!(options.template, Template::Basic) {
        main.push_str("        .transport(TransportConfig::gateway())\n");
    } else {
        main.push_str("        .transport(TransportConfig::polling())\n");
    }
    main.push_str("        .help_command(\"help\")\n");
    if !matches!(options.template, Template::Worker) {
        main.push_str("        .command(\"ping\", \"Check connectivity\", |ctx| async move {\n            ctx.reply(\"pong\").await?;\n            Ok(())\n        })\n");
    }
    main.push_str("        .build()?;\n");
    if matches!(options.template, Template::Worker) {
        main.push_str("    let reload = rappin::hot_reload::HotReload::load(\n        bot.handle(),\n        rappin::hot_reload::WorkerProject::example(\"Cargo.toml\", \"worker\"),\n    ).await?;\n    let _watch = reload.watch(std::time::Duration::from_millis(500)).await?;\n");
    }
    main.push_str("    bot.run().await\n}\n");
    let readme = format!(
        "# {name}\n\nSet `YAPPER_BOT_TOKEN` in this bot process environment, then run `cargo run`.\nSend `{}ping` to receive `pong`, or `{}help` for registered command help.\n\nEdit `src/main.rs` to add commands. This project enables the optional argument\nderive so typed commands can generate their parsing and help together.\n\n## Configuration\n\n- Startup logs report connection and command synchronization.\n- `state/events.json` stores restart checkpoints and is ignored by Git.\n- Set static prefixes with `prefixes`, or supply your own async/database-backed resolver with `prefixes_with`.\n- Register application-owned state with `state(Arc<T>)`.\n- Your application selects databases, cache backends and durable receipts.\n\nExpected input or permission rejections finish after a successful reply.\nUnexpected errors, failed replies and uncertain effects retain the checkpoint\nbarrier. Reconcile unfinished work before replaying it.\n\n{}\nDo not commit tokens. Generation is offline. Building can contact the registry,\nand running this bot contacts Yapin.\n",
        options.prefix, options.prefix,
        if matches!(options.template, Template::Basic) {
            "This starter uses long polling."
        } else if matches!(options.template, Template::Worker) {
            "This starter prefers WebSocket events with polling fallback. Edit `examples/worker.rs` to rebuild and replace the worker. Workers and their build scripts/procedural macros execute trusted code. Worker stdout is reserved for IPC. Each worker generation initializes its own process-local state."
        } else {
            "This starter prefers WebSocket events with polling fallback. HTTP API requests use their normal transport policy."
        }
    );
    let mut result = BTreeMap::from([
        ("Cargo.toml", manifest),
        ("src/main.rs", main),
        (".gitignore", "/target/\n/state/\n/.env\n".into()),
        ("README.md", readme),
    ]);
    if matches!(options.template, Template::Worker) {
        result.insert("examples/worker.rs", "use rappin::{hot_reload::WorkerCog, prelude::*};\n\n#[tokio::main]\nasync fn main() -> Result<()> {\n    WorkerCog::new(\"commands\")\n        .command_spec(CommandSpec::<()>::new(\"ping\", \"Check connectivity\"), |ctx, ()| async move {\n            ctx.reply(\"pong\").await\n        })\n        .run().await\n}\n".into());
    }
    Ok(result)
}
pub(super) fn generate(options: &Options, dry_run: bool) -> Result<Value> {
    let planned = files(options)?;
    let destination = &options.destination;
    if destination.file_name().is_none() {
        return Err(invalid("destination must be a new named directory"));
    }
    match destination.symlink_metadata() {
        Ok(_) => return Err(invalid("destination must be a new named directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    if !parent.is_dir() {
        return Err(invalid("destination parent must already exist"));
    }
    if !dry_run {
        fs::create_dir(destination)?; // Exclusive reservation: never merge into an existing tree.
        let mut created_files = Vec::new();
        let mut created_dirs = Vec::new();
        let write = (|| -> Result<()> {
            for (name, contents) in &planned {
                let path = destination.join(name);
                let dir = path.parent().expect("fixed template path");
                if dir != destination && !created_dirs.iter().any(|p| p == dir) {
                    fs::create_dir(dir)?;
                    created_dirs.push(dir.to_path_buf());
                }
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                created_files.push(path);
                output.write_all(contents.as_bytes())?;
            }
            Ok(())
        })();
        if let Err(error) = write {
            for file in created_files.iter().rev() {
                let _ = fs::remove_file(file);
            }
            for dir in created_dirs.iter().rev() {
                let _ = fs::remove_dir(dir);
            }
            let _ = fs::remove_dir(destination); // Never recursively delete unrelated files.
            return Err(error);
        }
    }
    Ok(
        json!({"template":format!("{:?}",options.template).to_ascii_lowercase(),"destination":destination,"dry_run":dry_run,"files":planned}),
    )
}
