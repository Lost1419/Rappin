//! Opt-in completion policy for server-message gates and listeners.
use super::{failure::catch, FailureClass, FailureDisposition, MessageContext};
use crate::{RemoteError, Result, YappinError};
use serde::{Deserialize, Serialize};

/// Completion of a server-message gate or fallible listener.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::{Bot, MessageDisposition};
/// let bot = Bot::builder("unused-documentation-token")
///     .message_gate(|ctx| async move {
///         if ctx.server_id().is_none() { return Ok(MessageDisposition::Continue); }
///         // Replace this condition with application authorization.
///         if ctx.content().trim().is_empty() { return Ok(MessageDisposition::Handled); }
///         Ok(MessageDisposition::Continue)
///     }).build();
/// ```
pub enum MessageDisposition {
    /// Run the next dispatch stage. This does not checkpoint the event yet.
    Continue,
    /// Required work is complete; skip the remaining command/listener dispatch.
    Handled,
    /// Leave the event incomplete and stop intake with a cursor barrier.
    /// Use Err(cause) instead when an original structured cause is available.
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
/// Stage that produced a fallible server-message failure.
///
/// # Example
///
/// ```rust
/// use rappin::MessageStage;
/// let stage = MessageStage::Gate;
/// assert_ne!(stage, MessageStage::Listener);
/// ```
pub enum MessageStage {
    /// Shared message gate, before command parsing and dispatch.
    Gate,
    /// Fallible event-handler or cog message listener after command dispatch.
    Listener,
}

/// Original sanitized cause for a gate or fallible message listener.
/// Separate from HandlerFailure so existing exhaustive HandlerKind matches work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
/// # Example
///
/// ```rust
/// use rappin::{Bot, FailureDisposition};
/// let bot = Bot::builder("unused-documentation-token")
///     .on_message_failure(|_ctx, failure| async move {
///         eprintln!("{:?} / {}: {}", failure.stage, failure.handler, failure.error);
///         Ok(FailureDisposition::Stop)
///     }).build();
/// ```
pub struct MessageFailure {
    /// Authenticated original event ID.
    pub event_id: u64,
    /// Gate or listener stage that failed.
    pub stage: MessageStage,
    /// "gate", "event_handler", or the cog's registered name.
    pub handler: String,
    /// Expected rejection or unexpected/incomplete classification.
    pub class: FailureClass,
    /// Original sanitized structured cause; status and mutation evidence are retained.
    pub error: RemoteError,
}

impl MessageDisposition {
    pub(super) fn completed(
        self,
        event_id: u64,
        stage: MessageStage,
        handler: &str,
    ) -> Result<bool> {
        match self {
            Self::Continue => Ok(false),
            Self::Handled => Ok(true),
            Self::Stop => Err(YappinError::Remote(RemoteError {
                kind: "message_stopped".into(),
                message: "message processing explicitly stopped before completion".into(),
                status: None,
                retry_after: None,
                details: [
                    ("event_id".into(), event_id.into()),
                    (
                        "stage".into(),
                        serde_json::to_value(stage).expect("message stage"),
                    ),
                    ("handler".into(), handler.into()),
                    ("outcome".into(), "incomplete".into()),
                    ("retry_safety".into(), "do_not_replay_workflow".into()),
                ]
                .into(),
            })),
        }
    }
}

impl super::Bot {
    pub(super) async fn message_result(
        &self,
        ctx: &MessageContext,
        event_id: u64,
        stage: MessageStage,
        handler: &str,
        result: Result<MessageDisposition>,
    ) -> Result<bool> {
        let error = match result {
            Ok(disposition) => return disposition.completed(event_id, stage, handler),
            Err(error) => error,
        };
        let info = MessageFailure {
            event_id,
            stage,
            handler: handler.into(),
            class: if error.is_command_rejection() {
                FailureClass::Rejected
            } else {
                FailureClass::Unexpected
            },
            error: self.failure_cause(&error),
        };
        if let Some(callback) = &self.handle.inner.message_failure_handler {
            match catch(async { callback(ctx.clone(), info.clone()).await }).await {
                Ok(FailureDisposition::Handled) => return Ok(stage == MessageStage::Gate),
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
            self.reply_rejection(ctx, &info.error, None)
                .await
                .map_err(|handling_error| YappinError::FailureHandling {
                    source: Box::new(error),
                    handling_error: Box::new(handling_error),
                })?;
            // A rejected gate never falls through to commands. A completed listener
            // rejection completes that invocation only, like a command rejection.
            return Ok(stage == MessageStage::Gate);
        }
        Err(error)
    }
}
