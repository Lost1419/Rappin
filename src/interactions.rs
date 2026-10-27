//! Response coordination shared by Client clones, contexts, and the worker API proxy.
use crate::{InteractionEvent, Result, YappinError};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Mutex as AsyncMutex, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponsePhase {
    #[default]
    Unanswered,
    Acknowledged,
    Responded,
    Uncertain,
    Expired,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct InteractionState {
    pub phase: ResponsePhase,
    pub responses: u32,
}
pub(crate) struct Tracked {
    pub state: InteractionState,
    pub expires: Instant,
}
pub(crate) struct Registry {
    entries: Mutex<HashMap<[u8; 32], Arc<AsyncMutex<Tracked>>>>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}
pub(crate) fn path(id: &str, token: &str) -> String {
    let encode = |value: &str| {
        url::form_urlencoded::byte_serialize(value.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    };
    format!("/interactions/{}/{}/callback", encode(id), encode(token))
}
impl Registry {
    pub(crate) fn entry(&self, path: &str) -> Result<Arc<AsyncMutex<Tracked>>> {
        let key: [u8; 32] = Sha256::digest(path.as_bytes()).into();
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|_, entry| {
            entry
                .try_lock()
                .map_or(true, |tracked| tracked.expires > Instant::now())
        });
        if let Some(entry) = entries.get(&key) {
            return Ok(entry.clone());
        }
        if entries.len() >= 4096 {
            return Err(YappinError::Config(
                "interaction state capacity reached".into(),
            ));
        }
        let entry = Arc::new(AsyncMutex::new(Tracked {
            state: InteractionState::default(),
            expires: Instant::now() + Duration::from_secs(900),
        }));
        entries.insert(key, entry.clone());
        Ok(entry)
    }
}

impl crate::Client {
    pub async fn interaction_state(&self, event: &InteractionEvent) -> Result<InteractionState> {
        if event
            .created_at
            .is_some_and(|time| (chrono::Utc::now() - time).num_seconds() >= 900)
        {
            return Ok(InteractionState {
                phase: ResponsePhase::Expired,
                responses: 0,
            });
        }
        let entry = self
            .interaction_registry()
            .entry(&path(&event.id, &event.token))?;
        let tracked = entry.lock().await;
        if Instant::now() >= tracked.expires {
            return Ok(InteractionState {
                phase: ResponsePhase::Expired,
                ..tracked.state
            });
        }
        Ok(tracked.state)
    }
    pub(crate) fn auto_ack(&self, event: InteractionEvent, after: Duration) {
        let Some(permit) = self.auto_ack_permit() else {
            tracing::warn!("automatic acknowledgement capacity reached");
            return;
        };
        let client = self.clone();
        let task = tokio::spawn(async move {
            let _permit = permit;
            let age = event
                .created_at
                .map(|time| {
                    (chrono::Utc::now() - time)
                        .to_std()
                        .unwrap_or(Duration::ZERO)
                })
                .unwrap_or(Duration::ZERO);
            // Leave one second for the API, and count time already spent in delivery.
            let delay = after.min(Duration::from_secs(2).saturating_sub(age));
            tokio::time::sleep(delay).await;
            if let Err(error) = client
                .respond_interaction(&event.id, &event.token, &crate::InteractionCallback::ack())
                .await
            {
                tracing::warn!(
                    status = error.status_code(),
                    "automatic interaction acknowledgement failed"
                );
            }
        });
        self.track_auto_ack(task.abort_handle());
    }
}
