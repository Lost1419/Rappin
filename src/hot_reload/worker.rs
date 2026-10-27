use super::protocol::{self, CogEvent, Listener, Manifest, Wire};
use crate::{Command, CreateMessage, InteractionCallback, Message, Result, YappinError};
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{
    io::{BufReader, Stdin, Stdout},
    sync::Mutex,
};

type Handler =
    Arc<dyn Fn(WorkerContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;
struct Connection {
    input: BufReader<Stdin>,
    output: Stdout,
    poisoned: bool,
}

/// Rust handler context inside a worker. Authenticated JSON API requests go through the parent.
#[derive(Clone)]
pub struct WorkerContext {
    pub event: CogEvent,
    connection: Arc<Mutex<Connection>>,
    active: Arc<AtomicBool>,
}
impl WorkerContext {
    pub fn args(&self) -> &str {
        self.event.args()
    }
    pub fn argv(&self) -> &[String] {
        self.event.argv()
    }
    pub fn author_id(&self) -> Option<uuid::Uuid> {
        self.event.author_id()
    }
    pub fn channel_id(&self) -> Option<uuid::Uuid> {
        self.event.channel_id()
    }
    /// JSON-only API access; paths are checked by the parent Client. No bot token is exposed.
    pub async fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        self.ensure_active()?;
        let mut connection = self.connection.lock().await;
        self.ensure_active()?;
        if connection.poisoned {
            return Err(protocol::protocol_error());
        }
        // Cancellation during an exchange must not let a later call consume its response.
        connection.poisoned = true;
        protocol::write(
            &mut connection.output,
            &Wire::Api {
                method: method.into(),
                path: path.into(),
                body,
            },
        )
        .await?;
        match protocol::read(&mut connection.input).await? {
            Wire::ApiResult { value, error } => {
                connection.poisoned = false;
                match error {
                    Some(error) => Err(YappinError::Config(error)),
                    None => Ok(value.unwrap_or(Value::Null)),
                }
            }
            _ => Err(protocol::protocol_error()),
        }
    }
    fn ensure_active(&self) -> Result<()> {
        if self.active.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(YappinError::Config(
                "worker handler has already returned".into(),
            ))
        }
    }
    pub async fn send(&self, channel: uuid::Uuid, message: CreateMessage) -> Result<Message> {
        message.validate()?;
        let response = self
            .api(
                "POST",
                &format!("/channels/{channel}/messages"),
                Some(serde_json::to_value(message)?),
            )
            .await?;
        Ok(serde_json::from_value(response["message"].clone())?)
    }
    pub async fn say(&self, content: impl Into<String>) -> Result<Message> {
        let channel = self
            .channel_id()
            .ok_or_else(|| YappinError::Config("event has no channel".into()))?;
        self.send(channel, CreateMessage::new(content)).await
    }
    pub async fn reply(&self, content: impl Into<String>) -> Result<()> {
        let content = content.into();
        if let Some(message) = self.event.message() {
            self.send(
                message.channel_id,
                CreateMessage::reply(content, message.id),
            )
            .await?;
            Ok(())
        } else {
            self.respond(InteractionCallback::reply(content)).await
        }
    }
    pub async fn reply_with(&self, mut message: CreateMessage) -> Result<Message> {
        let event = self
            .event
            .message()
            .ok_or_else(|| YappinError::Config("reply_with requires a message event".into()))?;
        if message.reply_to.is_none() {
            message.reply_to = Some(event.id);
        }
        self.send(event.channel_id, message).await
    }
    pub async fn respond(&self, callback: InteractionCallback) -> Result<()> {
        callback.validate()?;
        let event = self
            .event
            .interaction()
            .ok_or_else(|| YappinError::Config("event is not an interaction".into()))?;
        let encode = |s: &str| {
            url::form_urlencoded::byte_serialize(s.as_bytes())
                .collect::<String>()
                .replace('+', "%20")
        };
        self.api(
            "POST",
            &format!(
                "/interactions/{}/{}/callback",
                encode(&event.id),
                encode(&event.token)
            ),
            Some(serde_json::to_value(callback)?),
        )
        .await?;
        Ok(())
    }
    pub async fn ephemeral(&self, content: impl Into<String>) -> Result<()> {
        self.respond(InteractionCallback::ephemeral(content)).await
    }
    pub async fn update(&self, content: impl Into<String>) -> Result<()> {
        self.respond(InteractionCallback::update(content)).await
    }
    pub async fn ack(&self) -> Result<()> {
        self.respond(InteractionCallback::ack()).await
    }
}

/// A small executable cog. Keep stdout exclusively for the protocol; log with eprintln!.
/// Handler futures must finish their API requests before returning (do not detach them).
pub struct WorkerCog {
    manifest: Manifest,
    commands: HashMap<String, Handler>,
    buttons: HashMap<String, Handler>,
    listeners: HashMap<Listener, Handler>,
}
impl WorkerCog {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            manifest: Manifest {
                name: name.into(),
                commands: vec![],
                buttons: vec![],
                listeners: vec![],
            },
            commands: HashMap::new(),
            buttons: HashMap::new(),
            listeners: HashMap::new(),
        }
    }
    pub fn command<F, Fut>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        self.manifest
            .commands
            .push(Command::new(&name, description));
        self.commands
            .insert(name, Arc::new(move |ctx| Box::pin(handler(ctx))));
        self
    }
    pub fn button<F, Fut>(mut self, prefix: impl Into<String>, handler: F) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let prefix = prefix.into();
        self.manifest.buttons.push(prefix.clone());
        self.buttons
            .insert(prefix, Arc::new(move |ctx| Box::pin(handler(ctx))));
        self
    }
    pub fn listen<F, Fut>(mut self, kind: Listener, handler: F) -> Self
    where
        F: Fn(WorkerContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.manifest.listeners.push(kind);
        self.listeners
            .insert(kind, Arc::new(move |ctx| Box::pin(handler(ctx))));
        self
    }
    pub async fn run(self) -> Result<()> {
        self.manifest.validate()?;
        let connection = Arc::new(Mutex::new(Connection {
            input: BufReader::new(tokio::io::stdin()),
            output: tokio::io::stdout(),
            poisoned: false,
        }));
        protocol::write(
            &mut connection.lock().await.output,
            &Wire::Ready {
                protocol: protocol::VERSION,
                manifest: self.manifest,
            },
        )
        .await?;
        loop {
            let event = match protocol::read(&mut connection.lock().await.input).await? {
                Wire::Invoke { event } => *event,
                Wire::Shutdown => return Ok(()),
                _ => return Err(protocol::protocol_error()),
            };
            let handler = match &event {
                CogEvent::Command { command, .. } => self.commands.get(command),
                CogEvent::Button { prefix, .. } => self.buttons.get(prefix),
                CogEvent::Message { .. } => self.listeners.get(&Listener::Message),
                CogEvent::Interaction { .. } => self.listeners.get(&Listener::Interaction),
                CogEvent::MemberJoin { .. } => self.listeners.get(&Listener::MemberJoin),
                CogEvent::MemberLeave { .. } => self.listeners.get(&Listener::MemberLeave),
            };
            let active = Arc::new(AtomicBool::new(true));
            let result = match handler {
                Some(handler) => {
                    handler(WorkerContext {
                        event,
                        connection: connection.clone(),
                        active: active.clone(),
                    })
                    .await
                }
                None => Err(YappinError::Config(
                    "worker has no handler for this event".into(),
                )),
            };
            let mut connection = connection.lock().await;
            active.store(false, Ordering::SeqCst);
            if connection.poisoned {
                return Err(protocol::protocol_error());
            }
            protocol::write(
                &mut connection.output,
                &Wire::Done {
                    error: result.err().map(|e| e.to_string()),
                },
            )
            .await?;
        }
    }
}
