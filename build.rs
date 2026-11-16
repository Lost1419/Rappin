use std::{env, path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_CLI").is_none() {
        return;
    }

    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let version = match checkout_commit() {
        Some(commit) => format!("{version} ({})", &commit[..7]),
        None => version,
    };
    println!("cargo:rustc-env=RAPPIN_CLI_VERSION={version}");
}

fn checkout_commit() -> Option<String> {
    // A packaged crate can be extracted inside an unrelated Git checkout.
    if !git(&["rev-parse", "--show-prefix"])?.is_empty() {
        return None;
    }

    watch_git_path("HEAD");
    watch_git_path("packed-refs");
    if let Some(reference) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
        watch_git_path(&reference);
    }

    let commit = git(&["rev-parse", "--verify", "HEAD"])?;
    (commit.len() >= 7 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(commit)
}

fn watch_git_path(name: &str) {
    if let Some(path) = git(&["rev-parse", "--git-path", name]) {
        let path = Path::new(&path);
        // Watch a missing ref as well, in case Git creates it on the next commit.
        if name != "packed-refs" || path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
