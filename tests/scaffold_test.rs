#![cfg(feature = "cli")]
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("rappin-init-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("nonexistent.toml"), "not valid = [ TOML").unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_rappin"))
            .env_remove("YAPPER_BOT_TOKEN")
            .env_remove("YAPPER_API_BASE")
            .args([
                "--config",
                self.0.join("nonexistent.toml").to_str().unwrap(),
                "--base-url",
                "http://127.0.0.1:1",
            ])
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn value(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn init_is_offline_and_never_merges_or_overwrites() {
    let scratch = Scratch::new();
    let destination = scratch.0.join("my-bot");
    let path = destination.to_str().unwrap();
    let preview = scratch.run(&["--dry-run", "init", path, "--template", "worker"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stdout)
    );
    assert!(!destination.exists());
    assert!(value(&preview)["data"]["files"]["examples/worker.rs"].is_string());
    let output = scratch.run(&["init", path]);
    assert!(output.status.success());
    let before = std::fs::read(destination.join("src/main.rs")).unwrap();
    assert!(!scratch.run(&["init", path]).status.success());
    assert!(!scratch.run(&["--dry-run", "init", path]).status.success());
    assert_eq!(
        before,
        std::fs::read(destination.join("src/main.rs")).unwrap()
    );
    assert!(!scratch
        .run(&["init", scratch.0.join("invalid name").to_str().unwrap()])
        .status
        .success());
    assert!(!scratch
        .run(&[
            "init",
            scratch.0.join("new").to_str().unwrap(),
            "--prefix",
            "\n"
        ])
        .status
        .success());
    assert!(!scratch.0.join("new").exists());
}
#[cfg(unix)]
#[test]
fn init_refuses_a_destination_symlink() {
    let scratch = Scratch::new();
    let target = scratch.0.join("target");
    std::fs::create_dir(&target).unwrap();
    let link = scratch.0.join("bot");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(!scratch
        .run(&["init", link.to_str().unwrap()])
        .status
        .success());
    assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
}
#[test]
fn every_generated_template_compiles_offline_with_its_declared_features() {
    let scratch = Scratch::new();
    for template in ["basic", "gateway", "worker"] {
        let root = scratch.0.join(template);
        let output = scratch.run(&[
            "init",
            root.to_str().unwrap(),
            "--template",
            template,
            "--prefix",
            "\"🦀\\",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let manifest = root.join("Cargo.toml");
        let mut text = std::fs::read_to_string(&manifest).unwrap();
        // Compile generated code against the exact checkout under test, without registry publication.
        text.push_str(&format!(
            "\n[patch.crates-io]\nrappin-rs = {{ path = {:?} }}\n",
            env!("CARGO_MANIFEST_DIR")
        ));
        std::fs::write(&manifest, text).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock"),
            root.join("Cargo.lock"),
        )
        .unwrap();
        let status = Command::new(env!("CARGO"))
            .args(["check", "--offline", "--all-targets", "--manifest-path"])
            .arg(&manifest)
            .arg("--target-dir")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/scaffold-check"))
            .status()
            .unwrap();
        assert!(status.success(), "{template} template failed compilation");
    }
}
