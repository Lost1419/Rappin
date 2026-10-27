//! Optional Yapin gateway transport. Checkpoints belong to the consumer, not the socket.
use crate::diagnostics::{
    EventTransport, FallbackReason, Telemetry, TransportMode, TransportStatus,
};
use crate::{Client, EventsResponse, Result, YappinError};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{timeout, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

pub const DEFAULT_GATEWAY: &str = "wss://rt.yapper.website/bot";
pub struct Gateway {
    client: Client,
    url: String,
    socket: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    interval: Duration,
    heartbeat: Instant,
    awaiting_ack: bool,
    events: Vec<String>,
    max_age: Option<u32>,
    fallback_until: Option<Instant>,
    telemetry: Telemetry,
    allow_fallback: bool,
    heartbeat_sent: Option<Instant>,
}
impl Gateway {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            url: DEFAULT_GATEWAY.into(),
            socket: None,
            interval: Duration::from_secs(30),
            heartbeat: Instant::now(),
            awaiting_ack: false,
            events: vec![],
            max_age: None,
            fallback_until: None,
            telemetry: Telemetry::new(TransportMode::GatewayPreferred),
            allow_fallback: true,
            heartbeat_sent: None,
        }
    }
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }
    pub fn events(mut self, events: Vec<String>) -> Self {
        self.events = events;
        self
    }
    pub fn max_age(mut self, age: Option<u32>) -> Self {
        self.max_age = age;
        self
    }
    /// Disable REST fallback. A connection failure is returned to the caller.
    pub fn gateway_only(mut self) -> Self {
        self.allow_fallback = false;
        self.telemetry
            .update(|s| s.mode = TransportMode::GatewayOnly);
        self
    }
    pub fn status(&self) -> TransportStatus {
        self.telemetry.status()
    }
    pub fn subscribe_status(&self) -> tokio::sync::watch::Receiver<TransportStatus> {
        self.telemetry.subscribe()
    }
    pub(crate) fn telemetry(mut self, telemetry: Telemetry) -> Self {
        self.telemetry = telemetry;
        self
    }
    pub(crate) fn backpressure(&mut self) {
        self.socket = None;
        self.telemetry.update(|s| {
            s.active = None;
            s.heartbeat_rtt = None;
            s.fallback = Some(FallbackReason::Backpressure);
        });
    }
    async fn connect(&mut self, after: u64) -> Result<()> {
        crate::security::endpoint(&self.url, true)?;
        // These dependency log targets include raw frame payloads, including authentication.
        if wire_trace_enabled() {
            return Err(YappinError::Config(
                "gateway wire tracing is enabled; refusing to send credentials".into(),
            ));
        }
        let (mut ws, _) = timeout(Duration::from_secs(10), connect_async(&self.url))
            .await
            .map_err(|_| transport())?
            .map_err(|_| transport())?;
        let hello = timeout(Duration::from_secs(10), ws.next())
            .await
            .map_err(|_| transport())?
            .ok_or_else(transport)?
            .map_err(|_| transport())?;
        if let Message::Close(frame) = hello {
            return Err(close(frame.map(|f| u16::from(f.code))));
        }
        let value: serde_json::Value =
            serde_json::from_str(hello.to_text().map_err(|_| transport())?)?;
        if value["op"] != "hello" {
            return Err(transport());
        }
        let ms = value["heartbeat_interval"]
            .as_u64()
            .filter(|n| *n >= 100 && *n <= 60000)
            .ok_or_else(transport)?;
        self.interval = Duration::from_millis(ms);
        let mut auth = serde_json::json!({"op":if after==0{"identify"}else{"resume"},"token":self.client.token()});
        if after > 0 {
            auth["seq"] = after.into();
        }
        if !self.events.is_empty() {
            auth["events"] = serde_json::to_value(&self.events)?;
        }
        if let Some(age) = self.max_age {
            auth["max_age"] = age.into();
        }
        if wire_trace_enabled() {
            return Err(YappinError::Config(
                "gateway wire tracing is enabled; refusing to send credentials".into(),
            ));
        }
        ws.send(Message::Text(auth.to_string().into()))
            .await
            .map_err(|_| transport())?;
        self.socket = Some(ws);
        self.heartbeat = Instant::now() + self.interval;
        self.awaiting_ack = false;
        self.heartbeat_sent = None;
        Ok(())
    }
    async fn receive(&mut self, after: u64) -> Result<EventsResponse> {
        if self.socket.is_none() {
            self.connect(after).await?;
        }
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let socket = self.socket.as_mut().ok_or_else(transport)?;
            tokio::select! {
                _=tokio::time::sleep_until(deadline)=>return Ok(EventsResponse{events:vec![],last:after}),
                _=tokio::time::sleep_until(self.heartbeat)=>{
                    if self.awaiting_ack{return Err(transport());}
                    socket.send(Message::Text("{\"op\":\"heartbeat\"}".into())).await.map_err(|_|transport())?;
                    self.awaiting_ack=true;self.heartbeat_sent=Some(Instant::now());self.heartbeat=Instant::now()+self.interval;
                },
                frame=socket.next()=>{
                    match frame.ok_or_else(transport)?.map_err(|_|transport())?{
                        Message::Close(f)=>return Err(close(f.map(|f|u16::from(f.code)))),
                        Message::Ping(p)=>{socket.send(Message::Pong(p)).await.map_err(|_|transport())?;},
                        Message::Text(t)=>{
                            let v:serde_json::Value=serde_json::from_str(&t)?;
                            match v["op"].as_str(){
                                Some("heartbeat_ack")=>{
                                    self.awaiting_ack=false;
                                    if let Some(sent)=self.heartbeat_sent.take(){self.telemetry.update(|s|s.heartbeat_rtt=Some(sent.elapsed()));}
                                },
                                Some("ready")=>{
                                    self.telemetry.update(|s|{s.active=Some(EventTransport::Gateway);s.fallback=None;s.heartbeat_rtt=None;});
                                    if after==0{return Ok(EventsResponse{events:vec![],last:v["seq"].as_u64().unwrap_or(0)});}
                                },
                                Some("event")=>{let event:crate::Event=serde_json::from_value(v["d"].clone())?;if event.id>after{self.telemetry.update(|s|{s.active=Some(EventTransport::Gateway);s.fallback=None;s.received_cursor=event.id;});return Ok(EventsResponse{last:event.id,events:vec![event]});}},
                                _=>(),
                            }
                        },_=>(),
                    }
                }
            }
        }
    }
    /// Reconnect on the next call using the last *handled* ID. Beta denial falls back for 5 min.
    pub async fn poll(&mut self, after: u64) -> Result<EventsResponse> {
        crate::security::endpoint(&self.url, true)?;
        let result = if self.fallback_until.is_some_and(|t| t > Instant::now()) {
            self.client
                .get_events(Some(after), Some(25), self.max_age)
                .await
        } else {
            match self.receive(after).await {
                Ok(batch) => Ok(batch),
                Err(e) => {
                    self.socket = None;
                    self.telemetry.update(|s| {
                        s.active = None;
                        s.heartbeat_rtt = None;
                    });
                    if e.is_unauthorized() {
                        return Err(e);
                    }
                    if !self.allow_fallback {
                        return Err(e);
                    }
                    let delay = match e.status_code() {
                        Some(4003) => 300,
                        Some(4029) | Some(4008) => 60,
                        _ => 5,
                    };
                    self.fallback_until = Some(Instant::now() + Duration::from_secs(delay));
                    let reason = match e.status_code() {
                        Some(4003) => FallbackReason::AccessDenied,
                        Some(4008 | 4029) => FallbackReason::RateLimited,
                        _ if wire_trace_enabled() => FallbackReason::WireLogging,
                        _ => FallbackReason::Unavailable,
                    };
                    self.telemetry.update(|s| {
                        s.active = Some(EventTransport::Polling);
                        s.fallback = Some(reason);
                    });
                    tracing::info!(?reason, "gateway switched to polling");
                    self.client
                        .get_events(Some(after), Some(25), self.max_age)
                        .await
                }
            }
        };
        let mut batch = result?;
        self.telemetry.update(|s| s.received_cursor = batch.last);
        if !self.events.is_empty() {
            batch.events.retain(|e| self.events.contains(&e.kind));
        }
        Ok(batch)
    }
}
fn transport() -> YappinError {
    YappinError::Config("gateway disconnected; resume with the last handled event ID".into())
}
fn close(code: Option<u16>) -> YappinError {
    if code == Some(4004) {
        YappinError::Unauthorized
    } else {
        YappinError::Api {
            status: code.unwrap_or(1006),
            message: "gateway closed".into(),
            retry_after: None,
        }
    }
}

fn wire_trace_enabled() -> bool {
    log::log_enabled!(target:"tungstenite::protocol",log::Level::Trace)
        || log::log_enabled!(target:"tungstenite::protocol::frame",log::Level::Trace)
        || log::log_enabled!(target:"tokio_tungstenite",log::Level::Trace)
}
