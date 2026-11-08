//! Explicit completion decisions for failed command and button invocations.
use crate::{RemoteError, Result, YappinError};
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use std::future::Future;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Whether a command/button failure is an expected typed rejection or unexpected/incomplete work.
///
/// # Example
///
/// ```rust
/// use rappin::{Bot, FailureClass, FailureDisposition};
/// let bot = Bot::builder("unused-documentation-token")
///     .on_command_failure(|ctx, failure| async move {
///         if failure.class == FailureClass::Rejected {
///             ctx.reply("The request was rejected.").await?;
///             Ok(FailureDisposition::Handled)
///         } else {
///             // Preserve unexpected/partial/uncertain failures for reconciliation.
///             Ok(FailureDisposition::Stop)
///         }
///     }).build();
/// ```
pub enum FailureClass {
    /// Typed user-input/check rejection; no incomplete handler work is implied.
    Rejected,
    /// Unexpected error or incomplete work; default behavior retains the event barrier and stops intake.
    Unexpected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Explicit completion decision; no disposition retries a handler.
///
/// # Example
///
/// ```rust
/// use rappin::{Bot, FailureClass, FailureDisposition};
/// let bot = Bot::builder("unused-documentation-token")
///     .on_command_failure(|ctx, failure| async move {
///         if failure.class == FailureClass::Rejected {
///             ctx.reply("The request was rejected.").await?;
///             Ok(FailureDisposition::Handled)
///         } else {
///             // Preserve unexpected/partial/uncertain failures for reconciliation.
///             Ok(FailureDisposition::Stop)
///         }
///     }).build();
/// ```
pub enum FailureDisposition {
    /// Reply to a typed rejection and complete it; propagate unexpected failures.
    Default,
    /// The callback explicitly completed/reconciled this invocation. Never retries it.
    Handled,
    /// Leave the event incomplete. Stop intake, drain active work, retain a cursor barrier.
    Stop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Command or button stage reporting the original failure.
///
/// # Example
///
/// ```rust
/// use rappin::{Bot, FailureClass, FailureDisposition};
/// let bot = Bot::builder("unused-documentation-token")
///     .on_command_failure(|ctx, failure| async move {
///         if failure.class == FailureClass::Rejected {
///             ctx.reply("The request was rejected.").await?;
///             Ok(FailureDisposition::Handled)
///         } else {
///             // Preserve unexpected/partial/uncertain failures for reconciliation.
///             Ok(FailureDisposition::Stop)
///         }
///     }).build();
/// ```
pub enum HandlerKind {
    /// A native or worker command invocation.
    Command,
    /// A native or worker button invocation.
    Button,
}

/// Safe structured original cause, identical for native and worker invocations.
#[derive(Clone, Debug, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{Bot, FailureClass, FailureDisposition};
/// let bot = Bot::builder("unused-documentation-token")
///     .on_command_failure(|ctx, failure| async move {
///         if failure.class == FailureClass::Rejected {
///             ctx.reply("The request was rejected.").await?;
///             Ok(FailureDisposition::Handled)
///         } else {
///             // Preserve unexpected/partial/uncertain failures for reconciliation.
///             Ok(FailureDisposition::Stop)
///         }
///     }).build();
/// ```
pub struct HandlerFailure {
    /// Original intake event ID.
    pub event_id: u64,
    /// Command or button that failed.
    pub kind: HandlerKind,
    /// Registered command name or button route.
    pub handler: String,
    /// Expected rejection or unexpected failure classification.
    pub class: FailureClass,
    /// Sanitized structured original cause, including available status, retry and mutation evidence.
    pub error: RemoteError,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
/// An incomplete event and its original sanitized structured cause.
///
/// # Example
///
/// ```rust
/// use rappin::{EventProcessingFailure, YappinError};
/// fn inspect(error: &YappinError) {
///     if let YappinError::EventProcessing { report } = error {
///         println!("Committed through {}; failed: {}", report.committed_cursor, report.failed_events.len());
///         for failure in &report.failed_events {
///             println!("Event {}: {}", failure.event_id, failure.error);
///         }
///         // Reconcile incomplete effects; do not replay the batch blindly.
///     }
/// }
/// ```
pub struct EventFailure {
    /// Event that did not complete.
    pub event_id: u64,
    /// Original structured failure for that event.
    pub error: RemoteError,
}
/// A stopped run's failed and not-started work. Later completed events can still
/// be uncommitted: only the ordered successful prefix is checkpointed.
#[derive(Clone, Debug, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::{EventProcessingFailure, YappinError};
/// fn inspect(error: &YappinError) {
///     if let YappinError::EventProcessing { report } = error {
///         println!("Committed through {}; failed: {}", report.committed_cursor, report.failed_events.len());
///         for failure in &report.failed_events {
///             println!("Event {}: {}", failure.event_id, failure.error);
///         }
///         // Reconcile incomplete effects; do not replay the batch blindly.
///     }
/// }
/// ```
pub struct EventProcessingFailure {
    /// Last successfully persisted ordered polling checkpoint.
    pub committed_cursor: u64,
    /// Latest watermark received from intake; not proof of completed processing.
    pub received_cursor: u64,
    /// Events whose processing failed, with original structured causes.
    pub failed_events: Vec<EventFailure>,
    /// Received event IDs whose handlers never started.
    pub unprocessed_events: Vec<u64>,
    /// Completed events beyond an earlier barrier; completion does not imply their cursor was committed.
    pub completed_uncommitted_events: Vec<u64>,
    /// Independent authentication, transport or intake cause, if any.
    pub intake_error: Option<RemoteError>,
    /// Independent checkpoint persistence cause, if any.
    pub cursor_error: Option<RemoteError>,
}
impl EventProcessingFailure {
    pub(crate) fn primary(&self) -> Option<&RemoteError> {
        self.cursor_error
            .as_ref()
            .or(self.intake_error.as_ref())
            .or_else(|| self.failed_events.first().map(|e| &e.error))
    }
}
pub(crate) async fn catch<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    std::panic::AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            Err(YappinError::HandlerInterrupted {
                reason: "handler or failure callback panicked".into(),
                source: None,
            })
        })
}

impl super::Bot {
    pub(super) fn failure_cause(&self, error: &YappinError) -> RemoteError {
        serde_json::from_value(self.client().redact_credentials(error.to_json()))
            .expect("sanitized error structure")
    }
    fn failure_info(
        &self,
        event_id: u64,
        kind: HandlerKind,
        handler: &str,
        error: &YappinError,
    ) -> HandlerFailure {
        let class = if error.is_command_rejection() {
            FailureClass::Rejected
        } else {
            FailureClass::Unexpected
        };
        let error = self.failure_cause(error);
        HandlerFailure {
            event_id,
            kind,
            handler: handler.into(),
            class,
            error,
        }
    }
    pub(super) async fn reply_rejection(
        &self,
        ctx: &super::MessageContext,
        error: &RemoteError,
        usage: Option<&str>,
    ) -> Result<()> {
        let text = format!(
            "{}{}",
            error,
            usage.map(|u| format!("\nUsage: {u}")).unwrap_or_default()
        );
        catch(async {
            let parts = crate::split_message(&text);
            let split = parts.len() > 1;
            let mut completed = vec![];
            for (index, part) in parts.into_iter().enumerate() {
                match ctx.reply(part).await {
                    Ok(message) => completed.push(
                        self.client()
                            .redact_credentials(serde_json::json!({"message":message})),
                    ),
                    Err(error) if split => {
                        return Err(YappinError::PartialWrite {
                            completed,
                            failed_step: index + 1,
                            source: Box::new(error),
                        })
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })
        .await
    }
    pub(super) async fn command_failed(
        &self,
        ctx: super::MessageContext,
        event_id: u64,
        name: &str,
        usage: Option<String>,
        error: YappinError,
    ) -> Result<()> {
        let info = self.failure_info(event_id, HandlerKind::Command, name, &error);
        if let Some(callback) = &self.handle.inner.command_failure_handler {
            match catch(async { callback(ctx.clone(), info.clone()).await }).await {
                Ok(FailureDisposition::Handled) => return Ok(()),
                Ok(FailureDisposition::Stop) => return Err(error),
                Ok(FailureDisposition::Default) => (),
                Err(handling_error) => {
                    return Err(YappinError::FailureHandling {
                        source: Box::new(error),
                        handling_error: Box::new(handling_error),
                    })
                }
            }
        }
        if info.class == FailureClass::Rejected {
            let result = self
                .reply_rejection(&ctx, &info.error, usage.as_deref())
                .await;
            return result.map_err(|handling_error| YappinError::FailureHandling {
                source: Box::new(error),
                handling_error: Box::new(handling_error),
            });
        }
        if let Some(callback) = &self.handle.inner.command_error_handler {
            let returned = info.error;
            if let Err(handling_error) = catch(async {
                callback(ctx, error).await;
                Ok(())
            })
            .await
            {
                return Err(YappinError::FailureHandling {
                    source: Box::new(returned.into()),
                    handling_error: Box::new(handling_error),
                });
            }
            return Err(returned.into());
        }
        tracing::error!("Command /{name} failed: {}", info.error);
        Err(error)
    }
    pub(super) async fn button_failed(
        &self,
        ctx: super::InteractionContext,
        event_id: u64,
        prefix: &str,
        error: YappinError,
    ) -> Result<()> {
        let info = self.failure_info(event_id, HandlerKind::Button, prefix, &error);
        if let Some(callback) = &self.handle.inner.button_failure_handler {
            match catch(async { callback(ctx.clone(), info.clone()).await }).await {
                Ok(FailureDisposition::Handled) => return Ok(()),
                Ok(FailureDisposition::Stop) => return Err(error),
                Ok(FailureDisposition::Default) => (),
                Err(handling_error) => {
                    return Err(YappinError::FailureHandling {
                        source: Box::new(error),
                        handling_error: Box::new(handling_error),
                    })
                }
            }
        }
        if info.class == FailureClass::Rejected {
            return catch(ctx.ephemeral(info.error.to_string()))
                .await
                .map_err(|handling_error| YappinError::FailureHandling {
                    source: Box::new(error),
                    handling_error: Box::new(handling_error),
                });
        }
        if let Some(callback) = &self.handle.inner.button_error_handler {
            let returned = info.error;
            if let Err(handling_error) = catch(async {
                callback(ctx, error).await;
                Ok(())
            })
            .await
            {
                return Err(YappinError::FailureHandling {
                    source: Box::new(returned.into()),
                    handling_error: Box::new(handling_error),
                });
            }
            return Err(returned.into());
        }
        tracing::error!("Button {prefix} failed: {}", info.error);
        Err(error)
    }
}
