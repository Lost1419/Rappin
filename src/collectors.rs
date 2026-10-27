//! Bounded, typed subscriptions. Register before sending a prompt; dropping a collector unregisters it.
use crate::{
    Event, InteractionEvent, MemberJoinEvent, MemberLeaveEvent, MessageEvent, ReactionEvent,
    VoiceEvent,
};
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::{
    collections::HashMap,
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex, Weak,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::mpsc,
    time::{Instant, Sleep},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WaitError {
    #[error("event wait timed out")]
    Timeout,
    #[error("event subscription overflowed; events were lost")]
    Lagged,
    #[error("event source closed or subscription cancelled")]
    Closed,
    #[error("event subscription limit reached")]
    Limit,
    #[error("capacity, timeout and item limit must be positive")]
    InvalidOptions,
}

/// Serializable routing criteria, also usable by isolated Rust workers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    pub kind: Option<String>,
    pub channel_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub author_id: Option<Uuid>,
    pub message_id: Option<Uuid>,
    pub custom_id_prefix: Option<String>,
}
impl EventFilter {
    pub fn matches(&self, event: &Event) -> bool {
        let data = &event.data;
        let is_id = |key: &str, id: Uuid| {
            data[key].as_str().and_then(|s| Uuid::parse_str(s).ok()) == Some(id)
        };
        self.kind.as_ref().is_none_or(|k| &event.kind == k)
            && self.channel_id.is_none_or(|id| is_id("channel_id", id))
            && self.server_id.is_none_or(|id| is_id("server_id", id))
            && self.author_id.is_none_or(|id| {
                is_id(
                    if event.kind == "message" {
                        "author_id"
                    } else {
                        "user_id"
                    },
                    id,
                )
            })
            && self.message_id.is_none_or(|id| {
                is_id(
                    if event.kind == "message" {
                        "id"
                    } else {
                        "message_id"
                    },
                    id,
                )
            })
            && self.custom_id_prefix.as_ref().is_none_or(|prefix| {
                data["custom_id"]
                    .as_str()
                    .is_some_and(|s| s.starts_with(prefix))
            })
    }
}

pub trait Collectible: Clone + Send + Sync + 'static {
    fn from_event(event: &Event) -> Option<Self>;
}
impl Collectible for Event {
    fn from_event(event: &Event) -> Option<Self> {
        Some(event.clone())
    }
}
macro_rules! collectible {
    ($ty:ty, $($kind:literal)|+) => {
        impl Collectible for $ty {
            fn from_event(event:&Event)->Option<Self>{
                if matches!(event.kind.as_str(),$($kind)|+) { serde_json::from_value(event.data.clone()).ok() } else { None }
            }
        }
    };
}
collectible!(MessageEvent, "message");
collectible!(InteractionEvent, "interaction");
collectible!(MemberJoinEvent, "member_join");
collectible!(MemberLeaveEvent, "member_leave");
collectible!(ReactionEvent, "reaction_add" | "reaction_remove");
collectible!(VoiceEvent, "voice_join" | "voice_leave");

type Filter = Arc<dyn Fn(&Event) -> bool + Send + Sync>;
struct Activity {
    last: Mutex<Instant>,
    idle: Option<Duration>,
}
impl Activity {
    fn deadline(&self, expires: Instant) -> Instant {
        let last = *self.last.lock().unwrap_or_else(|p| p.into_inner());
        self.idle
            .and_then(|duration| last.checked_add(duration))
            .map_or(expires, |idle| idle.min(expires))
    }
}
struct Subscription {
    filter: Filter,
    sender: mpsc::Sender<Arc<Event>>,
    state: Arc<AtomicU8>,
    expires: Instant,
    activity: Arc<Activity>,
}
struct Hub {
    closed: AtomicBool,
    next: AtomicU64,
    subscriptions: Mutex<HashMap<u64, Subscription>>,
}
/// Fan-out for authenticated events received by a Bot. It never owns a delivery checkpoint.
#[derive(Clone)]
pub struct EventHub(Arc<Hub>);
impl Default for EventHub {
    fn default() -> Self {
        Self(Arc::new(Hub {
            closed: AtomicBool::new(false),
            next: AtomicU64::new(1),
            subscriptions: Mutex::new(HashMap::new()),
        }))
    }
}
impl EventHub {
    pub fn collect<T: Collectible>(&self) -> CollectorBuilder<T> {
        CollectorBuilder {
            hub: self.clone(),
            criteria: EventFilter::default(),
            filter: Arc::new(|_| true),
            capacity: 64,
            timeout: Duration::from_secs(30),
            idle: None,
            limit: usize::MAX,
        }
    }
    pub fn subscriber_count(&self) -> usize {
        self.0
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
    /// Publish once per received event. Custom transports can use this directly.
    pub fn publish(&self, event: Event) {
        let event = Arc::new(event);
        // User filters run without the registry lock, allowing registration/drop inside a filter.
        let subscribers: Vec<_> = self
            .0
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, s)| {
                (
                    *id,
                    s.filter.clone(),
                    s.sender.clone(),
                    s.state.clone(),
                    s.expires,
                    s.activity.clone(),
                )
            })
            .collect();
        let mut remove = vec![];
        for (id, filter, sender, state, expires, activity) in subscribers {
            if Instant::now() >= activity.deadline(expires) {
                state.store(1, Ordering::SeqCst);
                remove.push(id);
                continue;
            }
            if !filter(&event) {
                continue;
            }
            *activity.last.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
            match sender.try_send(event.clone()) {
                Ok(()) => (),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    state.store(2, Ordering::SeqCst);
                    remove.push(id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => remove.push(id),
            }
        }
        let removed: Vec<_> = {
            let mut subscriptions = self
                .0
                .subscriptions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            remove
                .into_iter()
                .filter_map(|id| subscriptions.remove(&id))
                .collect()
        };
        drop(removed);
    }
    pub(crate) fn reopen(&self) {
        self.0.closed.store(false, Ordering::SeqCst);
    }
    pub(crate) fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        let subscriptions = {
            let mut guard = self
                .0
                .subscriptions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *guard)
        };
        for (_, s) in subscriptions {
            s.state.store(3, Ordering::SeqCst);
        }
    }
}

pub struct CollectorBuilder<T: Collectible> {
    hub: EventHub,
    criteria: EventFilter,
    filter: Arc<dyn Fn(&T) -> bool + Send + Sync>,
    capacity: usize,
    timeout: Duration,
    idle: Option<Duration>,
    limit: usize,
}
impl<T: Collectible> CollectorBuilder<T> {
    pub fn criteria(mut self, criteria: EventFilter) -> Self {
        self.criteria = criteria;
        self
    }
    pub fn channel(mut self, id: Uuid) -> Self {
        self.criteria.channel_id = Some(id);
        self
    }
    pub fn server(mut self, id: Uuid) -> Self {
        self.criteria.server_id = Some(id);
        self
    }
    pub fn author(mut self, id: Uuid) -> Self {
        self.criteria.author_id = Some(id);
        self
    }
    pub fn message(mut self, id: Uuid) -> Self {
        self.criteria.message_id = Some(id);
        self
    }
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.criteria.kind = Some(kind.into());
        self
    }
    pub fn custom_id_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.criteria.custom_id_prefix = Some(prefix.into());
        self
    }
    pub fn filter(mut self, filter: impl Fn(&T) -> bool + Send + Sync + 'static) -> Self {
        self.filter = Arc::new(filter);
        self
    }
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Timeout since the last matching event arrived, independent of consumer polling speed.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle = Some(timeout);
        self
    }
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
    pub fn subscribe(self) -> Result<Collector<T>, WaitError> {
        if self.capacity == 0
            || self.capacity > 4096
            || self.timeout.is_zero()
            || self
                .idle
                .is_some_and(|d| d.is_zero() || Instant::now().checked_add(d).is_none())
            || self.limit == 0
        {
            return Err(WaitError::InvalidOptions);
        }
        let expires = Instant::now()
            .checked_add(self.timeout)
            .ok_or(WaitError::InvalidOptions)?;
        let mut subscriptions = self
            .hub
            .0
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if self.hub.0.closed.load(Ordering::SeqCst) {
            return Err(WaitError::Closed);
        }
        let expired: Vec<_> = subscriptions
            .iter()
            .filter(|(_, s)| {
                s.activity.deadline(s.expires) <= Instant::now() || s.sender.is_closed()
            })
            .map(|(id, _)| *id)
            .collect();
        let removed: Vec<_> = expired
            .into_iter()
            .filter_map(|id| subscriptions.remove(&id))
            .collect();
        for subscription in &removed {
            subscription.state.store(1, Ordering::SeqCst);
        }
        if subscriptions.len() >= 1024 {
            drop(subscriptions);
            drop(removed);
            return Err(WaitError::Limit);
        }
        let id = self.hub.0.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel(self.capacity);
        let state = Arc::new(AtomicU8::new(0));
        let activity = Arc::new(Activity {
            last: Mutex::new(Instant::now()),
            idle: self.idle,
        });
        let criteria = self.criteria;
        let filter = self.filter;
        subscriptions.insert(
            id,
            Subscription {
                filter: Arc::new(move |e| {
                    criteria.matches(e) && T::from_event(e).is_some_and(|v| filter(&v))
                }),
                sender,
                state: state.clone(),
                expires,
                activity: activity.clone(),
            },
        );
        drop(subscriptions);
        drop(removed);
        Ok(Collector {
            hub: Arc::downgrade(&self.hub.0),
            id,
            receiver,
            state,
            expires,
            activity,
            timer: None,
            remaining: self.limit,
            ended: false,
            marker: PhantomData,
        })
    }
    pub async fn wait(self) -> Result<T, WaitError> {
        self.limit(1).subscribe()?.wait().await
    }
}

/// A bounded Stream. Overflow is an explicit terminal Lagged error, never silent loss.
pub struct Collector<T: Collectible> {
    hub: Weak<Hub>,
    id: u64,
    receiver: mpsc::Receiver<Arc<Event>>,
    state: Arc<AtomicU8>,
    expires: Instant,
    activity: Arc<Activity>,
    timer: Option<Pin<Box<Sleep>>>,
    remaining: usize,
    ended: bool,
    marker: PhantomData<fn() -> T>,
}
impl<T: Collectible> Unpin for Collector<T> {}
impl<T: Collectible> Collector<T> {
    pub async fn next(&mut self) -> Result<Option<T>, WaitError> {
        StreamExt::next(self).await.transpose()
    }
    pub async fn wait(mut self) -> Result<T, WaitError> {
        self.next().await?.ok_or(WaitError::Closed)
    }
    fn unregister(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            let removed = hub
                .subscriptions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.id);
            drop(removed);
        }
        self.receiver.close();
    }
    pub fn cancel(&mut self) {
        self.state.store(3, Ordering::SeqCst);
        self.unregister();
    }
}
impl<T: Collectible> Stream for Collector<T> {
    type Item = Result<T, WaitError>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        let deadline = this.activity.deadline(this.expires);
        let timer = this
            .timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        timer.as_mut().reset(deadline);
        let error = match this.state.load(Ordering::SeqCst) {
            1 => Some(WaitError::Timeout),
            2 => Some(WaitError::Lagged),
            3 => Some(WaitError::Closed),
            _ => None,
        };
        if let Some(error) = error.or_else(|| {
            if timer.as_mut().poll(cx).is_ready() {
                Some(WaitError::Timeout)
            } else {
                None
            }
        }) {
            this.ended = true;
            this.unregister();
            return Poll::Ready(Some(Err(error)));
        }
        loop {
            match this.receiver.poll_recv(cx) {
                Poll::Ready(Some(event)) => {
                    if let Some(value) = T::from_event(&event) {
                        this.remaining -= 1;
                        if this.remaining == 0 {
                            this.ended = true;
                            this.unregister();
                        }
                        return Poll::Ready(Some(Ok(value)));
                    }
                }
                Poll::Ready(None) => {
                    this.ended = true;
                    this.unregister();
                    return Poll::Ready(Some(Err(WaitError::Closed)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
impl<T: Collectible> Drop for Collector<T> {
    fn drop(&mut self) {
        self.unregister();
    }
}
