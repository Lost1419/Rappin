//! Connection-scoped JSON requests. This module never retries or falls back.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GatewayRequestConfig {
    pub timeout: Duration,
}
impl Default for GatewayRequestConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RequestTransport {
    Https,
    Gateway,
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GatewayRequestStatus {
    pub enabled: bool,
    pub capability: bool,
    pub connected: bool,
    pub selected: RequestTransport,
    pub in_flight: usize,
    pub submitted: u64,
    pub completed: u64,
    pub admission_rejected: u64,
    pub unknown_responses: u64,
    pub uncertain_completions: u64,
    pub response_time: Duration,
    /// Last connection termination, separate from a request response deadline.
    pub last_disconnect: Option<crate::RemoteError>,
}
impl Default for GatewayRequestStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            capability: false,
            connected: false,
            selected: RequestTransport::Https,
            in_flight: 0,
            submitted: 0,
            completed: 0,
            admission_rejected: 0,
            unknown_responses: 0,
            uncertain_completions: 0,
            response_time: Duration::ZERO,
            last_disconnect: None,
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct RequestRouter(Arc<Mutex<RouterState>>);
#[derive(Default)]
struct RouterState {
    config: Option<GatewayRequestConfig>,
    active: Option<Channel>,
    epoch: Option<uuid::Uuid>,
    status: GatewayRequestStatus,
}
#[derive(Clone)]
struct Channel {
    epoch: uuid::Uuid,
    sender: mpsc::Sender<Request>,
    slots: Arc<Semaphore>,
    terminal: Arc<Mutex<Option<crate::RemoteError>>>,
}
struct Request {
    method: String,
    path: String,
    body: Option<Value>,
    reply: oneshot::Sender<Result<reqwest::Response>>,
    state: Arc<AtomicU8>,
    deadline: Instant,
    _slot: OwnedSemaphorePermit,
}
struct Pending {
    request: Request,
    started: Instant,
}
struct Cancel {
    state: Arc<AtomicU8>,
    router: RequestRouter,
    finished: bool,
}
impl Drop for Cancel {
    fn drop(&mut self) {
        if !self.finished
            && self
                .state
                .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            && self.state.load(Ordering::SeqCst) == 1
        {
            self.router
                .update(|s| s.uncertain_completions = s.uncertain_completions.saturating_add(1));
        }
    }
}
fn failure(submitted: bool, reason: &str) -> YappinError {
    YappinError::Remote(crate::RemoteError {
        kind: "gateway_request".into(),
        message: reason.into(),
        status: None,
        retry_after: None,
        details: std::collections::BTreeMap::from([
            (
                "outcome".into(),
                if submitted { "unknown" } else { "not_applied" }.into(),
            ),
            ("retry_safety".into(), "do_not_replay_workflow".into()),
            ("submitted".into(), submitted.into()),
            (
                "reason".into(),
                match reason {
                    "gateway request capacity is full" => "capacity",
                    "gateway request connection is closed or full" => "admission_closed",
                    "gateway request expired before submission" => "queue_timeout",
                    "gateway request expired after possible submission" => "response_timeout",
                    _ => "invalid_response",
                }
                .into(),
            ),
        ]),
    })
}
fn interrupted(submitted: bool, reason: &str, cause: Option<&crate::RemoteError>) -> YappinError {
    let mut error = failure(
        submitted,
        "gateway request did not receive a confirmed response",
    )
    .to_remote();
    error.details.insert("reason".into(), reason.into());
    if let Some(cause) = cause {
        error.status = cause.status;
        error.retry_after = cause.retry_after;
        error.details.insert("source".into(), cause.to_json());
    }
    error.into()
}
fn termination(reason: &str, source: Option<&YappinError>) -> crate::RemoteError {
    let mut error = disconnected().to_remote();
    error.details.insert("reason".into(), reason.into());
    if let Some(source) = source {
        error.status = source.status_code();
        error.retry_after = source.retry_after().map(|d| d.as_secs_f64());
        error.details.insert("source".into(), source.to_json());
    }
    error
}
// Pump tasks do not inherit the caller's availability task-local context.
// Keep transient I/O distinguishable from terminal protocol/configuration errors.
fn disconnected() -> YappinError {
    YappinError::Remote(crate::RemoteError {
        kind: "gateway_transport".into(),
        message: "gateway connection was interrupted".into(),
        status: None,
        retry_after: None,
        details: Default::default(),
    })
}
fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> YappinError {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        Error::Io(error) => {
            let mut cause = termination("io_error", None);
            cause
                .details
                .insert("io_kind".into(), format!("{:?}", error.kind()).into());
            cause.into()
        }
        Error::Tls(_) => termination("tls_error", None).into(),
        Error::ConnectionClosed | Error::AlreadyClosed => {
            termination("peer_disconnected", None).into()
        }
        Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ) => termination("connection_reset", None).into(),
        _ => YappinError::Config("gateway returned an invalid protocol frame".into()),
    }
}
impl RequestRouter {
    pub(crate) fn configure(&self, config: Option<GatewayRequestConfig>) -> Result<()> {
        if config
            .as_ref()
            .is_some_and(|c| c.timeout.is_zero() || c.timeout > Duration::from_secs(120))
        {
            return Err(YappinError::validation(
                "gateway_requests.timeout",
                "must be greater than zero and at most 120 seconds",
            ));
        }
        let mut state = self.0.lock().unwrap();
        state.status.enabled = config.is_some();
        state.config = config;
        Ok(())
    }
    pub(crate) fn enabled(&self) -> bool {
        self.0.lock().unwrap().config.is_some()
    }
    pub(crate) fn status(&self) -> GatewayRequestStatus {
        self.0.lock().unwrap().status.clone()
    }
    fn update(&self, f: impl FnOnce(&mut GatewayRequestStatus)) {
        f(&mut self.0.lock().unwrap().status)
    }
    pub(super) fn capability(&self, capability: bool) {
        let mut state = self.0.lock().unwrap();
        state.active = None;
        state.status.connected = false;
        state.status.capability = capability;
    }
    fn disconnect(&self, epoch: uuid::Uuid, cause: crate::RemoteError) {
        let mut s = self.0.lock().unwrap();
        if s.epoch == Some(epoch) {
            s.status.last_disconnect = Some(cause);
        }
        if s.active.as_ref().is_some_and(|c| c.epoch == epoch) {
            s.active = None;
            s.status.connected = false;
            s.status.capability = false;
            s.status.in_flight = 0;
        }
    }
    pub(crate) async fn request(
        &self,
        method: &reqwest::Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> Option<Result<reqwest::Response>> {
        let selected = {
            let mut state = self.0.lock().unwrap();
            if !matches!(
                *method,
                reqwest::Method::GET
                    | reqwest::Method::POST
                    | reqwest::Method::PUT
                    | reqwest::Method::PATCH
                    | reqwest::Method::DELETE
            ) || path
                .split('?')
                .next()
                .is_some_and(|p| p.trim_end_matches('/') == "/events")
                || state.config.is_none()
                || state.active.is_none()
            {
                state.status.selected = RequestTransport::Https;
                return None;
            }
            state.status.selected = RequestTransport::Gateway;
            (
                state.active.clone().unwrap(),
                state.config.as_ref().unwrap().timeout,
            )
        };
        let (channel, duration) = selected;
        // Leave ordinary HTTPS calls on their original serialization path.
        let body = match body.map(serde_json::from_slice::<Value>).transpose() {
            Ok(body) => body,
            Err(error) => return Some(Err(error.into())),
        };
        if serde_json::to_vec(&json!({"op":"request","id":"00000000-0000-0000-0000-000000000000:18446744073709551615","method":method.as_str(),"path":path,"body":body})).map_or(true, |v|v.len()>1024*1024) {
            self.update(|s| s.selected = RequestTransport::Https);
            return None;
        }
        let slot = match channel.slots.clone().try_acquire_owned() {
            Ok(slot) => slot,
            Err(_) => {
                self.update(|s| s.admission_rejected = s.admission_rejected.saturating_add(1));
                return Some(Err(failure(false, "gateway request capacity is full")));
            }
        };
        let state = Arc::new(AtomicU8::new(0));
        let (reply, receipt) = oneshot::channel();
        let request = Request {
            method: method.as_str().into(),
            path: path.into(),
            body,
            reply,
            state: state.clone(),
            deadline: Instant::now() + duration,
            _slot: slot,
        };
        if channel.sender.try_send(request).is_err() {
            self.update(|s| s.admission_rejected = s.admission_rejected.saturating_add(1));
            return Some(Err(failure(
                false,
                "gateway request connection is closed or full",
            )));
        }
        let mut cancel = Cancel {
            state: state.clone(),
            router: self.clone(),
            finished: false,
        };
        let result = match timeout(duration, receipt).await {
            Ok(Ok(result)) => result,
            failed => {
                let submitted = state
                    .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                    && state.load(Ordering::SeqCst) == 1;
                let cause = channel.terminal.lock().unwrap().clone();
                let reason = if failed.is_err() {
                    "request_timeout"
                } else {
                    cause
                        .as_ref()
                        .and_then(|e| e.details.get("reason"))
                        .and_then(Value::as_str)
                        .unwrap_or("response_channel_closed")
                };
                Err(interrupted(submitted, reason, cause.as_ref()))
            }
        };
        cancel.finished = true;
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.to_json()["outcome"] == "unknown")
        {
            self.update(|s| s.uncertain_completions = s.uncertain_completions.saturating_add(1));
        }
        Some(result)
    }
}
/// Pipeline-owned response lifetimes; gateway admission tasks retain only a weak
/// reference. Dropping event intake never disconnects handlers being drained.
#[derive(Default)]
pub(crate) struct ResponseLease(Arc<Mutex<LeaseState>>);
#[derive(Clone)]
pub(crate) struct WeakResponseLease(std::sync::Weak<Mutex<LeaseState>>);
#[derive(Default)]
struct LeaseState {
    owners: Vec<Arc<PumpOwner>>,
    closed: Option<String>,
}
impl ResponseLease {
    pub(crate) fn downgrade(&self) -> WeakResponseLease {
        WeakResponseLease(Arc::downgrade(&self.0))
    }
    pub(crate) fn shutdown(&self, reason: &str) {
        let (owners, reason) = {
            let mut lease = self.0.lock().unwrap();
            let reason = lease.closed.get_or_insert_with(|| reason.into()).clone();
            (std::mem::take(&mut lease.owners), reason)
        };
        for owner in owners {
            owner.shutdown(&reason);
        }
    }
}
impl WeakResponseLease {
    fn retain(&self, owner: &Arc<PumpOwner>) {
        let reason = match self.0.upgrade() {
            Some(lease) => {
                let mut lease = lease.lock().unwrap();
                if let Some(reason) = &lease.closed {
                    Some(reason.clone())
                } else {
                    lease.owners.retain(|owner| !owner.task.is_finished());
                    lease.owners.push(owner.clone());
                    None
                }
            }
            None => Some("run_cancelled".into()),
        };
        // A concurrent handshake may finish after admission has stopped. Never
        // resurrect a closed lease or hold its mutex while aborting a pump.
        if let Some(reason) = reason {
            owner.shutdown(&reason);
        }
    }
}
impl Drop for ResponseLease {
    fn drop(&mut self) {
        self.shutdown("run_cancelled");
    }
}
struct PumpOwner {
    task: tokio::task::AbortHandle,
    router: RequestRouter,
    epoch: uuid::Uuid,
    terminal: Arc<Mutex<Option<crate::RemoteError>>>,
    slots: Arc<Semaphore>,
}
impl PumpOwner {
    fn shutdown(&self, reason: &str) {
        let cause = {
            let mut terminal = self.terminal.lock().unwrap();
            terminal
                .get_or_insert_with(|| termination(reason, None))
                .clone()
        };
        self.router.disconnect(self.epoch, cause);
        self.task.abort();
    }
}
impl Drop for PumpOwner {
    fn drop(&mut self) {
        self.shutdown("gateway_dropped");
    }
}
#[derive(Default)]
struct EventDelivery {
    paused: AtomicBool,
    error: Mutex<Option<crate::RemoteError>>,
    changed: tokio::sync::Notify,
}
impl EventDelivery {
    fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.error
            .lock()
            .unwrap()
            .get_or_insert_with(|| termination("event_backpressure", None));
        self.changed.notify_waiters();
    }
}
pub(super) struct Pump {
    receiver: mpsc::Receiver<Result<crate::Event>>,
    owner: Arc<PumpOwner>,
    delivery: Arc<EventDelivery>,
}
pub(super) fn event_backpressure() -> YappinError {
    termination("event_backpressure", None).into()
}
impl Pump {
    pub(super) fn start(
        mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
        router: RequestRouter,
        interval: Duration,
        telemetry: Telemetry,
        lease: Option<&WeakResponseLease>,
    ) -> Self {
        let epoch = uuid::Uuid::new_v4();
        let (sender, mut requests) = mpsc::channel::<Request>(8);
        let (events, receiver) = mpsc::channel(64);
        let terminal = Arc::new(Mutex::new(None));
        let delivery = Arc::new(EventDelivery::default());
        let event_delivery = delivery.clone();
        let slots = Arc::new(Semaphore::new(8));
        {
            let mut state = router.0.lock().unwrap();
            state.active = Some(Channel {
                epoch,
                sender,
                slots: slots.clone(),
                terminal: terminal.clone(),
            });
            state.epoch = Some(epoch);
            state.status.connected = true;
            state.status.capability = true;
        }
        let route = router.clone();
        let ended = terminal.clone();
        let task = tokio::spawn(async move {
            let mut pending: HashMap<String, Pending> = HashMap::new();
            let mut sent: VecDeque<Instant> = VecDeque::new();
            let mut counter = 0u64;
            let mut heartbeat = Instant::now() + interval;
            let mut awaiting = None::<Instant>;
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let result: Result<()> = async {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep_until(heartbeat) => {
                            if awaiting.is_some() { return Err(termination("heartbeat_timeout",None).into()); }
                            send_frame(&mut socket, Message::Text("{\"op\":\"heartbeat\"}".into())).await?;
                            awaiting = Some(Instant::now());
                            heartbeat = Instant::now() + interval;
                        },
                        frame = socket.next() => {
                            match frame.ok_or_else(disconnected)?.map_err(socket_error)? {
                                Message::Close(frame) => return Err(close(frame.map(|f| u16::from(f.code)))),
                                Message::Ping(payload) => send_frame(&mut socket, Message::Pong(payload)).await?,
                                Message::Text(text) => {
                                    let value: Value = serde_json::from_str(&text)?;
                                    match value["op"].as_str() {
                                        Some("heartbeat_ack") => {
                                            if let Some(started) = awaiting.take() {
                                                telemetry.update(|s| s.heartbeat_rtt = Some(started.elapsed()));
                                            }
                                        },
                                        Some("event") => {
                                            let event = serde_json::from_value(value["d"].clone())?;
                                            if !event_delivery.paused.load(Ordering::SeqCst) && events.try_send(Ok(event)).is_err() {
                                                // Preserve response correlations and heartbeat. Lost
                                                // event coverage is explicit; never checkpoint it.
                                                event_delivery.pause();
                                            }
                                        },
                                        Some("response") => {
                                            let id = value["id"].as_str().unwrap_or_default();
                                            if let Some(completed) = pending.remove(id) {
                                                route.update(|s| {
                                                    s.in_flight = pending.len();
                                                    s.completed = s.completed.saturating_add(1);
                                                    s.response_time = s.response_time.saturating_add(completed.started.elapsed());
                                                });
                                                let _ = completed.request.reply.send(response(&value));
                                            } else {
                                                route.update(|s| s.unknown_responses = s.unknown_responses.saturating_add(1));
                                            }
                                        },
                                        _ => (),
                                    }
                                },
                                _ => (),
                            }
                        },
                        _ = tick.tick() => {
                            let expired: Vec<_> = pending.iter()
                                .filter(|(_, p)| p.request.deadline <= Instant::now() || p.request.reply.is_closed())
                                .map(|(id, _)| id.clone()).collect();
                            for id in expired {
                                if let Some(expired) = pending.remove(&id) {
                                    let _ = expired.request.reply.send(Err(failure(true, "gateway request expired after possible submission")));
                                }
                            }
                            route.update(|s| s.in_flight = pending.len());
                        },
                        request = requests.recv(), if pending.len() < 8 => {
                            let Some(mut request) = request else { return Ok(()); };
                            if request.reply.is_closed() || request.state.load(Ordering::SeqCst) == 2 { continue; }
                            if request.deadline <= Instant::now() {
                                let _ = request.reply.send(Err(failure(false, "gateway request expired before submission")));
                                continue;
                            }
                            while sent.front().is_some_and(|t| t.elapsed() >= Duration::from_secs(10)) { sent.pop_front(); }
                            if sent.len() >= 100 {
                                let delay = Duration::from_secs(10).saturating_sub(sent.front().unwrap().elapsed()).as_secs_f64();
                                let _ = request.reply.send(response(&json!({"status":429,"body":{"error":"gateway socket request limit","retry_after":delay}})));
                                continue;
                            }
                            counter = counter.checked_add(1).ok_or_else(protocol_failure)?;
                            let id = format!("{epoch}:{counter}");
                            let mut frame = json!({"op":"request","id":id});
                            frame["method"] = std::mem::take(&mut request.method).into();
                            frame["path"] = std::mem::take(&mut request.path).into();
                            if let Some(body) = request.body.take() { frame["body"] = body; }
                            if request.state.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err() { continue; }
                            sent.push_back(Instant::now());
                            route.update(|s| s.submitted = s.submitted.saturating_add(1));
                            // A sink write error cannot prove that the peer received no bytes.
                            pending.insert(id, Pending { request, started: Instant::now() });
                            route.update(|s| s.in_flight = pending.len());
                            send_frame(&mut socket, Message::Text(frame.to_string().into())).await?;
                        }
                    }
                }
            }.await;
            let final_cause = match &result {
                Err(error) => {
                    let reason = if error.status_code().is_some() {
                        "peer_closed".to_owned()
                    } else {
                        error
                            .to_remote()
                            .details
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("transport_failure")
                            .to_owned()
                    };
                    termination(&reason, Some(error))
                }
                Ok(()) => termination("request_channel_closed", None),
            };
            let final_cause = ended.lock().unwrap().get_or_insert(final_cause).clone();
            route.disconnect(epoch, final_cause.clone());
            if let Err(error) = result {
                let cause = error.to_json();
                for (_, pending) in pending {
                    let reason = final_cause
                        .details
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("transport_failure");
                    let mut failure = interrupted(true, reason, Some(&final_cause)).to_remote();
                    failure.status = error.status_code();
                    failure.retry_after = error.retry_after().map(|d| d.as_secs_f64());
                    failure.details.insert("source".into(), cause.clone());
                    let _ = pending.request.reply.send(Err(failure.into()));
                }
                let _ = events.try_send(Err(error));
            }
            drop(requests);
        });
        let owner = Arc::new(PumpOwner {
            task: task.abort_handle(),
            router,
            epoch,
            terminal,
            slots,
        });
        if let Some(lease) = lease {
            lease.retain(&owner);
        }
        // The task is independently owned by its abort handle; it has no strong
        // reference to PumpOwner, so this cannot create a reference cycle.
        drop(task);
        Self {
            receiver,
            owner,
            delivery,
        }
    }
    pub(super) fn pause(&self) {
        self.delivery.pause();
    }
    pub(super) fn paused(&self) -> bool {
        self.delivery.paused.load(Ordering::SeqCst)
    }
    pub(super) fn retire_if_idle(&self) -> bool {
        let mut state = self.owner.router.0.lock().unwrap();
        if self.owner.slots.available_permits() != 8 && !self.owner.task.is_finished() {
            return false;
        }
        if state
            .active
            .as_ref()
            .is_some_and(|c| c.epoch == self.owner.epoch)
        {
            state.active = None;
            state.status.connected = false;
        }
        drop(state);
        self.owner.shutdown("event_backpressure");
        true
    }
    pub(super) async fn poll(
        &mut self,
        after: u64,
        deliver_duplicates: bool,
    ) -> Result<EventsResponse> {
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let changed = self.delivery.changed.notified();
            if let Some(error) = self.delivery.error.lock().unwrap().clone() {
                return Err(error.into());
            }
            let received = tokio::select! {
                value=tokio::time::timeout_at(deadline,self.receiver.recv())=>value,
                _=changed=>continue,
            };
            match received {
                Err(_) => {
                    return Ok(EventsResponse {
                        events: vec![],
                        last: after,
                    })
                }
                Ok(Some(Ok(event))) if deliver_duplicates || event.id > after => {
                    return Ok(EventsResponse {
                        last: after.max(event.id),
                        events: vec![event],
                    })
                }
                Ok(Some(Ok(_))) => continue,
                Ok(Some(Err(error))) => return Err(error),
                Ok(None) => return Err(disconnected()),
            }
        }
    }
}
async fn send_frame(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    frame: Message,
) -> Result<()> {
    timeout(Duration::from_secs(1), socket.send(frame))
        .await
        .map_err(|_| YappinError::from(termination("send_timeout", None)))?
        .map_err(socket_error)
}
fn response(value: &Value) -> Result<reqwest::Response> {
    let status = value["status"]
        .as_u64()
        .and_then(|s| u16::try_from(s).ok())
        .filter(|s| (100..600).contains(s))
        .ok_or_else(|| failure(true, "gateway response status is invalid"))?;
    let mut response = http::Response::builder().status(status);
    if let Some(delay) = value
        .get("retry_after")
        .or_else(|| value["body"].get("retry_after"))
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s >= 0.0)
    {
        response = response.header("Retry-After", delay.to_string());
    }
    let body = serde_json::to_vec(value.get("body").unwrap_or(&json!({})))?;
    Ok(reqwest::Response::from(
        response
            .body(reqwest::Body::from(body))
            .map_err(|_| failure(true, "gateway response is invalid"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn response_lease_closes_late_handshakes_and_preserves_the_first_reason() {
        for late in [false, true] {
            let lease = ResponseLease::default();
            let weak = lease.downgrade();
            let router = RequestRouter::default();
            let task = tokio::spawn(std::future::pending::<()>());
            let owner = Arc::new(PumpOwner {
                task: task.abort_handle(),
                router: router.clone(),
                epoch: uuid::Uuid::new_v4(),
                terminal: Default::default(),
                slots: Arc::new(Semaphore::new(8)),
            });
            router.0.lock().unwrap().epoch = Some(owner.epoch);
            if !late {
                weak.retain(&owner);
            }
            lease.shutdown("drain_timeout");
            lease.shutdown("pipeline_shutdown");
            if late {
                weak.retain(&owner);
            }
            assert!(task.await.unwrap_err().is_cancelled());
            assert_eq!(
                owner.terminal.lock().unwrap().as_ref().unwrap().details["reason"],
                "drain_timeout"
            );
            assert!(lease.0.lock().unwrap().owners.is_empty());
            drop(owner);
            assert_eq!(
                router.status().last_disconnect.unwrap().details["reason"],
                "drain_timeout"
            );
        }
        let weak = ResponseLease::default().downgrade();
        let task = tokio::spawn(std::future::pending::<()>());
        let owner = Arc::new(PumpOwner {
            task: task.abort_handle(),
            router: Default::default(),
            epoch: uuid::Uuid::new_v4(),
            terminal: Default::default(),
            slots: Arc::new(Semaphore::new(8)),
        });
        weak.retain(&owner);
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            owner.terminal.lock().unwrap().as_ref().unwrap().details["reason"],
            "run_cancelled"
        );
    }
    #[tokio::test]
    async fn retired_pump_cannot_replace_a_new_connections_disconnect_cause() {
        let router = RequestRouter::default();
        let task = tokio::spawn(std::future::pending::<()>());
        let owner = PumpOwner {
            task: task.abort_handle(),
            router: router.clone(),
            epoch: uuid::Uuid::new_v4(),
            terminal: Default::default(),
            slots: Arc::new(Semaphore::new(8)),
        };
        let latest = uuid::Uuid::new_v4();
        router.0.lock().unwrap().epoch = Some(latest);
        router.disconnect(latest, termination("peer_closed", None));
        drop(owner);
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            router.status().last_disconnect.unwrap().details["reason"],
            "peer_closed"
        );
    }
    fn queued_router(timeout: Duration) -> (RequestRouter, mpsc::Receiver<Request>) {
        let router = RequestRouter::default();
        let config = GatewayRequestConfig { timeout };
        router.configure(Some(config)).unwrap();
        let (sender, receiver) = mpsc::channel(8);
        router.0.lock().unwrap().active = Some(Channel {
            epoch: uuid::Uuid::new_v4(),
            sender,
            slots: Arc::new(Semaphore::new(8)),
            terminal: Arc::new(Mutex::new(None)),
        });
        (router, receiver)
    }
    #[tokio::test]
    async fn https_selection_does_not_decode_the_gateway_body() {
        let router = RequestRouter::default();
        assert!(router
            .request(&reqwest::Method::POST, "/mutate", Some(b"not JSON"))
            .await
            .is_none());
        router
            .configure(Some(GatewayRequestConfig::default()))
            .unwrap();
        assert!(router
            .request(&reqwest::Method::POST, "/mutate", Some(b"not JSON"))
            .await
            .is_none());
    }
    #[tokio::test]
    async fn queued_timeout_proves_no_later_submission_is_possible() {
        let (router, mut receiver) = queued_router(Duration::from_millis(10));
        let route = router.clone();
        let future = tokio::spawn(async move {
            route
                .request(&reqwest::Method::POST, "/mutate", None)
                .await
                .unwrap()
        });
        let queued = receiver.recv().await.unwrap();
        let error = future.await.unwrap().unwrap_err();
        assert_eq!(error.to_json()["submitted"], false);
        assert_eq!(error.to_json()["outcome"], "not_applied");
        assert!(queued
            .state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err());
        assert_eq!(router.status().uncertain_completions, 0);
    }
    #[tokio::test]
    async fn queued_cancellation_and_disconnect_preserve_non_submission() {
        for cancel in [false, true] {
            let (router, mut receiver) = queued_router(Duration::from_secs(1));
            let route = router.clone();
            let future = tokio::spawn(async move {
                route
                    .request(&reqwest::Method::POST, "/mutate", None)
                    .await
                    .unwrap()
            });
            let queued = receiver.recv().await.unwrap();
            let state = queued.state.clone();
            if cancel {
                future.abort();
                let _ = future.await;
            } else {
                drop(queued);
                assert_eq!(
                    future.await.unwrap().unwrap_err().to_json()["submitted"],
                    false
                );
            }
            assert!(state
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err());
            assert_eq!(router.status().uncertain_completions, 0);
        }
    }
}
