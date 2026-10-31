//! Bounded name searches that distinguish uniqueness from an unfinished scan.
use crate::{Client, Member, Result, YappinError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupStatus {
    Unique,
    NotFound,
    Ambiguous,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchStop {
    Exhausted,
    PageLimit,
    NoProgress,
    ExactId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LookupResult<T> {
    pub matches: Vec<T>,
    pub complete: bool,
    pub scanned: usize,
    pub pages: usize,
    pub next_cursor: Option<Uuid>,
    pub stop: SearchStop,
}
impl<T> LookupResult<T> {
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
    pub async fn search_channels(
        &self,
        server: Option<Uuid>,
        name: &str,
    ) -> Result<LookupResult<crate::Channel>> {
        let channels = match server {
            Some(id) => self.get_server_channels(id, None).await?.channels,
            None => self.get_channels(None).await?,
        };
        let scanned = channels.len();
        Ok(LookupResult {
            matches: channels
                .into_iter()
                .filter(|c| c.id.to_string() == name || c.name.eq_ignore_ascii_case(name))
                .collect(),
            complete: true,
            scanned,
            pages: 1,
            next_cursor: None,
            stop: SearchStop::Exhausted,
        })
    }
    /// Searches at most max_pages pages, preserving candidates on incomplete scans.
    /// A full final page cannot establish uniqueness, even if it contains one match.
    pub async fn search_members(
        &self,
        server: Uuid,
        name: &str,
        max_pages: usize,
    ) -> Result<LookupResult<Member>> {
        if !(1..=100).contains(&max_pages) {
            return Err(YappinError::validation(
                "max_pages",
                "must be between 1 and 100",
            ));
        }
        if let Ok(id) = name.parse::<Uuid>() {
            let member = self.get_member(server, id).await?.member;
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
            let batch = self.get_members(server, Some(500), out.next_cursor).await?;
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
}
