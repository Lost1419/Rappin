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
/// Explicit timeout, overflow, closure or subscription rejection; not permission to replay effects.
///
/// # Example
///
/// ```rust
/// use rappin::{Event, collectors::EventHub};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let hub = EventHub::default();
/// // Subscribe before publishing a prompt or receiving a matching response.
/// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// hub.publish(event);
/// assert_eq!(replies.next().await?.unwrap().id, 1);
/// assert!(replies.next().await?.is_none());
/// # Ok(())
/// # }
/// ```
pub enum WaitError {
    #[error("event wait timed out")]
    /// Total or idle deadline expired before a matching item arrived.
    Timeout,
    #[error("event subscription overflowed; events were lost")]
    /// Subscription overflowed; some events were lost.
    Lagged,
    #[error("event source closed or subscription cancelled")]
    /// Hub closed or the collector was cancelled.
    Closed,
    #[error("event subscription limit reached")]
    /// The hub's active subscription limit was reached.
    Limit,
    #[error("capacity, timeout and item limit must be positive")]
    /// A capacity, timeout or item limit is outside supported positive bounds.
    InvalidOptions,
}

/// Serializable routing criteria, also usable by isolated Rust workers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// # Example
///
/// ```rust
/// use rappin::collectors::EventFilter;
/// let criteria = EventFilter { kind: Some("test".into()), ..Default::default() };
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// assert!(criteria.matches(&event));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub struct EventFilter {
    /// Exact event kind; None does not restrict kind.
    pub kind: Option<String>,
    /// Required data.channel_id, if supplied.
    pub channel_id: Option<Uuid>,
    /// Required data.server_id, if supplied.
    pub server_id: Option<Uuid>,
    /// Required message author_id or other event user_id, depending on event kind.
    pub author_id: Option<Uuid>,
    /// Required message id or interaction message_id, depending on event kind.
    pub message_id: Option<Uuid>,
    /// Required prefix of interaction custom_id, if supplied.
    pub custom_id_prefix: Option<String>,
}
impl EventFilter {
    /// Test all supplied routing criteria without authenticating the event or claiming its checkpoint.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::collectors::EventFilter;
    /// let criteria = EventFilter { kind: Some("test".into()), ..Default::default() };
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// assert!(criteria.matches(&event));
    /// # Ok::<(), serde_json::Error>(())
    /// ```
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
                    if matches!(event.kind.as_str(), "message" | "dm_message") {
                        "author_id"
                    } else {
                        "user_id"
                    },
                    id,
                )
            })
            && self.message_id.is_none_or(|id| {
                is_id(
                    if matches!(event.kind.as_str(), "message" | "dm_message") {
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

/// Decode a typed collector value from a raw event; mismatched kinds or invalid data return None.
///
/// # Example
///
/// ```rust
/// use rappin::{Event, collectors::Collectible};
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// let copy = Event::from_event(&event).unwrap();
/// assert_eq!(copy.id, event.id);
/// # Ok::<(), serde_json::Error>(())
/// ```
pub trait Collectible: Clone + Send + Sync + 'static {
    /// Return a typed copy if this kind/payload is supported; this conversion does not authenticate delivery.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::Collectible};
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// let copy = Event::from_event(&event).unwrap();
    /// assert_eq!(copy.id, event.id);
    /// # Ok::<(), serde_json::Error>(())
    /// ```
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
                if matches!(event.kind.as_str(),$($kind)|+) { serde::Deserialize::deserialize(&event.data).ok() } else { None }
            }
        }
    };
}
impl Collectible for MessageEvent {
    fn from_event(event: &Event) -> Option<Self> {
        match event.kind.as_str() {
            "message" => serde::Deserialize::deserialize(&event.data).ok(),
            "dm_message" => crate::DmMessageEvent::deserialize(&event.data)
                .ok()
                .map(|dm| dm.0),
            _ => None,
        }
    }
}
collectible!(crate::DmMessageEvent, "dm_message");
collectible!(InteractionEvent, "interaction");
collectible!(MemberJoinEvent, "member_join");
collectible!(MemberLeaveEvent, "member_leave");
collectible!(ReactionEvent, "reaction_add" | "reaction_remove");
collectible!(VoiceEvent, "voice_join" | "voice_leave");
collectible!(crate::InviteEvent, "invite_create" | "invite_delete");

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
/// # Example
///
/// ```rust
/// use rappin::{Event, collectors::EventHub};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let hub = EventHub::default();
/// // Subscribe before publishing a prompt or receiving a matching response.
/// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// hub.publish(event);
/// assert_eq!(replies.next().await?.unwrap().id, 1);
/// assert!(replies.next().await?.is_none());
/// # Ok(())
/// # }
/// ```
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
    /// Configure a typed subscription; registration occurs on subscribe/wait.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let hub = EventHub::default();
    /// // Subscribe before publishing a prompt or receiving a matching response.
    /// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// hub.publish(event);
    /// assert_eq!(replies.next().await?.unwrap().id, 1);
    /// assert!(replies.next().await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
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
    /// Current number of registered subscriptions; not an event traffic counter.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let hub = EventHub::default();
    /// // Subscribe before publishing a prompt or receiving a matching response.
    /// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// assert_eq!(hub.subscriber_count(), 1);
    /// hub.publish(event);
    /// assert_eq!(replies.next().await?.unwrap().id, 1);
    /// assert!(replies.next().await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn subscriber_count(&self) -> usize {
        self.0
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
    /// Publish once per received event. Custom transports can use this directly.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let hub = EventHub::default();
    /// // Subscribe before publishing a prompt or receiving a matching response.
    /// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// hub.publish(event);
    /// assert_eq!(replies.next().await?.unwrap().id, 1);
    /// assert!(replies.next().await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
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

/// Bounded typed subscription configuration; build it before posting a prompt.
///
/// # Example
///
/// ```rust
/// use rappin::{Event, collectors::EventHub};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let hub = EventHub::default();
/// // Subscribe before publishing a prompt or receiving a matching response.
/// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// hub.publish(event);
/// assert_eq!(replies.next().await?.unwrap().id, 1);
/// assert!(replies.next().await?.is_none());
/// # Ok(())
/// # }
/// ```
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
    /// Replace routing criteria; a typed filter remains independent.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().criteria(rappin::collectors::EventFilter::default());
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn criteria(mut self, criteria: EventFilter) -> Self {
        self.criteria = criteria;
        self
    }
    /// Restrict to a channel ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().channel(uuid::Uuid::nil());
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn channel(mut self, id: Uuid) -> Self {
        self.criteria.channel_id = Some(id);
        self
    }
    /// Restrict to a server ID.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().server(uuid::Uuid::nil());
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn server(mut self, id: Uuid) -> Self {
        self.criteria.server_id = Some(id);
        self
    }
    /// Restrict to the message author or interaction user.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().author(uuid::Uuid::nil());
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn author(mut self, id: Uuid) -> Self {
        self.criteria.author_id = Some(id);
        self
    }
    /// Restrict to one message ID or interaction target message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().message(uuid::Uuid::nil());
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn message(mut self, id: Uuid) -> Self {
        self.criteria.message_id = Some(id);
        self
    }
    /// Restrict to an exact raw event kind.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().kind("test");
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.criteria.kind = Some(kind.into());
        self
    }
    /// Restrict to interaction custom IDs with this prefix.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().custom_id_prefix("confirm:");
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn custom_id_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.criteria.custom_id_prefix = Some(prefix.into());
        self
    }
    /// Apply a local pure typed predicate after routing criteria.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().filter(|event: &rappin::Event| event.id > 0);
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn filter(mut self, filter: impl Fn(&T) -> bool + Send + Sync + 'static) -> Self {
        self.filter = Arc::new(filter);
        self
    }
    /// Set queue capacity, 1 through 4096; overflow produces Lagged rather than silently dropping results.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().capacity(32);
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }
    /// Set the positive total subscription lifetime.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().timeout(std::time::Duration::from_secs(30));
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Timeout since the last matching event arrived, independent of consumer polling speed.
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().idle_timeout(std::time::Duration::from_secs(10));
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle = Some(timeout);
        self
    }
    /// Complete after this positive number of matching results.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// let hub = EventHub::default();
    /// let configured = hub.collect::<Event>().limit(5);
    /// // Call subscribe before sending a prompt; next/wait returns explicit timeout or loss errors.
    /// ```
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
    /// Register immediately with bounded capacity and deadlines; requires an active Tokio runtime.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let hub = EventHub::default();
    /// // Subscribe before publishing a prompt or receiving a matching response.
    /// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// hub.publish(event);
    /// assert_eq!(replies.next().await?.unwrap().id, 1);
    /// assert!(replies.next().await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
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
    /// Subscribe and await the first match; registration begins when this future is polled.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::{EventHub, WaitError}};
    /// # #[tokio::main]
    /// # async fn main() {
    /// let hub = EventHub::default();
    /// let result = hub.collect::<Event>().timeout(std::time::Duration::from_millis(1)).wait().await;
    /// assert_eq!(result.unwrap_err(), WaitError::Timeout);
    /// # }
    /// ```
    pub async fn wait(self) -> Result<T, WaitError> {
        self.limit(1).subscribe()?.wait().await
    }
}

/// A bounded Stream. Overflow is an explicit terminal Lagged error, never silent loss.
/// # Example
///
/// ```rust
/// use rappin::{Event, collectors::EventHub};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let hub = EventHub::default();
/// // Subscribe before publishing a prompt or receiving a matching response.
/// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
/// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
/// hub.publish(event);
/// assert_eq!(replies.next().await?.unwrap().id, 1);
/// assert!(replies.next().await?.is_none());
/// # Ok(())
/// # }
/// ```
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
    /// Await the next matching value; None means the configured item limit completed. Loss, timeout and closure remain explicit errors.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let hub = EventHub::default();
    /// // Subscribe before publishing a prompt or receiving a matching response.
    /// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// hub.publish(event);
    /// assert_eq!(replies.next().await?.unwrap().id, 1);
    /// assert!(replies.next().await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn next(&mut self) -> Result<Option<T>, WaitError> {
        StreamExt::next(self).await.transpose()
    }
    /// Consume the subscription and return its first matching value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::EventHub};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let hub = EventHub::default();
    /// // Subscribe before publishing a prompt or receiving a matching response.
    /// let mut replies = hub.collect::<Event>().kind("test").limit(1).subscribe()?;
    /// let event: rappin::Event = serde_json::from_value(serde_json::json!({"id":1,"type":"test","data":{}}))?;
    /// hub.publish(event);
    /// assert_eq!(replies.wait().await?.id, 1);
    /// # Ok(())
    /// # }
    /// ```
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
    /// Unregister this waiter; subsequent waits report Closed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rappin::{Event, collectors::{EventHub, WaitError}};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), WaitError> {
    /// let hub = EventHub::default();
    /// let mut waiter = hub.collect::<Event>().subscribe()?;
    /// waiter.cancel();
    /// assert_eq!(waiter.next().await.unwrap_err(), WaitError::Closed);
    /// # Ok(())
    /// # }
    /// ```
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
