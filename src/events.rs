//! Durable at-least-once polling. A single consumer owns each cursor file.
use crate::{Client, Event, EventsResponse, Result, YappinError};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
/// Single-owner persisted polling checkpoint with atomic file replacement.
///
/// # Example
///
/// ```rust
/// use rappin::events::EventCursor;
/// let cursor = EventCursor::new("./state/events.json");
/// assert_eq!(cursor.path(), std::path::Path::new("./state/events.json"));
/// // One running consumer must own this file; atomic replacement is not a cross-process lease.
/// ```
pub struct EventCursor {
    path: PathBuf,
}
impl EventCursor {
    /// Configure the checkpoint path without reading or writing the file.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::events::EventCursor;
    /// let cursor = EventCursor::new("./state/events.json");
    /// assert_eq!(cursor.path(), std::path::Path::new("./state/events.json"));
    /// // One running consumer must own this file; atomic replacement is not a cross-process lease.
    /// ```
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    /// Borrow the configured checkpoint path.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::events::EventCursor;
    /// let cursor = EventCursor::new("./state/events.json");
    /// assert_eq!(cursor.path(), std::path::Path::new("./state/events.json"));
    /// // One running consumer must own this file; atomic replacement is not a cross-process lease.
    /// ```
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Read the saved unsigned cursor; an absent file is None, while invalid or inaccessible files return an error.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{events::EventCursor, Result};
    /// async fn checkpoint(cursor: &EventCursor, completed_watermark: u64) -> Result<()> {
    ///     let saved = cursor.load().await?;
    ///     println!("Checkpoint: {saved:?}");
    ///     Ok(())
    /// }
    /// ```
    pub async fn load(&self) -> Result<Option<u64>> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                YappinError::Config("invalid event cursor: expected an unsigned event ID".into())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    /// Atomic replace: a crash cannot leave a partially written cursor.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{events::EventCursor, Result};
    /// async fn checkpoint(cursor: &EventCursor, completed_watermark: u64) -> Result<()> {
    ///     cursor.save(completed_watermark).await?;
    ///     Ok(())
    /// }
    /// ```
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
/// # Example
///
/// ```no_run
/// use rappin::{events::EventConsumer, Result};
/// async fn log_batch(consumer: &mut EventConsumer) -> Result<()> {
///     let batch = consumer.poll(25).await?;
///     for event in &batch.events { println!("{} {}", event.id, event.kind); }
///     // This consumer only logs. Mutating handlers must establish complete outcomes first.
///     consumer.acknowledge(batch.last).await
/// }
/// ```
pub struct EventConsumer {
    client: Client,
    cursor: Option<EventCursor>,
    last: Option<u64>,
    pending: Option<EventsResponse>,
    max_age: Option<u32>,
}
impl EventConsumer {
    /// Create a manual polling consumer; no requests occur until poll.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, events::EventConsumer};
    /// let consumer = EventConsumer::new(Client::new("unused-documentation-token"))
    ///     .cursor_file("./state/events.json").after(42).max_age(900);
    /// assert_eq!(consumer.last(), Some(42));
    /// ```
    pub fn new(client: Client) -> Self {
        Self {
            client,
            cursor: None,
            last: None,
            pending: None,
            max_age: None,
        }
    }
    /// Configure a single-owner durable checkpoint.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, events::EventConsumer};
    /// let consumer = EventConsumer::new(Client::new("unused-documentation-token"))
    ///     .cursor_file("./state/events.json").after(42).max_age(900);
    /// assert_eq!(consumer.last(), Some(42));
    /// ```
    pub fn cursor_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.cursor = Some(EventCursor::new(path));
        self
    }
    /// Set an explicit starting cursor; this takes precedence over loading a file.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, events::EventConsumer};
    /// let consumer = EventConsumer::new(Client::new("unused-documentation-token"))
    ///     .cursor_file("./state/events.json").after(42).max_age(900);
    /// assert_eq!(consumer.last(), Some(42));
    /// ```
    pub fn after(mut self, id: u64) -> Self {
        self.last = Some(id);
        self
    }
    /// Request events no older than this many seconds.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, events::EventConsumer};
    /// let consumer = EventConsumer::new(Client::new("unused-documentation-token"))
    ///     .cursor_file("./state/events.json").after(42).max_age(900);
    /// assert_eq!(consumer.last(), Some(42));
    /// ```
    pub fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = Some(seconds);
        self
    }
    /// Latest locally acknowledged or initialized watermark.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Client, events::EventConsumer};
    /// let consumer = EventConsumer::new(Client::new("unused-documentation-token"))
    ///     .cursor_file("./state/events.json").after(42).max_age(900);
    /// assert_eq!(consumer.last(), Some(42));
    /// ```
    pub fn last(&self) -> Option<u64> {
        self.last
    }
    /// Fetch a batch or return the unacknowledged local batch again; fresh startup initializes the current watermark rather than replaying history.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{events::EventConsumer, Result};
    /// async fn log_batch(consumer: &mut EventConsumer) -> Result<()> {
    ///     let batch = consumer.poll(25).await?;
    ///     for event in &batch.events { println!("{} {}", event.id, event.kind); }
    ///     // This consumer only logs. Mutating handlers must establish complete outcomes first.
    ///     consumer.acknowledge(batch.last).await
    /// }
    /// ```
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{events::EventConsumer, Result};
    /// async fn log_batch(consumer: &mut EventConsumer) -> Result<()> {
    ///     let batch = consumer.poll(25).await?;
    ///     for event in &batch.events { println!("{} {}", event.id, event.kind); }
    ///     // This consumer only logs. Mutating handlers must establish complete outcomes first.
    ///     consumer.acknowledge(batch.last).await
    /// }
    /// ```
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
        if self.last != Some(id) {
            if let Some(c) = &self.cursor {
                c.save(id).await?;
            }
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
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// use uuid::Uuid;
    /// async fn use_client(client: &Client, server: Uuid, channel: Uuid) -> Result<()> {
    ///     let result = client.message_history(channel, None, 3).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// Collect up to 1 through 100 pages of members; a bounded result does not establish an exhaustive inventory.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// use uuid::Uuid;
    /// async fn use_client(client: &Client, server: Uuid, channel: Uuid) -> Result<()> {
    ///     let result = client.member_pages(server, None, 3).await?;
    ///     Ok(())
    /// }
    /// ```
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
    /// Resolve a unique channel name in the supplied scope; ambiguity returns an error.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// use uuid::Uuid;
    /// async fn use_client(client: &Client, server: Uuid, channel: Uuid) -> Result<()> {
    ///     let result = client.resolve_channel(Some(server), "general").await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn resolve_channel(
        &self,
        server: Option<uuid::Uuid>,
        name: &str,
    ) -> Result<crate::Channel> {
        self.search_channels(server, name)
            .await?
            .into_unique("channel")
    }
    /// Resolve only a unique match in a fully searched scope. An incomplete
    /// bounded scan returns IncompleteSearch, never a guessed candidate.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// use uuid::Uuid;
    /// async fn use_client(client: &Client, server: Uuid, channel: Uuid) -> Result<()> {
    ///     let result = client.resolve_member(server, "river", 3).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn resolve_member(
        &self,
        server: uuid::Uuid,
        name: &str,
        max_pages: usize,
    ) -> Result<crate::Member> {
        self.search_members(server, name, max_pages)
            .await?
            .into_unique("member")
    }

    /// Sequentially send split message parts. This legacy helper returns the first error without prior receipts; use explicit per-part tracking or validated bindings when partial-write reconciliation is required.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// use uuid::Uuid;
    /// async fn use_client(client: &Client, server: Uuid, channel: Uuid) -> Result<()> {
    ///     let result = client.send_long_message(channel, "Long text").await?;
    ///     Ok(())
    /// }
    /// ```
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
/// Compare only the numeric event ID with a saved watermark. This does not authenticate
/// delivery, detect conflicting payloads or establish completed processing.
/// Use shared ingress for mixed-source deduplication.
/// # Example
///
/// ```rust
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// assert!(rappin::events::event_is_new(&event, 0));
/// assert!(!rappin::events::event_is_new(&event, 1));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn event_is_new(event: &Event, last: u64) -> bool {
    event.id > last
}
