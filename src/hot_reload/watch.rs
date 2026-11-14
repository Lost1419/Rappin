use super::Source;
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
#[error("watch {phase} `{path}`: {source}", path = .path.display())]
struct ScanError {
    phase: &'static str,
    path: PathBuf,
    #[source]
    source: io::Error,
}
type ScanResult<T> = std::result::Result<T, ScanError>;
fn at<T>(phase: &'static str, path: &Path, result: io::Result<T>) -> ScanResult<T> {
    result.map_err(|source| ScanError {
        phase,
        path: path.into(),
        source,
    })
}

// A small filesystem boundary lets tests reproduce enumeration/read races and
// access failures without timing-dependent churn or privileged chmod behavior.
trait FileSystem {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }
    fn metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        fs::symlink_metadata(path)
    }
    fn entries(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        fs::read_dir(path)?
            .filter_map(|entry| match entry {
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                other => Some(other.map(|entry| entry.path())),
            })
            .collect()
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}
struct Disk;
impl FileSystem for Disk {}

pub(super) async fn fingerprint(source: Source) -> Result<Vec<u8>> {
    tokio::task::spawn_blocking(move || scan(&source, &Disk))
        .await
        .map_err(|_| Error::Config("source watcher task failed".into()))?
        .map_err(|error| Error::Io(io::Error::new(error.source.kind(), error)))
}

fn scan(source: &Source, disk: &impl FileSystem) -> ScanResult<Vec<u8>> {
    // An editor can briefly remove a source during replacement. Retry the whole
    // snapshot, not just a file read, so a disappearing directory cannot produce
    // a mixed snapshot. Persistent missing required inputs still report errors.
    for attempt in 0..3 {
        match scan_once(source, disk) {
            Err(error) if error.source.kind() == io::ErrorKind::NotFound && attempt < 2 => {
                std::thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
    unreachable!()
}

fn scan_once(source: &Source, disk: &impl FileSystem) -> ScanResult<Vec<u8>> {
    let mut hash = Sha256::new();
    match source {
        Source::Executable(path) => {
            hash.update(at("read executable", path, disk.read(path))?);
        }
        Source::Project(project) => {
            let manifest = at(
                "resolve manifest",
                &project.manifest,
                disk.canonicalize(&project.manifest),
            )?;
            let root = manifest.parent().unwrap_or(Path::new("."));
            let mut files = vec![manifest.clone()];
            collect(root, true, disk, &mut files)?;
            for path in &project.watch_paths {
                // Explicit roots are required, even if normally ignored.
                let path = at("resolve watch root", path, disk.canonicalize(path))?;
                collect(&path, true, disk, &mut files)?;
            }
            files.sort();
            files.dedup();
            for path in files {
                let bytes = at("read source", &path, disk.read(&path))?;
                let name = path.to_string_lossy();
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name.as_bytes());
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
    }
    Ok(hash.finalize().to_vec())
}

fn source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("rs" | "toml")
    ) || path.file_name().is_some_and(|n| n == "Cargo.lock")
}
fn collect(
    path: &Path,
    required: bool,
    disk: &impl FileSystem,
    files: &mut Vec<PathBuf>,
) -> ScanResult<()> {
    let metadata = match disk.metadata(path) {
        Ok(metadata) => metadata,
        // A disappearing non-source entry is irrelevant. A removed source that
        // is absent from enumeration changes the digest and is checked by Cargo.
        Err(error)
            if error.kind() == io::ErrorKind::NotFound && !required && !source_file(path) =>
        {
            return Ok(())
        }
        result => at("inspect entry", path, result)?,
    };
    if metadata.is_symlink() && !required {
        return Ok(());
    }
    if metadata.is_file() || (metadata.is_symlink() && required) {
        if source_file(path) {
            files.push(path.into());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    let entries = match disk.entries(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && !required => return Ok(()),
        result => at("enumerate directory", path, result)?,
    };
    for entry in entries {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || matches!(&*name, "target" | "node_modules") {
            continue;
        }
        collect(&entry, false, disk, files)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
