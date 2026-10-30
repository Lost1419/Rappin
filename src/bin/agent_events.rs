//! Receipts supplement the SDK's compatible numeric cursor files.
use super::*;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, path::Path};

#[derive(Serialize, Deserialize)]
struct Delivery {
    binding: String,
    batch_id: String,
    after: u64,
    batch: rappin::EventsResponse,
}
fn sidecar(cursor: &Path, suffix: &str) -> PathBuf {
    let mut name = cursor.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
fn lock(cursor: &Path) -> Result<std::fs::File> {
    let path = sidecar(cursor, ".lock");
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            YappinError::Config("event cursor is in use by another consumer".into())
        } else {
            error.into()
        }
    })?;
    Ok(file)
}
fn binding(ctx: &Context) -> String {
    let mut digest = Sha256::new();
    digest.update(ctx.base.as_bytes());
    digest.update([0]);
    digest.update(ctx.token.as_deref().unwrap_or("").as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn load(cursor: &Path, ctx: &Context) -> Result<Option<Delivery>> {
    let bytes = match std::fs::read(sidecar(cursor, ".pending.json")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let delivery: Delivery = serde_json::from_slice(&bytes)
        .map_err(|_| YappinError::Config("invalid pending event receipt".into()))?;
    if delivery.binding != binding(ctx) {
        return Err(YappinError::Config(
            "event receipt belongs to a different bot or API base; choose a separate cursor file"
                .into(),
        ));
    }
    Ok(Some(delivery))
}
fn save(cursor: &Path, delivery: &Delivery) -> Result<()> {
    let target = sidecar(cursor, ".pending.json");
    let tmp = sidecar(cursor, &format!(".{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result: Result<()> = (|| {
        use std::io::Write;
        let mut file = options.open(&tmp)?;
        file.write_all(&serde_json::to_vec(delivery)?)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}
fn result(delivery: &Delivery) -> Value {
    json!({"events":delivery.batch.events,"next_cursor":delivery.batch.last,"batch_id":delivery.batch_id,"acknowledged":false})
}
pub async fn poll(
    ctx: &Context,
    after: Option<u64>,
    cursor: Option<&Path>,
    wait: u32,
    max_age: Option<u32>,
) -> Result<Value> {
    let _gate = ctx.events.lock().await;
    let _lock = cursor.map(lock).transpose()?;
    let saved = match cursor {
        Some(path) => EventCursor::new(path).load().await?,
        None => None,
    };
    if let Some(path) = cursor {
        if let Some(mut delivery) = load(path, ctx)? {
            if !saved.is_some_and(|id| id >= delivery.batch.last) {
                // Recover a crash between advancing the checkpoint and updating
                // the receipt. Retain its bot binding even after full acknowledgement.
                if let Some(saved) = saved.filter(|id| *id > delivery.after) {
                    delivery.after = saved;
                    delivery.batch.events.retain(|event| event.id > saved);
                    delivery.batch_id = Uuid::new_v4().to_string();
                    save(path, &delivery)?;
                }
                if after.is_some_and(|id| id != delivery.after) {
                    return Err(YappinError::validation(
                        "after",
                        "acknowledge the pending batch before choosing another cursor",
                    ));
                }
                return Ok(result(&delivery));
            }
        }
    }
    let start = after.or(saved).unwrap_or(0);
    let batch = ctx
        .client()?
        .get_events(Some(start), Some(wait), max_age)
        .await?;
    let delivery = Delivery {
        binding: binding(ctx),
        batch_id: Uuid::new_v4().to_string(),
        after: start,
        batch,
    };
    if let Some(path) = cursor {
        save(path, &delivery)?;
    }
    Ok(result(&delivery))
}
pub async fn acknowledge(
    ctx: &Context,
    id: u64,
    cursor: &Path,
    batch_id: Option<&str>,
    force: bool,
) -> Result<Value> {
    let _gate = ctx.events.lock().await;
    let _lock = lock(cursor)?;
    let checkpoint = EventCursor::new(cursor);
    let previous = checkpoint.load().await?;
    if previous.is_some_and(|old| old > id) {
        return Err(YappinError::validation(
            "id",
            "cannot move a cursor backwards",
        ));
    }
    let pending = if force {
        None
    } else {
        Some(load(cursor, ctx)?.ok_or_else(|| {
            YappinError::validation(
                "id",
                "no pending delivery; poll with this cursor file first",
            )
        })?)
    };
    if let Some(delivery) = &pending {
        if previous == Some(id) && delivery.after == id {
            return Ok(
                json!({"last":id,"dry_run":ctx.dry,"forced":false,"already_committed":true}),
            );
        }
        if batch_id.is_some_and(|id| id != delivery.batch_id) {
            return Err(YappinError::validation("batch_id", "stale event receipt"));
        }
        if id < delivery.after
            || (id != delivery.batch.last
                && !delivery.batch.events.iter().any(|event| event.id == id))
        {
            return Err(YappinError::validation(
                "id",
                "ID was not delivered in the pending batch",
            ));
        }
    }
    if !ctx.dry {
        if previous != Some(id) {
            checkpoint.save(id).await?;
        }
        if let Some(mut delivery) = pending {
            delivery.after = id;
            delivery.batch.events.retain(|event| event.id > id);
            delivery.batch_id = Uuid::new_v4().to_string();
            save(cursor, &delivery)?;
        } else {
            match std::fs::remove_file(sidecar(cursor, ".pending.json")) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(json!({"last":id,"dry_run":ctx.dry,"forced":force}))
}
