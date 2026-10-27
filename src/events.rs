//! Durable at-least-once polling. A single consumer owns each cursor file.
use crate::{Client, Event, EventsResponse, Result, YappinError};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EventCursor {
    path: PathBuf,
}
impl EventCursor {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub async fn load(&self) -> Result<Option<u64>> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    /// Atomic replace: a crash cannot leave a partially written cursor.
    pub async fn save(&self, id: u64) -> Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        tokio::fs::create_dir_all(parent).await?;
        let temporary = parent.join(format!(".rappin-cursor-{}.tmp", uuid::Uuid::new_v4()));
        let result: Result<()> = async {
            tokio::fs::write(&temporary, id.to_string()).await?;
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(&temporary)
                .await?
                .sync_all()
                .await?;
            tokio::fs::rename(&temporary, &self.path).await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result
    }
}
/// Poll, handle the returned batch, then call acknowledge. Unacknowledged batches are redelivered.
/// Events use global bot IDs, so reuse the same cursor when switching between gateway and polling.
pub struct EventConsumer {
    client: Client,
    cursor: Option<EventCursor>,
    last: Option<u64>,
    pending: Option<EventsResponse>,
    max_age: Option<u32>,
}
impl EventConsumer {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            cursor: None,
            last: None,
            pending: None,
            max_age: None,
        }
    }
    pub fn cursor_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.cursor = Some(EventCursor::new(path));
        self
    }
    pub fn after(mut self, id: u64) -> Self {
        self.last = Some(id);
        self
    }
    pub fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }
    pub fn last(&self) -> Option<u64> {
        self.last
    }
    pub async fn poll(&mut self, wait: u32) -> Result<EventsResponse> {
        if let Some(batch) = &self.pending {
            return Ok(batch.clone());
        }
        if self.last.is_none() {
            self.last = match &self.cursor {
                Some(c) => c.load().await?,
                None => None,
            };
            if self.last.is_none() {
                let last = self.client.get_events(Some(0), Some(0), None).await?.last;
                if let Some(c) = &self.cursor {
                    c.save(last).await?;
                }
                self.last = Some(last);
            }
        }
        let batch = self
            .client
            .get_events(self.last, Some(wait), self.max_age)
            .await?;
        self.pending = Some(batch.clone());
        Ok(batch)
    }
    /// Commits only IDs from the delivered batch (including its last watermark).
    pub async fn acknowledge(&mut self, id: u64) -> Result<()> {
        let batch = self
            .pending
            .as_ref()
            .ok_or_else(|| YappinError::Config("no delivered batch to acknowledge".into()))?;
        if id < self.last.unwrap_or(0)
            || (id != batch.last && !batch.events.iter().any(|e| e.id == id))
        {
            return Err(YappinError::validation(
                "cursor",
                "ID was not delivered by this consumer",
            ));
        }
        if let Some(c) = &self.cursor {
            c.save(id).await?;
        }
        self.last = Some(id);
        if id == batch.last {
            self.pending = None;
        } else if let Some(b) = &mut self.pending {
            b.events.retain(|e| e.id > id);
        }
        Ok(())
    }
}

impl Client {
    /// bounded page collection with a real timestamp continuation cursor.
    pub async fn message_history(
        &self,
        channel: uuid::Uuid,
        before: Option<chrono::DateTime<chrono::Utc>>,
        max_pages: usize,
    ) -> Result<crate::MessagesResponse> {
        if !(1..=100).contains(&max_pages) {
            return Err(YappinError::validation(
                "max_pages",
                "must be between 1 and 100",
            ));
        }
        let mut cursor = before;
        let mut out = crate::MessagesResponse {
            messages: vec![],
            authors: vec![],
        };
        let mut seen = std::collections::HashSet::new();
        let mut users = std::collections::HashSet::new();
        for _ in 0..max_pages {
            let batch = self.get_messages(channel, Some(100), cursor, None).await?;
            let next = batch.messages.iter().map(|m| m.created_at).min();
            let size = batch.messages.len();
            for m in batch.messages {
                if seen.insert(m.id) {
                    out.messages.push(m);
                }
            }
            for u in batch.authors {
                if users.insert(u.id) {
                    out.authors.push(u);
                }
            }
            if size < 100 || next.is_none() || next == cursor {
                break;
            }
            cursor = next;
        }
        out.messages.sort_by_key(|m| m.created_at);
        Ok(out)
    }
    pub async fn member_pages(
        &self,
        server: uuid::Uuid,
        after: Option<uuid::Uuid>,
        max_pages: usize,
    ) -> Result<Vec<crate::Member>> {
        if !(1..=100).contains(&max_pages) {
            return Err(YappinError::validation(
                "max_pages",
                "must be between 1 and 100",
            ));
        }
        let mut cursor = after;
        let mut out = vec![];
        for _ in 0..max_pages {
            let batch = self.get_members(server, Some(500), cursor).await?;
            let count = batch.len();
            let next = batch.last().map(|m| m.user.id);
            out.extend(batch);
            if count < 500 || next == cursor {
                break;
            }
            cursor = next;
        }
        Ok(out)
    }
    pub async fn resolve_channel(
        &self,
        server: Option<uuid::Uuid>,
        name: &str,
    ) -> Result<crate::Channel> {
        let channels = match server {
            Some(id) => self.get_server_channels(id, None).await?.channels,
            None => self.get_channels(None).await?,
        };
        unique(
            channels
                .into_iter()
                .filter(|c| c.id.to_string() == name || c.name.eq_ignore_ascii_case(name))
                .collect(),
            "channel",
        )
    }
    /// Search a bounded number of member pages; ambiguity is never silently resolved.
    pub async fn resolve_member(
        &self,
        server: uuid::Uuid,
        name: &str,
        max_pages: usize,
    ) -> Result<crate::Member> {
        if let Ok(id) = name.parse() {
            return Ok(self.get_member(server, id).await?.member);
        }
        unique(
            self.member_pages(server, None, max_pages)
                .await?
                .into_iter()
                .filter(|m| {
                    m.user.username.eq_ignore_ascii_case(name)
                        || m.effective_name().eq_ignore_ascii_case(name)
                })
                .collect(),
            "member",
        )
    }
    pub async fn send_long_message(
        &self,
        channel: uuid::Uuid,
        content: &str,
    ) -> Result<Vec<crate::Message>> {
        let mut sent = vec![];
        for part in crate::split_message(content) {
            sent.push(
                self.send_message(channel, &crate::CreateMessage::new(part))
                    .await?,
            );
        }
        Ok(sent)
    }
}
fn unique<T>(mut matches: Vec<T>, resource: &str) -> Result<T> {
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(YappinError::validation(
            resource,
            "no match in the requested search scope",
        )),
        _ => Err(YappinError::validation(
            resource,
            "ambiguous name; use a stable ID or narrower scope",
        )),
    }
}

/// Verify and deduplicate webhook delivery through a shared cursor. Invoke handlers before saving.
pub fn event_is_new(event: &Event, last: u64) -> bool {
    event.id > last
}
