use super::Bot;
use crate::{
    diagnostics::{EventTransport, TransportMode},
    BotProfile, Result, YappinError,
};
use futures_util::{
    future::BoxFuture,
    stream::{self, FuturesOrdered},
    StreamExt,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::sync::{mpsc, Semaphore};

struct Intake(
    tokio::task::JoinHandle<()>,
    crate::Client,
    crate::diagnostics::Telemetry,
);
struct PreparedBatch {
    last: u64,
    events: Vec<(crate::Event, Option<Box<crate::InteractionEvent>>)>,
}
impl Drop for Intake {
    fn drop(&mut self) {
        self.0.abort();
        self.1.events().close();
        self.1.cancel_auto_ack();
        self.2.update(|s| {
            s.active = None;
            s.heartbeat_rtt = None;
            s.pending_batches = 0;
        });
    }
}

impl Bot {
    pub(super) async fn run_pipeline(&self, me: &BotProfile, last: u64) -> Result<()> {
        let mode = self.handle.inner.telemetry.status().mode;
        self.client().events().reopen();
        let client = self.client().clone();
        let telemetry = self.handle.inner.telemetry.clone();
        let max_age = self.handle.inner.max_age;
        let capacity = self.handle.inner.pending_batches;
        let auto_ack = self.handle.inner.auto_ack_after;
        let (sender, mut receiver) = mpsc::channel::<Result<PreparedBatch>>(capacity);
        #[cfg(feature = "gateway")]
        let gateway_url = self
            .handle
            .inner
            .gateway_url
            .clone()
            .unwrap_or_else(|| crate::gateway::DEFAULT_GATEWAY.into());
        telemetry.update(|s| {
            s.received_cursor = last;
            s.committed_cursor = last;
        });
        let intake = Intake(
            tokio::spawn(async move {
                let mut received = last;
                let mut failures = 0u32;
                #[cfg(feature = "gateway")]
                let mut gateway = if mode == TransportMode::Polling {
                    None
                } else {
                    let gateway = crate::gateway::Gateway::new(client.clone())
                        .url(gateway_url)
                        .max_age(max_age)
                        .telemetry(telemetry.clone());
                    Some(if mode == TransportMode::GatewayOnly {
                        gateway.gateway_only()
                    } else {
                        gateway
                    })
                };
                loop {
                    // Drop a full gateway connection instead of leaving its heartbeat unattended.
                    // Resume from the last received cursor once bounded queue capacity returns.
                    #[cfg(feature = "gateway")]
                    if sender.capacity() == 0 {
                        if let Some(gateway) = &mut gateway {
                            gateway.backpressure();
                        }
                    }
                    let Ok(permit) = sender.reserve().await else {
                        break;
                    };
                    let response = async {
                        #[cfg(feature = "gateway")]
                        if let Some(gateway) = &mut gateway {
                            return gateway.poll(received).await;
                        }
                        telemetry.update(|s| {
                            s.active = Some(EventTransport::Polling);
                            s.heartbeat_rtt = None;
                        });
                        client.get_events(Some(received), Some(25), max_age).await
                    }
                    .await;
                    match response {
                        Ok(mut batch) => {
                            failures = 0;
                            batch.events.retain(|event| event.id > received);
                            batch.events.sort_by_key(|event| event.id);
                            batch.events.dedup_by_key(|event| event.id);
                            received = received
                                .max(batch.last)
                                .max(batch.events.last().map_or(0, |e| e.id));
                            batch.last = received;
                            let mut events = Vec::with_capacity(batch.events.len());
                            for event in batch.events {
                                let mut prepared = None;
                                if event.kind == "interaction" {
                                    if let Ok(crate::EventData::Interaction(mut interaction)) =
                                        event.parse_data()
                                    {
                                        interaction.created_at =
                                            interaction.created_at.or(event.created_at);
                                        if let Err(error) =
                                            client.track_interaction(&interaction).await
                                        {
                                            permit.send(Err(error));
                                            return;
                                        }
                                        if let Some(after) = auto_ack {
                                            client.auto_ack(interaction.clone(), after);
                                        }
                                        prepared = Some(Box::new(interaction));
                                    }
                                }
                                client.receive_event(event.clone()).await;
                                events.push((event, prepared));
                            }
                            telemetry.update(|s| s.received_cursor = received);
                            permit.send(Ok(PreparedBatch {
                                last: batch.last,
                                events,
                            }));
                        }
                        Err(error) => {
                            if error.is_unauthorized() || mode == TransportMode::GatewayOnly {
                                permit.send(Err(error));
                                break;
                            }
                            drop(permit);
                            failures = failures.saturating_add(1);
                            tracing::warn!("event intake failed: {error}");
                            tokio::time::sleep(Duration::from_secs(
                                (1u64 << failures.min(5)).min(30),
                            ))
                            .await;
                        }
                    }
                }
            }),
            self.client().clone(),
            self.handle.inner.telemetry.clone(),
        );
        let semaphore = Arc::new(Semaphore::new(self.handle.inner.handler_concurrency));
        let serial_batches = Arc::new(tokio::sync::Mutex::new(()));
        let mut batches: FuturesOrdered<BoxFuture<'_, (u64, Result<()>)>> = FuturesOrdered::new();
        let mut stopping = !self.handle.inner.running.load(Ordering::SeqCst);
        let mut failure = None;
        let mut committed = last;
        loop {
            if stopping {
                intake.0.abort();
                receiver.close();
                self.client().events().close();
                if batches.is_empty() {
                    break;
                }
            }
            tokio::select! {
                _=tokio::signal::ctrl_c(), if !stopping => {self.stop();stopping=true;},
                _=self.handle.inner.stopped.notified(), if !stopping => {stopping=true;},
                next=receiver.recv(), if !stopping && batches.len()<capacity => {
                    match next {
                        Some(Ok(batch))=>{
                            let semaphore=semaphore.clone();
                            let limit=self.handle.inner.handler_concurrency;
                            let serial_batches=serial_batches.clone();
                            batches.push_back(Box::pin(async move {
                                let _ordered = if limit==1 {Some(serial_batches.lock().await)} else {None};
                                let results=stream::iter(batch.events).map(|(event, prepared)|{
                                    let semaphore=semaphore.clone();
                                    async move {
                                        let _permit=semaphore.acquire().await.map_err(|_|YappinError::Config("handler executor closed".into()))?;
                                        self.dispatch_event(event,me,prepared).await
                                    }
                                }).buffer_unordered(limit).collect::<Vec<_>>().await;
                                let result=results.into_iter().collect::<Result<Vec<_>>>().map(|_|());
                                (batch.last,result)
                            }));
                            self.handle.inner.telemetry.update(|s|s.pending_batches=batches.len());
                        }
                        Some(Err(error))=>{failure=Some(error);stopping=true;},
                        None=>{failure=Some(YappinError::Config("event intake exited unexpectedly".into()));stopping=true;},
                    }
                },
                completed=batches.next(), if !batches.is_empty()=>{
                    if let Some((watermark,result))=completed {
                        match result {
                            Ok(()) if failure.is_none()=>{
                                if watermark != committed {
                                    if let Some(cursor)=&self.handle.inner.cursor {
                                        if let Err(error)=cursor.save(watermark).await{failure=Some(error);stopping=true;}
                                    }
                                }
                                if failure.is_none(){committed=watermark;self.handle.inner.telemetry.update(|s|s.committed_cursor=watermark);}
                            }
                            Err(error)=>{if failure.is_none(){failure=Some(error);}stopping=true;},
                            _=>(),
                        }
                        self.handle.inner.telemetry.update(|s|s.pending_batches=batches.len());
                    }
                    if !self.handle.inner.running.load(Ordering::SeqCst){stopping=true;}
                }
            }
        }
        self.handle.inner.telemetry.update(|s| {
            s.active = None;
            s.heartbeat_rtt = None;
            s.pending_batches = 0;
        });
        failure.map_or(Ok(()), Err)
    }
}
