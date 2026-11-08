//! # Example
//!
//! ```rust
//! use rappin::{LookupResult, SearchStop, LookupStatus};
//! let result = LookupResult { matches: vec![42], complete: true, scanned: 1, pages: 1, next_cursor: None, stop: SearchStop::Exhausted };
//! assert_eq!(result.status(), LookupStatus::Unique);
//! ```
//! Bounded name searches that distinguish uniqueness from an unfinished scan.
use crate::{Client, Member, Result, YappinError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Lookup classification that separates a unique result from incomplete evidence.
///
/// # Example
///
/// ```rust
/// use rappin::{LookupResult, SearchStop, LookupStatus};
/// let result = LookupResult { matches: vec![42], complete: true, scanned: 1, pages: 1, next_cursor: None, stop: SearchStop::Exhausted };
/// assert_eq!(result.status(), LookupStatus::Unique);
/// ```
pub enum LookupStatus {
    /// Exactly one match was found and the requested scan completed.
    Unique,
    /// No matches were found in a completed scan.
    NotFound,
    /// At least two matches were found, regardless of whether the scan completed.
    Ambiguous,
    /// At most one candidate was found but the scan did not prove completeness.
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Why a bounded inventory search ended.
///
/// # Example
///
/// ```rust
/// let stop = rappin::SearchStop::PageLimit;
/// assert!(matches!(stop, rappin::SearchStop::PageLimit));
/// ```
pub enum SearchStop {
    /// The requested inventory was exhausted.
    Exhausted,
    /// The configured page limit stopped scanning before completeness was established.
    PageLimit,
    /// Repeated or nonadvancing cursor data prevented safe continuation.
    NoProgress,
    /// A scoped exact-ID fetch resolved the target directly.
    ExactId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Candidates and completion evidence from a bounded lookup. One candidate from an incomplete scan is not proof of uniqueness.
///
/// # Example
///
/// ```rust
/// use rappin::{LookupResult, SearchStop, LookupStatus};
/// let result = LookupResult { matches: vec![42], complete: true, scanned: 1, pages: 1, next_cursor: None, stop: SearchStop::Exhausted };
/// assert_eq!(result.status(), LookupStatus::Unique);
/// assert_eq!(result.into_unique("member").unwrap(), 42);
/// ```
pub struct LookupResult<T> {
    /// Candidates found within the inspected search scope.
    pub matches: Vec<T>,
    /// Whether the scan established completeness for the requested scope.
    pub complete: bool,
    /// Distinct candidate records inspected.
    pub scanned: usize,
    /// Pages requested while scanning.
    pub pages: usize,
    /// Continuation cursor when the search stopped before exhaustion.
    pub next_cursor: Option<Uuid>,
    /// Reason scanning stopped.
    pub stop: SearchStop,
}
impl<T> LookupResult<T> {
    /// Classify candidates and completeness. Multiple matches are ambiguous even if the scan is incomplete.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{LookupResult, SearchStop, LookupStatus};
    /// let result = LookupResult { matches: vec![42], complete: true, scanned: 1, pages: 1, next_cursor: None, stop: SearchStop::Exhausted };
    /// assert_eq!(result.status(), LookupStatus::Unique);
    /// ```
    pub fn status(&self) -> LookupStatus {
        if self.matches.len() > 1 {
            LookupStatus::Ambiguous
        } else if !self.complete {
            LookupStatus::Incomplete
        } else if self.matches.is_empty() {
            LookupStatus::NotFound
        } else {
            LookupStatus::Unique
        }
    }
    /// Return the only match from a complete scan. Otherwise return an ambiguity, not-found or structured IncompleteSearch error without choosing a candidate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{LookupResult, SearchStop, LookupStatus};
    /// let result = LookupResult { matches: vec![42], complete: true, scanned: 1, pages: 1, next_cursor: None, stop: SearchStop::Exhausted };
    /// assert_eq!(result.into_unique("member").unwrap(), 42);
    /// let unfinished = LookupResult { matches: vec![42], complete: false, scanned: 1, pages: 1, next_cursor: None, stop: SearchStop::PageLimit };
    /// assert!(matches!(unfinished.into_unique("member"), Err(rappin::YappinError::IncompleteSearch { .. })));
    /// ```
    pub fn into_unique(mut self, resource: &str) -> Result<T> {
        match self.status() {
            LookupStatus::Unique => Ok(self.matches.remove(0)),
            LookupStatus::Incomplete => Err(YappinError::IncompleteSearch {
                resource: resource.into(),
                scanned: self.scanned,
                pages: self.pages,
                next_cursor: self.next_cursor,
                matches: self.matches.len(),
            }),
            LookupStatus::NotFound => Err(YappinError::validation(
                resource,
                "no match in the requested search scope",
            )),
            LookupStatus::Ambiguous => Err(YappinError::validation(
                resource,
                "ambiguous name; use a stable ID or narrower scope",
            )),
        }
    }
}
impl Client {
    /// Search channel IDs or case-insensitive names in the selected server, or across visible channels when server is None. Use into_unique to reject missing or ambiguous results.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// async fn find(client: &Client, server: uuid::Uuid) -> Result<()> {
    ///     let result = client.search_channels(Some(server), "general").await?;
    ///     let channel = result.into_unique("channel")?;
    ///     println!("Resolved channel: {}", channel.id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn search_channels(
        &self,
        server: Option<Uuid>,
        name: &str,
    ) -> Result<LookupResult<crate::Channel>> {
        let channels = match server {
            Some(id) => self.get_server_channels(id, None).await?.channels,
            None => self.get_channels(None).await?,
        };
        Ok(channel_matches(channels, name))
    }
    /// Searches at most max_pages pages, preserving candidates on incomplete scans.
    /// A full final page cannot establish uniqueness, even if it contains one match.
    /// # Example
    ///
    /// ```no_run
    /// use rappin::{Client, Result};
    /// async fn find(client: &Client, server: uuid::Uuid) -> Result<()> {
    ///     let result = client.search_members(server, "river", 3).await?;
    ///     let member = result.into_unique("member")?;
    ///     println!("Resolved member: {}", member.user.id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn search_members(
        &self,
        server: Uuid,
        name: &str,
        max_pages: usize,
    ) -> Result<LookupResult<Member>> {
        search_members_with(
            name,
            max_pages,
            |id| self.get_member(server, id),
            |after| self.get_members(server, Some(500), after),
        )
        .await
    }
}

pub(crate) fn channel_matches(
    channels: Vec<crate::Channel>,
    name: &str,
) -> LookupResult<crate::Channel> {
    let scanned = channels.len();
    LookupResult {
        matches: channels
            .into_iter()
            .filter(|c| c.id.to_string() == name || c.name.eq_ignore_ascii_case(name))
            .collect(),
        complete: true,
        scanned,
        pages: 1,
        next_cursor: None,
        stop: SearchStop::Exhausted,
    }
}
pub(crate) async fn search_members_with<F, P, MF, PF>(
    name: &str,
    max_pages: usize,
    get_member: F,
    get_page: P,
) -> Result<LookupResult<Member>>
where
    F: Fn(Uuid) -> MF + Send + Sync,
    P: Fn(Option<Uuid>) -> PF + Send + Sync,
    MF: std::future::Future<Output = Result<crate::MemberDetail>> + Send,
    PF: std::future::Future<Output = Result<Vec<Member>>> + Send,
{
    if !(1..=100).contains(&max_pages) {
        return Err(YappinError::validation(
            "max_pages",
            "must be between 1 and 100",
        ));
    }
    if let Ok(id) = name.parse::<Uuid>() {
        let member = get_member(id).await?.member;
        if member.user.id != id {
            return Err(YappinError::validation(
                "member",
                "API returned a different member ID",
            ));
        }
        return Ok(LookupResult {
            matches: vec![member],
            complete: true,
            scanned: 1,
            pages: 1,
            next_cursor: None,
            stop: SearchStop::ExactId,
        });
    }
    let mut out = LookupResult {
        matches: vec![],
        complete: false,
        scanned: 0,
        pages: 0,
        next_cursor: None,
        stop: SearchStop::PageLimit,
    };
    let mut seen = std::collections::HashSet::new();
    for _ in 0..max_pages {
        let batch = get_page(out.next_cursor).await?;
        out.pages += 1;
        let size = batch.len();
        let next = batch.last().map(|m| m.user.id);
        let mut progress = true;
        for member in batch {
            if out
                .next_cursor
                .is_some_and(|cursor| member.user.id <= cursor)
                || !seen.insert(member.user.id)
            {
                progress = false;
                continue;
            }
            out.scanned += 1;
            if member.user.username.eq_ignore_ascii_case(name)
                || member.effective_name().eq_ignore_ascii_case(name)
            {
                out.matches.push(member);
            }
        }
        if !progress || (size > 0 && next <= out.next_cursor) {
            out.stop = SearchStop::NoProgress;
            break;
        }
        if size < 500 {
            out.complete = true;
            out.next_cursor = None;
            out.stop = SearchStop::Exhausted;
            break;
        }
        out.next_cursor = next;
    }
    Ok(out)
}
