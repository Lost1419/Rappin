//! Shutdown evidence for ordinary intake, which has no durable ingress ledger.
use super::{Bot, EventProcessingFailure};
use crate::{Error, RemoteError, Result};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
#[derive(Default)]
pub(super) struct Ledger(Mutex<BTreeMap<u64, State>>);
enum State {
    Queued,
    Started,
    Completed,
    Failed(RemoteError),
}
impl Ledger {
    pub(super) fn queue(&self, ids: impl IntoIterator<Item = u64>) {
        let mut entries = self.0.lock().unwrap();
        for id in ids {
            entries.insert(id, State::Queued);
        }
    }
    pub(super) fn start(&self, id: u64) {
        self.0.lock().unwrap().insert(id, State::Started);
    }
    pub(super) fn finish(&self, id: u64, result: &Result<()>, bot: &Bot) {
        self.0.lock().unwrap().insert(
            id,
            match result {
                Ok(()) => State::Completed,
                Err(error) => State::Failed(bot.failure_cause(error)),
            },
        );
    }
    pub(super) fn prune(&self, cursor: u64) {
        self.0.lock().unwrap().retain(|id, _| *id > cursor);
    }
    pub(super) fn evidence(&self, report: &mut EventProcessingFailure, interruption: &RemoteError) {
        for (&id, state) in self.0.lock().unwrap().iter() {
            if id <= report.committed_cursor {
                continue;
            }
            match state {
                State::Queued => {
                    if !report.unprocessed_events.contains(&id) {
                        report.unprocessed_events.push(id);
                    }
                }
                State::Completed => {
                    if !report.completed_uncommitted_events.contains(&id) {
                        report.completed_uncommitted_events.push(id);
                    }
                }
                State::Started | State::Failed(_) => {
                    if !report.failed_events.iter().any(|e| e.event_id == id) {
                        report.failed_events.push(super::EventFailure {
                            event_id: id,
                            error: match state {
                                State::Failed(error) => error.clone(),
                                _ => interruption.clone(),
                            },
                        });
                    }
                }
            }
        }
    }
}
pub(super) fn timeout(limit: Duration) -> Error {
    Error::Remote(RemoteError {
        kind: "event_drain".into(),
        message: "handler drain deadline reached; effects may be incomplete or uncertain".into(),
        status: None,
        retry_after: None,
        details: BTreeMap::from([
            ("reason".into(), "drain_timeout".into()),
            ("timeout_seconds".into(), limit.as_secs_f64().into()),
            ("outcome".into(), "unknown".into()),
            ("retry_safety".into(), "do_not_replay_workflow".into()),
        ]),
    })
}
pub(super) fn attach(report: &mut EventProcessingFailure, cause: Error) {
    report.intake_error = Some(match report.intake_error.take() {
        Some(original) => Error::FailureHandling {
            source: Box::new(original.into()),
            handling_error: Box::new(cause),
        }
        .to_remote(),
        None => cause.to_remote(),
    });
}

pub(super) struct Guard(pub(super) super::BotHandle);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0
            .inner
            .draining
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
