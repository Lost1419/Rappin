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
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
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
struct IntakeFailure {
    error: YappinError,
    unprocessed: Vec<u64>,
}
pub(super) fn retryable_read(error: &YappinError) -> bool {
    matches!(
        error,
        YappinError::Http(_) | YappinError::RateLimited { .. }
    ) || matches!(error.status_code(), Some(408 | 429 | 500 | 502 | 503 | 504))
}
struct CompletedBatch {
    watermark: u64,
    outcomes: Vec<(u64, Option<Result<()>>)>,
}
pub(super) struct ExecutionGate {
    pub(super) limit: usize,
    pub(super) semaphore: Semaphore,
    pub(super) serial: tokio::sync::Mutex<()>,
    pub(super) halted: AtomicBool,
    pub(super) failed: tokio::sync::Notify,
}
impl ExecutionGate {
    pub(super) fn halt(&self) {
        if !self.halted.swap(true, Ordering::SeqCst) {
            self.failed.notify_one();
        }
    }
    pub(super) fn halted(&self) -> bool {
        self.halted.load(Ordering::SeqCst)
    }
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
        if let Some(ingress) = &self.handle.inner.ingress {
            return self.run_shared_pipeline(me, last, ingress.clone()).await;
        }
        let mode = self.handle.inner.telemetry.status().mode;
        self.client().events().reopen();
        let client = self.client().clone();
        let telemetry = self.handle.inner.telemetry.clone();
        let max_age = self.handle.inner.max_age;
        let capacity = self.handle.inner.pending_batches;
        let auto_ack = self.handle.inner.auto_ack_after;
        let availability = self.handle.inner.availability_policy.clone();
        let availability_handle = self.handle.clone();
        let (sender, mut receiver) =
            mpsc::channel::<std::result::Result<PreparedBatch, IntakeFailure>>(capacity);
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
        let mut intake = Intake(
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
                    let mut episode = availability.as_ref().map(|policy| {
                        crate::availability::Episode::new(
                            availability_handle.clone(),
                            policy.clone(),
                            crate::AvailabilityPhase::Intake,
                        )
                    });
                    // Keep the read's reserved slot through recovery. Queue admission
                    // waits are backpressure, not time spent in an availability outage.
                    let response = loop {
                        let read = async {
                            #[cfg(feature = "gateway")]
                            if let Some(gateway) = &mut gateway {
                                return gateway.poll(received).await;
                            }
                            telemetry.update(|s| {
                                s.active = Some(EventTransport::Polling);
                                s.heartbeat_rtt = None;
                            });
                            client.get_events(Some(received), Some(25), max_age).await
                        };
                        let result = match &mut episode {
                            Some(episode) => episode.attempt(read).await,
                            None => read.await,
                        };
                        match result {
                            Err(error) => match &mut episode {
                                Some(episode) => match episode.retry(error).await {
                                    Ok(()) => continue,
                                    Err(error) => break Err(error),
                                },
                                None => break Err(error),
                            },
                            value => break value,
                        }
                    };
                    match response {
                        Ok(mut batch) => {
                            if let Some(mut episode) = episode.take() {
                                episode.success();
                            }
                            failures = 0;
                            batch.events.retain(|event| event.id > received);
                            batch.events.sort_by_key(|event| event.id);
                            batch.events.dedup_by_key(|event| event.id);
                            received = received
                                .max(batch.last)
                                .max(batch.events.last().map_or(0, |e| e.id));
                            batch.last = received;
                            telemetry.update(|s| s.received_cursor = received);
                            let mut events: Vec<(
                                crate::Event,
                                Option<Box<crate::InteractionEvent>>,
                            )> = Vec::with_capacity(batch.events.len());
                            let mut pending = batch.events.into_iter();
                            while let Some(event) = pending.next() {
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
                                            let mut unprocessed: Vec<_> =
                                                events.iter().map(|(event, _)| event.id).collect();
                                            unprocessed.push(event.id);
                                            unprocessed.extend(pending.map(|event| event.id));
                                            permit.send(Err(IntakeFailure { error, unprocessed }));
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
                            permit.send(Ok(PreparedBatch {
                                last: batch.last,
                                events,
                            }));
                        }
                        Err(error) => {
                            if availability.is_some() {
                                if !availability_handle.inner.running.load(Ordering::SeqCst) {
                                    break;
                                }
                                permit.send(Err(IntakeFailure {
                                    error,
                                    unprocessed: vec![],
                                }));
                                break;
                            }
                            if error.is_unauthorized()
                                || mode == TransportMode::GatewayOnly
                                || !retryable_read(&error)
                            {
                                permit.send(Err(IntakeFailure {
                                    error,
                                    unprocessed: vec![],
                                }));
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
        let gate = Arc::new(ExecutionGate {
            limit: self.handle.inner.handler_concurrency,
            semaphore: Semaphore::new(self.handle.inner.handler_concurrency),
            serial: tokio::sync::Mutex::new(()),
            halted: AtomicBool::new(false),
            failed: tokio::sync::Notify::new(),
        });
        let mut batches: FuturesOrdered<BoxFuture<'_, CompletedBatch>> = FuturesOrdered::new();
        let mut stopping = !self.handle.inner.running.load(Ordering::SeqCst);
        let mut barrier = false;
        let mut report = super::EventProcessingFailure {
            committed_cursor: last,
            received_cursor: last,
            failed_events: vec![],
            unprocessed_events: vec![],
            completed_uncommitted_events: vec![],
            intake_error: None,
            cursor_error: None,
        };
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
                _=gate.failed.notified(), if !stopping => {stopping=true;},
                next=receiver.recv(), if !stopping && batches.len()<capacity => {
                    match next {
                        Some(Ok(batch))=>{
                            batches.push_back(Box::pin(self.dispatch_batch(batch,me,gate.clone())));
                            self.handle.inner.telemetry.update(|s|s.pending_batches=batches.len());
                        }
                        Some(Err(failure))=>{
                            report.intake_error=Some(self.failure_cause(&failure.error));
                            report.unprocessed_events.extend(failure.unprocessed);
                            gate.halt();stopping=true;
                        }
                        None=>{
                            if !self.handle.inner.running.load(Ordering::SeqCst) { stopping=true; continue; }
                            let error=YappinError::Config("event intake exited unexpectedly".into());
                            report.intake_error=Some(self.failure_cause(&error));
                            gate.halt();stopping=true;
                        }
                    }
                },
                completed=batches.next(), if !batches.is_empty()=>{
                    if let Some(batch)=completed {
                        stopping|=self.checkpoint_batch(batch,&mut report,&mut barrier,&gate).await;
                        self.handle.inner.telemetry.update(|s|s.pending_batches=batches.len());
                    }
                    if !self.handle.inner.running.load(Ordering::SeqCst){stopping=true;}
                }
            }
        }
        // These batches were prefetched but never accepted by the executor.
        intake.0.abort();
        let _ = (&mut intake.0).await;
        while let Ok(batch) = receiver.try_recv() {
            match batch {
                Ok(batch) => report
                    .unprocessed_events
                    .extend(batch.events.into_iter().map(|(event, _)| event.id)),
                Err(failure) => {
                    if report.intake_error.is_none() {
                        report.intake_error = Some(self.failure_cause(&failure.error));
                    }
                    report.unprocessed_events.extend(failure.unprocessed);
                }
            }
        }
        report.received_cursor = self.handle.inner.telemetry.status().received_cursor;
        report.completed_uncommitted_events.sort_unstable();
        report.unprocessed_events.sort_unstable();
        report.unprocessed_events.dedup();
        report.failed_events.sort_by_key(|event| event.event_id);
        self.handle.inner.telemetry.update(|s| {
            s.active = None;
            s.heartbeat_rtt = None;
            s.pending_batches = 0;
        });
        if report.failed_events.is_empty()
            && report.cursor_error.is_none()
            && report.intake_error.is_none()
        {
            Ok(())
        } else {
            Err(YappinError::EventProcessing {
                report: Box::new(report),
            })
        }
    }

    async fn dispatch_batch(
        &self,
        batch: PreparedBatch,
        me: &BotProfile,
        gate: Arc<ExecutionGate>,
    ) -> CompletedBatch {
        let _ordered = if gate.limit == 1 {
            Some(gate.serial.lock().await)
        } else {
            None
        };
        let mut outcomes = stream::iter(batch.events)
            .map(|(event, prepared)| {
                let gate = gate.clone();
                async move {
                    let id = event.id;
                    if gate.halted() {
                        return (id, None);
                    }
                    let permit = gate.semaphore.acquire().await;
                    if gate.halted() {
                        return (id, None);
                    }
                    let result = match permit {
                        Ok(_permit) => {
                            super::failure::catch(self.dispatch_event(event, me, prepared)).await
                        }
                        Err(_) => Err(YappinError::Config("handler executor closed".into())),
                    };
                    if result.is_err() {
                        gate.halt();
                    }
                    (id, Some(result))
                }
            })
            .buffer_unordered(gate.limit)
            .collect::<Vec<_>>()
            .await;
        outcomes.sort_by_key(|(id, _)| *id);
        CompletedBatch {
            watermark: batch.last,
            outcomes,
        }
    }

    async fn checkpoint_batch(
        &self,
        batch: CompletedBatch,
        report: &mut super::EventProcessingFailure,
        barrier: &mut bool,
        gate: &ExecutionGate,
    ) -> bool {
        let mut prefix = report.committed_cursor;
        let mut complete = !*barrier;
        let mut stop = false;
        for (id, result) in batch.outcomes {
            match result {
                Some(Ok(())) => {
                    report.completed_uncommitted_events.push(id);
                    if complete {
                        prefix = id;
                    }
                }
                Some(Err(error)) => {
                    report.failed_events.push(super::EventFailure {
                        event_id: id,
                        error: self.failure_cause(&error),
                    });
                    complete = false;
                    stop = true;
                }
                None => {
                    report.unprocessed_events.push(id);
                    complete = false;
                    stop = true;
                }
            }
        }
        if complete {
            prefix = prefix.max(batch.watermark);
        }
        if !*barrier && prefix != report.committed_cursor {
            let saved = match &self.handle.inner.cursor {
                Some(cursor) => cursor.save(prefix).await,
                None => Ok(()),
            };
            match saved {
                Ok(()) => {
                    report.committed_cursor = prefix;
                    self.handle
                        .inner
                        .telemetry
                        .update(|s| s.committed_cursor = prefix);
                }
                Err(error) => {
                    report.cursor_error = Some(self.failure_cause(&error));
                    gate.halt();
                    stop = true;
                    complete = false;
                }
            }
        }
        // Healthy runs must not retain a history of every completed event.
        report
            .completed_uncommitted_events
            .retain(|id| *id > report.committed_cursor);
        if !complete {
            *barrier = true;
        }
        stop
    }
}
