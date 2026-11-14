//! Shared execution with polling-owned watermarks. Disabled by default.
use super::{
    ingress::{Batch, ClaimedEvent},
    pipeline::ExecutionGate,
    Bot, BotHandle,
};
use crate::{BotProfile, EventIngress, EventProcessingFailure, ReceiptStage, Result, YappinError};
use futures_util::{
    future::BoxFuture,
    stream::{self, FuturesOrdered},
    StreamExt,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, Semaphore};
struct RunGuard {
    ingress: EventIngress,
    handle: BotHandle,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    #[cfg(feature = "gateway")]
    responses: crate::gateway::requests::ResponseLease,
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        #[cfg(feature = "gateway")]
        self.responses.shutdown("run_cancelled");
        self.ingress.close();
        self.ingress.abort_unfinished();
        for task in &self.tasks {
            task.abort();
        }
        self.handle.client().events().close();
        self.handle.client().cancel_auto_ack();
    }
}
struct EventGuard {
    ingress: EventIngress,
    id: u64,
    armed: bool,
}
impl Drop for EventGuard {
    fn drop(&mut self) {
        if self.armed {
            self.ingress.interrupted(
                self.id,
                Some(
                    YappinError::HandlerInterrupted {
                        reason: "shared event processing cancelled before confirmed completion"
                            .into(),
                        source: None,
                    }
                    .to_remote(),
                ),
            );
        }
    }
}
fn supervise(
    future: impl std::future::Future<Output = ()> + Send + 'static,
    failures: mpsc::UnboundedSender<YappinError>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = super::failure::catch(async {
            future.await;
            Ok(())
        })
        .await
        {
            let _ = failures.send(error);
        }
    })
}
struct Completed {
    watermark: Option<u64>,
    outcomes: Vec<(u64, Option<Result<()>>)>,
}
impl Bot {
    pub(super) async fn run_shared_pipeline(
        &self,
        me: &BotProfile,
        last: u64,
        ingress: EventIngress,
    ) -> Result<()> {
        self.client().events().reopen();
        let capacity = self.handle.inner.pending_batches;
        let (sender, mut receiver) = mpsc::channel::<Result<Batch>>(capacity);
        let mut guard = RunGuard {
            ingress: ingress.clone(),
            handle: self.handle.clone(),
            tasks: vec![],
            #[cfg(feature = "gateway")]
            responses: Default::default(),
        };
        let mut recovery = ingress.open(
            sender.clone(),
            last,
            self.handle.inner.client.clone(),
            self.handle.inner.auto_ack_after,
        )?;
        for claimed in &mut recovery {
            ingress
                .transition(&claimed.event, ReceiptStage::Started, None)
                .await?;
            claimed.prepared = ingress.prepare_owner(&claimed.event).await?;
        }
        let gate = Arc::new(ExecutionGate {
            limit: self.handle.inner.handler_concurrency,
            semaphore: Semaphore::new(self.handle.inner.handler_concurrency),
            serial: tokio::sync::Mutex::new(()),
            halted: AtomicBool::new(false),
            failed: tokio::sync::Notify::new(),
        });
        let mut batches: FuturesOrdered<BoxFuture<'_, Completed>> = FuturesOrdered::new();
        if !recovery.is_empty() {
            batches.push_back(Box::pin(self.dispatch_shared_batch(
                Batch {
                    watermark: None,
                    events: recovery,
                },
                me,
                gate.clone(),
                ingress.clone(),
            )));
        }
        let (supervised, mut intake_failures) = mpsc::unbounded_channel();
        let bot = self.handle.clone();
        let intake = ingress.clone();
        let poll_sender = sender.clone();
        guard.tasks.push(supervise(
            async move {
                let mut after = last;
                let mut failures = 0u32;
                loop {
                    if poll_sender.is_closed() {
                        break;
                    }
                    let mut episode = bot.read_availability_policy().map(|policy| {
                        crate::availability::Episode::new(
                            bot.clone(),
                            policy.clone(),
                            crate::AvailabilityPhase::Intake,
                        )
                    });
                    after = intake.recovery_after(after);
                    let response = loop {
                        let read =
                            bot.client()
                                .get_events(Some(after), Some(25), bot.inner.max_age);
                        let result = match &mut episode {
                            Some(e) => e.attempt(read).await,
                            None => read.await,
                        };
                        match result {
                            Err(error) => {
                                if let Some(e) = &mut episode {
                                    match e.retry(error).await {
                                        Ok(()) => continue,
                                        Err(error) => break Err(error),
                                    }
                                } else if !error.is_unauthorized()
                                    && super::pipeline::retryable_read(&error)
                                {
                                    failures = failures.saturating_add(1);
                                    tokio::time::sleep(std::time::Duration::from_secs(
                                        (1u64 << failures.min(5)).min(30),
                                    ))
                                    .await;
                                    continue;
                                } else {
                                    break Err(error);
                                }
                            }
                            value => break value,
                        }
                    };
                    if !bot.inner.running.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Err(error) = &response {
                        intake.fatal(error.to_remote());
                        break;
                    }
                    if let Ok(response) = &response {
                        if let Err(error) = intake.raw_pending(&response.events) {
                            intake.fatal(error.to_remote());
                            break;
                        }
                    }
                    if response.is_ok() {
                        if let Some(mut episode) = episode.take() {
                            episode.success();
                        }
                    }
                    // A long poll must not monopolize the only admission slot.
                    // Hold at most one raw response; reserve before any claim or
                    // intake effects, so gateway delivery can proceed meanwhile.
                    let Ok(permit) = poll_sender.reserve().await else {
                        break;
                    };
                    if !bot.inner.running.load(Ordering::SeqCst) {
                        break;
                    }
                    let result: Result<Batch> = async {
                        let response = response?;
                        failures = 0;
                        let watermark = after.max(response.last);
                        if response.events.iter().any(|event| event.id > watermark) {
                            return Err(YappinError::validation(
                                "ingress.watermark",
                                "polling event exceeds the authoritative watermark",
                            ));
                        }
                        let mut events = vec![];
                        // Compare same-batch duplicates canonically before claiming
                        // any event, so a contradiction precedes all intake effects.
                        let unique = intake.polling_batch(response.events).await?;
                        intake
                            .confirm_retirement_coverage(&unique, watermark)
                            .await?;
                        for event in unique {
                            events.push(intake.claim_polling(event).await?.ok_or_else(|| {
                                YappinError::Config(
                                    "shared ingress claim capacity exhausted during polling".into(),
                                )
                            })?);
                        }
                        after = watermark;
                        intake.watermark(watermark);
                        bot.inner.telemetry.update(|s| {
                            s.received_cursor = watermark;
                            if s.mode == crate::TransportMode::Polling || s.active.is_none() {
                                s.active = Some(crate::EventTransport::Polling);
                            }
                        });
                        Ok(Batch {
                            watermark: Some(watermark),
                            events,
                        })
                    }
                    .await;
                    if let Err(error) = &result {
                        intake.fatal(error.to_remote());
                    }
                    let failed = result.is_err();
                    let owned: Vec<_> = result
                        .as_ref()
                        .ok()
                        .map(|b| {
                            b.events
                                .iter()
                                .filter(|e| e.owner)
                                .map(|e| e.event.id)
                                .collect()
                        })
                        .unwrap_or_default();
                    permit.send(result);
                    intake.publish(&owned);
                    if !failed {
                        intake.raw_published();
                    }
                    if failed {
                        break;
                    }
                    if intake.recovery_after(after) < after {
                        // Re-read only the authoritative unresolved range, bounded
                        // by the conflict deadline. Never re-execute completed work.
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            },
            supervised.clone(),
        ));
        #[cfg(feature = "gateway")]
        if self.transport_status().mode == crate::TransportMode::GatewayPreferred {
            let bot = self.handle.clone();
            let input = ingress.clone();
            let tx = sender.clone();
            let response_lease = guard.responses.downgrade();
            guard.tasks.push(supervise(async move {
                let mut gateway = crate::gateway::Gateway::new(bot.client().clone())
                    .url(bot.inner.gateway_url.clone().unwrap_or_else(|| crate::gateway::DEFAULT_GATEWAY.into()))
                    .max_age(bot.inner.max_age).telemetry(bot.inner.telemetry.clone()).response_lease(response_lease)
                    .without_fallback().shared_delivery();
                let mut after = last;
                let mut failures = 0u32;
                loop {
                    // Polling already supplies fallback and checkpoint authority.
                    // One read attempt keeps this source's retry ownership explicit.
                    match crate::client::single_attempt(gateway.poll(after)).await {
                        Ok(batch) => {
                            failures = 0;
                            after = after.max(batch.last);
                            for event in batch.events {
                                if input.polling_only() {continue;}
                                if tx.capacity() == 0 { gateway.backpressure(); }
                                let Ok(permit) = tx.reserve().await else { return; };
                                let result = input.claim(event, super::IngressSource::Gateway).await
                                    .and_then(|claim| claim.ok_or_else(|| YappinError::Config("shared ingress claim capacity exhausted during gateway delivery".into())))
                                    .map(|event| Batch { watermark: None, events: vec![event] });
                                if result.as_ref().err().is_some_and(|error|input.recoverable_conflict(error)) {drop(permit);continue;}
                                let failed = result.is_err();
                                let owned: Vec<_> = result.as_ref().ok()
                                    .map(|batch| batch.events.iter().filter(|event| event.owner).map(|event| event.event.id).collect())
                                    .unwrap_or_default();
                                permit.send(result);
                                input.publish(&owned);
                                if failed { return; }
                            }
                        },
                        Err(error) => {
                            let wait = match error.status_code() {
                                Some(4003) => Some(300),
                                Some(4008 | 4029) => Some(60),
                                _ if crate::availability::retryable_read(&error) => {
                                    failures = failures.saturating_add(1);
                                    Some((1u64 << failures.min(5)).min(30))
                                },
                                _ => None,
                            };
                            if error.is_unauthorized() || wait.is_none() {
                                let _ = tx.send(Err(error)).await;
                                return;
                            }
                            bot.inner.telemetry.update(|status| {
                                status.active = Some(crate::EventTransport::Polling);
                                status.fallback = Some(if error.status_code() == Some(4003) {
                                    crate::FallbackReason::AccessDenied
                                } else if error.to_json()["reason"] == "event_backpressure" {crate::FallbackReason::Backpressure} else { crate::FallbackReason::Unavailable });
                            });
                            tokio::time::sleep(std::time::Duration::from_secs(wait.unwrap())).await;
                        }
                    }
                }
            }, supervised.clone()));
        }
        drop(sender);
        let mut report = EventProcessingFailure {
            committed_cursor: last,
            received_cursor: last,
            failed_events: vec![],
            unprocessed_events: vec![],
            completed_uncommitted_events: vec![],
            intake_error: None,
            cursor_error: None,
        };
        let mut stopping = false;
        let mut drain_deadline = None;
        let _draining = super::drain::Guard(self.handle.clone());
        let mut barrier = false;
        let mut polling_prefix = last;
        self.handle.inner.telemetry.update(|s| {
            s.received_cursor = last;
            s.committed_cursor = last;
        });
        loop {
            if stopping {
                self.handle.inner.draining.store(true, Ordering::SeqCst);
                if drain_deadline.is_none() {
                    drain_deadline = self
                        .handle
                        .inner
                        .drain_timeout
                        .map(|limit| tokio::time::Instant::now() + limit);
                }
                ingress.close();
                self.client().events().close();
                for task in &guard.tasks {
                    task.abort();
                }
                ingress.cancel_unpublished();
                receiver.close();
                if batches.is_empty() {
                    break;
                }
            }
            tokio::select! {
                _=async {if let Some(deadline)=drain_deadline {tokio::time::sleep_until(deadline).await;}else{std::future::pending::<()>().await;}},if stopping=>{
                    let cause=super::drain::timeout(self.handle.inner.drain_timeout.expect("drain deadline"));
                    ingress.interrupt_unfinished(cause.to_remote());super::drain::attach(&mut report,cause);
                    self.client().cancel_auto_ack();
                    #[cfg(feature="gateway")] guard.responses.shutdown("drain_timeout");
                    drop(std::mem::take(&mut batches));break;
                },
                error=ingress.failure(),if !stopping=>{report.intake_error=Some(self.failure_cause(&error));gate.halt();stopping=true;},
                Some(error)=intake_failures.recv(),if !stopping=>{report.intake_error=Some(self.failure_cause(&error));gate.halt();stopping=true;},
                _=tokio::signal::ctrl_c(),if !stopping=>{self.stop();stopping=true;},
                _=self.handle.inner.stopped.notified(),if !stopping=>{stopping=true;},
                _=gate.failed.notified(),if !stopping=>{stopping=true;},
                batch=receiver.recv(),if !stopping&&batches.len()<capacity=>{
                    match batch {
                        Some(Ok(batch))=>batches.push_back(Box::pin(self.dispatch_shared_batch(batch,me,gate.clone(),ingress.clone()))),
                        Some(Err(error))=>{report.intake_error=Some(self.failure_cause(&error));gate.halt();stopping=true;},
                        None=>{report.intake_error=Some(YappinError::Config("shared ingress intake stopped".into()).to_remote());gate.halt();stopping=true;},
                    }
                },
                completed=batches.next(),if !batches.is_empty()=>{if let Some(batch)=completed{stopping|=self.checkpoint_shared(batch,&ingress,&mut report,&mut barrier,&mut polling_prefix,&gate).await;}}
            }
            self.handle
                .inner
                .telemetry
                .update(|s| s.pending_batches = batches.len());
            if !self.handle.inner.running.load(Ordering::SeqCst) {
                stopping = true;
            }
        }
        self.client().cancel_auto_ack();
        #[cfg(feature = "gateway")]
        guard.responses.shutdown("pipeline_shutdown");
        for task in &mut guard.tasks {
            task.abort();
            let _ = task.await;
        }
        while let Ok(batch) = receiver.try_recv() {
            match batch {
                Ok(batch) => {
                    for event in batch.events {
                        if event.owner {
                            ingress.interrupted(event.event.id, None);
                        }
                    }
                }
                Err(error) => {
                    report
                        .intake_error
                        .get_or_insert_with(|| self.failure_cause(&error));
                }
            }
        }
        ingress.add_evidence(&mut report);
        report.unprocessed_events.retain(|id| {
            !report.failed_events.iter().any(|e| e.event_id == *id)
                && !report.completed_uncommitted_events.contains(id)
                && *id > report.committed_cursor
        });
        report.received_cursor = ingress.status().polling_watermark;
        report.unprocessed_events.sort_unstable();
        report.unprocessed_events.dedup();
        report.completed_uncommitted_events.sort_unstable();
        report.completed_uncommitted_events.dedup();
        self.handle.inner.telemetry.update(|s| {
            s.active = None;
            s.pending_batches = 0;
            s.heartbeat_rtt = None;
        });
        if report.failed_events.is_empty()
            && report.intake_error.is_none()
            && report.cursor_error.is_none()
        {
            Ok(())
        } else {
            Err(YappinError::EventProcessing {
                report: Box::new(report),
            })
        }
    }
    async fn dispatch_shared_batch(
        &self,
        batch: Batch,
        me: &BotProfile,
        gate: Arc<ExecutionGate>,
        ingress: EventIngress,
    ) -> Completed {
        let mut outcomes = stream::iter(batch.events)
            .map(|mut claimed: ClaimedEvent| {
                let gate = gate.clone();
                let ingress = ingress.clone();
                async move {
                    let id = claimed.event.id;
                    if !claimed.owner {
                        if ingress.retired(id) {
                            return (id, Some(Ok(())));
                        }
                        // Waiting for a claim never owns an execution permit. Its owner
                        // can be queued in a later batch (e.g. slow durable admission).
                        let status = claimed.receipt.wait().await;
                        return (
                            id,
                            match status.stage {
                                ReceiptStage::Completed => Some(Ok(())),
                                ReceiptStage::Failed => Some(Err(status
                                    .error
                                    .map(YappinError::Remote)
                                    .unwrap_or_else(|| {
                                        YappinError::Config(
                                            "shared event failed without completion".into(),
                                        )
                                    }))),
                                _ => None,
                            },
                        );
                    }
                    ingress.publish(&[id]);
                    if gate.halted() {
                        ingress.interrupted(id, None);
                        return (id, None);
                    }
                    let permit = gate.semaphore.acquire().await;
                    if gate.halted() {
                        ingress.interrupted(id, None);
                        return (id, None);
                    }
                    let mut guard = EventGuard {
                        ingress: ingress.clone(),
                        id,
                        armed: true,
                    };
                    let result = match permit {
                        Ok(_permit) => {
                            super::failure::catch(async {
                                self.dispatch_event(
                                    claimed.event.clone(),
                                    me,
                                    claimed.prepared.take(),
                                )
                                .await
                            })
                            .await
                        }
                        Err(_) => Err(YappinError::Config("shared executor closed".into())),
                    };
                    let stage = if result.is_ok() {
                        ReceiptStage::Completed
                    } else {
                        ReceiptStage::Failed
                    };
                    let cause = result.as_ref().err().map(|e| self.failure_cause(e));
                    let result =
                        match ingress.transition(&claimed.event, stage, cause).await {
                            Ok(()) => result,
                            Err(storage) => {
                                let original = result.err().unwrap_or_else(|| {
                                    YappinError::HandlerInterrupted {
                                    reason:
                                        "handler completed but durable completion receipt failed"
                                            .into(),
                                    source: None,
                                }
                                });
                                let error = YappinError::FailureHandling {
                                    source: Box::new(original),
                                    handling_error: Box::new(storage),
                                };
                                ingress.interrupted(id, Some(self.failure_cause(&error)));
                                Err(error)
                            }
                        };
                    guard.armed = false;
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
        Completed {
            watermark: batch.watermark,
            outcomes,
        }
    }
    async fn checkpoint_shared(
        &self,
        batch: Completed,
        ingress: &EventIngress,
        report: &mut EventProcessingFailure,
        barrier: &mut bool,
        polling_prefix: &mut u64,
        gate: &ExecutionGate,
    ) -> bool {
        let mut prefix = report.committed_cursor;
        let mut complete = !*barrier;
        let mut stop = false;
        for (id, result) in batch.outcomes {
            match result {
                Some(Ok(())) => {
                    if !ingress.retired(id) {
                        report.completed_uncommitted_events.push(id);
                    }
                    if complete && batch.watermark.is_some() {
                        prefix = prefix.max(id);
                    }
                }
                Some(Err(error)) => {
                    if !report.failed_events.iter().any(|e| e.event_id == id) {
                        report.failed_events.push(super::EventFailure {
                            event_id: id,
                            error: self.failure_cause(&error),
                        });
                    }
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
            if let Some(watermark) = batch.watermark {
                prefix = prefix.max(watermark);
            }
        }
        if !*barrier {
            *polling_prefix = (*polling_prefix).max(prefix);
        }
        if !*barrier && *polling_prefix > report.committed_cursor {
            let saved = ingress
                .checkpoint(*polling_prefix, self.handle.inner.cursor.as_ref())
                .await;
            match saved {
                Ok(saved) => {
                    report.committed_cursor = saved;
                    self.handle
                        .inner
                        .telemetry
                        .update(|s| s.committed_cursor = saved);
                }
                Err(error) => {
                    report.cursor_error = Some(self.failure_cause(&error));
                    gate.halt();
                    stop = true;
                    complete = false;
                }
            }
        }
        report
            .completed_uncommitted_events
            .retain(|id| *id > report.committed_cursor);
        if !complete {
            *barrier = true;
        }
        stop
    }
}
