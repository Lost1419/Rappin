use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("rappin-scan-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        Self(fs::canonicalize(root).unwrap())
    }
    fn source(&self) -> Source {
        Source::Project(super::super::WorkerProject::new(
            self.0.join("Cargo.toml"),
            "fixture",
        ))
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn runtime_churn_is_irrelevant_but_sources_and_explicit_roots_remain_watched() {
    let project = Project::new();
    let initial = scan(&project.source(), &Disk).unwrap();
    for name in ["data", "state"] {
        fs::create_dir(project.0.join(name)).unwrap();
        fs::write(project.0.join(name).join("db.sqlite-journal"), "not source").unwrap();
    }
    struct NeverReadRuntime;
    impl FileSystem for NeverReadRuntime {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            assert!(
                source_file(path),
                "runtime file must not be read: {}",
                path.display()
            );
            Disk.read(path)
        }
    }
    assert_eq!(initial, scan(&project.source(), &NeverReadRuntime).unwrap());
    fs::write(project.0.join("data/plugin.rs"), "pub fn plugin() {}\n").unwrap();
    assert_ne!(initial, scan(&project.source(), &Disk).unwrap());
    let Source::Project(mut explicit) = project.source() else {
        unreachable!()
    };
    explicit = explicit.watch_path(project.0.join("data"));
    assert_ne!(
        initial,
        scan(&Source::Project(explicit.clone()), &Disk).unwrap()
    );
    fs::remove_dir_all(project.0.join("data")).unwrap();
    let error = scan(&Source::Project(explicit), &Disk).unwrap_err();
    assert_eq!(error.phase, "resolve watch root");
    assert_eq!(error.source.kind(), io::ErrorKind::NotFound);
}

#[test]
fn disappearing_irrelevant_file_and_directory_after_enumeration_are_harmless() {
    let project = Project::new();
    let expected = scan(&project.source(), &Disk).unwrap();
    let journal = project.0.join("db.sqlite-journal");
    let transient = project.0.join("sessions");
    fs::write(&journal, "journal").unwrap();
    fs::create_dir(&transient).unwrap();
    struct Churn {
        journal: PathBuf,
        transient: PathBuf,
    }
    impl FileSystem for Churn {
        fn metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
            if path == self.journal {
                fs::remove_file(path)?;
            }
            Disk.metadata(path)
        }
        fn entries(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            if path == self.transient {
                fs::remove_dir(path)?;
            }
            Disk.entries(path)
        }
    }
    assert_eq!(
        expected,
        scan(&project.source(), &Churn { journal, transient }).unwrap()
    );
}

#[test]
fn atomic_source_replacement_retries_a_complete_snapshot() {
    let project = Project::new();
    let path = project.0.join("src/main.rs");
    let before = scan(&project.source(), &Disk).unwrap();
    struct Replace {
        path: PathBuf,
        replaced: AtomicBool,
    }
    impl FileSystem for Replace {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            if path == self.path && !self.replaced.swap(true, Ordering::SeqCst) {
                let replacement = path.with_extension("new");
                fs::write(&replacement, "fn main() { println!(\"new\"); }\n")?;
                fs::rename(replacement, path)?;
                return Err(io::ErrorKind::NotFound.into());
            }
            Disk.read(path)
        }
    }
    let after = scan(
        &project.source(),
        &Replace {
            path,
            replaced: AtomicBool::new(false),
        },
    )
    .unwrap();
    assert_ne!(before, after);
    assert_eq!(after, scan(&project.source(), &Disk).unwrap());
}

#[test]
fn missing_and_inaccessible_inputs_report_phase_and_path() {
    let project = Project::new();
    let source = project.0.join("src/main.rs");
    struct Inaccessible(PathBuf);
    impl FileSystem for Inaccessible {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            if path == self.0 {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            Disk.read(path)
        }
    }
    let error = scan(&project.source(), &Inaccessible(source.clone())).unwrap_err();
    assert_eq!(error.phase, "read source");
    assert_eq!(error.path, source);
    assert_eq!(error.source.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("main.rs"));
    struct MissingRead {
        path: PathBuf,
        attempts: AtomicUsize,
    }
    impl FileSystem for MissingRead {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            if path == self.path {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                return Err(io::ErrorKind::NotFound.into());
            }
            Disk.read(path)
        }
    }
    let missing = MissingRead {
        path: source.clone(),
        attempts: AtomicUsize::new(0),
    };
    let error = scan(&project.source(), &missing).unwrap_err();
    assert_eq!(error.phase, "read source");
    assert_eq!(error.path, source);
    assert_eq!(missing.attempts.load(Ordering::SeqCst), 3);
    let before = scan(&project.source(), &Disk).unwrap();
    fs::remove_file(&source).unwrap();
    assert_ne!(
        before,
        scan(&project.source(), &Disk).unwrap(),
        "source deletions must trigger Cargo validation"
    );
    fs::remove_file(project.0.join("Cargo.toml")).unwrap();
    let error = scan(&project.source(), &Disk).unwrap_err();
    assert_eq!(error.phase, "resolve manifest");
    assert!(error.to_string().contains("Cargo.toml"));
    let executable = Source::Executable(project.0.join("missing-worker"));
    assert_eq!(
        scan(&executable, &Disk).unwrap_err().phase,
        "read executable"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_source_is_not_silently_ignored() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    let source = project.0.join("src/main.rs");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o0)).unwrap();
    // Root can bypass mode bits. The injected access-error test covers that case.
    if fs::read(&source).is_ok() {
        return;
    }
    let error = scan(&project.source(), &Disk).unwrap_err();
    assert_eq!(error.path, source);
    assert_eq!(error.phase, "read source");
    assert_eq!(error.source.kind(), io::ErrorKind::PermissionDenied);
}
