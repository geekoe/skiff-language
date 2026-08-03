//! Task control plane owner: pending-attempt correlation, terminal
//! settlement through [`TaskStore`], lease bookkeeping through the
//! scheduler, and the ordinary request-deadline sweep.
//!
//! The dispatcher reports task-attempt terminals through the injected
//! [`TaskAttemptTerminalSink`]; this owner maps definite outcomes to
//! `TaskStore.settle` and uncertain outcomes to lease-loss recovery (no
//! settlement, no release; the scheduler stops renewing so lease expiry is
//! the store-authority arbiter).

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};
use skiff_runtime_transport::protocol::TelemetryLevel;
use skiff_task_control::model::{
    DurableUtcTimestamp, LeaseId, TaskId, TaskOutcome, TaskTerminal,
};
use skiff_task_control::scheduler::Scheduler;
use skiff_task_control::store::{ReleaseInput, SettleInput, SettleOutcome, TaskStore};

use crate::dispatch::{
    RequestDispatcher, TaskAttemptTerminalOutcome, TaskAttemptTerminalSink,
};
use crate::ws::Clock;
use crate::telemetry::{task_event, TaskTelemetrySink};

use super::actor_attempt::{ActorAttemptTerminal, ActorAttemptTerminalSink};
use super::health::TaskControlCounters;

/// One pending task-attempt correlation: the ordinary `request.start` frame
/// is keyed by its transport `request_id`; the durable task facts stay with
/// the task store / scheduler.
#[derive(Debug, Clone)]
struct PendingAttempt {
    /// Local request deadline (ordinary full budget; not the task lease).
    deadline_ms: u64,
}

#[derive(Debug, Clone)]
enum ControlEvent {
    Settle {
        request_id: String,
        task_id: TaskId,
        lease_id: LeaseId,
        outcome: TaskOutcome,
    },
    Release {
        request_id: String,
        task_id: TaskId,
        lease_id: LeaseId,
        retry_after_ms: u64,
    },
}

/// Durable task settlement / deadline owner (authoritative design "Runtime
/// Admission And Settlement").
pub struct DurableTaskControl {
    store: Arc<dyn TaskStore>,
    /// Deferred scheduler handle (assembled after this control plane; the
    /// admission seam cannot create attempts before the scheduler exists).
    scheduler: Arc<Mutex<Option<Arc<Scheduler>>>>,
    /// Deferred dispatcher handle: the scheduler/admission are assembled
    /// before the dispatcher exists; the composition installs it before any
    /// listener starts, and the deadline sweep no-ops until then.
    dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
    clock: Arc<dyn Clock>,
    counters: Arc<TaskControlCounters>,
    telemetry: Arc<dyn TaskTelemetrySink>,
    pending: Mutex<HashMap<String, PendingAttempt>>,
    events: tokio::sync::mpsc::Sender<ControlEvent>,
    events_rx: Mutex<Option<tokio::sync::mpsc::Receiver<ControlEvent>>>,
    sweep_interval: Duration,
    started: AtomicBool,
}

impl fmt::Debug for DurableTaskControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableTaskControl")
            .field("pending", &self.pending.lock().map(|guard| guard.len()).unwrap_or(0))
            .finish()
    }
}

impl DurableTaskControl {
    pub fn new(
        store: Arc<dyn TaskStore>,
        scheduler: Arc<Mutex<Option<Arc<Scheduler>>>>,
        dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
        clock: Arc<dyn Clock>,
        counters: Arc<TaskControlCounters>,
        telemetry: Arc<dyn TaskTelemetrySink>,
        sweep_interval: Duration,
    ) -> Self {
        let (events, events_rx) = tokio::sync::mpsc::channel(4096);
        Self {
            store,
            scheduler,
            dispatcher,
            clock,
            counters,
            telemetry,
            pending: Mutex::new(HashMap::new()),
            events,
            events_rx: Mutex::new(Some(events_rx)),
            sweep_interval,
            started: AtomicBool::new(false),
        }
    }

    /// Starts the settlement worker + deadline sweep task. Exactly once per
    /// control plane instance.
    pub fn spawn_worker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        if self.started.swap(true, Ordering::SeqCst) {
            panic!("task control worker already started");
        }
        let control = Arc::clone(self);
        tokio::spawn(async move {
            let mut sweep = tokio::time::interval(control.sweep_interval);
            sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut events = control
                .events_rx
                .lock()
                .expect("events receiver lock")
                .take()
                .expect("task control worker started exactly once");
            loop {
                tokio::select! {
                    _ = sweep.tick() => control.sweep_deadlines(),
                    event = events.recv() => match event {
                        Some(event) => control.handle_event(event).await,
                        None => break,
                    },
                }
            }
        })
    }

    /// Registers one accepted attempt so the deadline sweep can enforce the
    /// ordinary request timeout (a definite failed settlement, never lease
    /// extension).
    pub fn track_attempt(
        &self,
        request_id: &str,
        _task_id: &TaskId,
        _lease_id: &LeaseId,
        deadline_ms: u64,
    ) {
        self.pending.lock().expect("pending lock").insert(
            request_id.to_string(),
            PendingAttempt {
                deadline_ms,
            },
        );
    }

    /// Number of attempts this replica is currently correlating.
    pub fn pending_attempt_count(&self) -> usize {
        self.pending.lock().expect("pending lock").len()
    }

    /// Read-only store backlog (observability; store-authority snapshot).
    pub async fn backlog(&self) -> skiff_task_control::store::BacklogObservation {
        self.store
            .observe_backlog()
            .await
            .unwrap_or_default()
    }

    /// Counter bank shared with the sink / admission seam.
    pub fn counters(&self) -> &TaskControlCounters {
        &self.counters
    }

    fn sweep_deadlines(&self) {
        let now = self.clock.now_ms();
        let expired = {
            let mut pending = self.pending.lock().expect("pending lock");
            let ids = pending
                .iter()
                .filter(|(_, attempt)| attempt.deadline_ms <= now)
                .map(|(request_id, _)| request_id.clone())
                .collect::<Vec<_>>();
            for request_id in &ids {
                pending.remove(request_id);
            }
            ids
        };
        for request_id in expired {
            if let Some(dispatcher) = self
                .dispatcher
                .lock()
                .expect("deferred dispatcher lock")
                .clone()
            {
                // Ordinary request timeout: the dispatcher terminal maps to
                // a definite failed settlement (never lease-loss recovery).
                let _ = dispatcher.timeout(&request_id);
            }
        }
    }

    async fn handle_event(&self, event: ControlEvent) {
        match event {
            ControlEvent::Settle {
                request_id,
                task_id,
                lease_id,
                outcome,
            } => {
                let settled_at = self
                    .store
                    .now()
                    .await
                    .unwrap_or(DurableUtcTimestamp::from_millis(
                        i64::try_from(self.clock.now_ms()).unwrap_or(i64::MAX),
                    ));
                let result = self
                    .store
                    .settle(SettleInput {
                        task_id: task_id.clone(),
                        lease_id: lease_id.clone(),
                        terminal: TaskTerminal { settled_at, outcome },
                    })
                    .await;
                self.emit_settle_outcome(&request_id, &task_id, &result);
                self.pending.lock().expect("pending lock").remove(&request_id);
            }
            ControlEvent::Release {
                request_id,
                task_id,
                lease_id,
                retry_after_ms,
            } => {
                self.pending.lock().expect("pending lock").remove(&request_id);
                let now = self
                    .store
                    .now()
                    .await
                    .unwrap_or(DurableUtcTimestamp::from_millis(
                        i64::try_from(self.clock.now_ms()).unwrap_or(i64::MAX),
                    ));
                let retry_ms = i64::try_from(retry_after_ms).unwrap_or(i64::MAX);
                let retry_not_before = now
                    .checked_add_millis(retry_ms)
                    .unwrap_or(DurableUtcTimestamp::from_millis(i64::MAX));
                let _ = self
                    .store
                    .release(ReleaseInput {
                        task_id,
                        lease_id,
                        retry_not_before,
                    })
                    .await;
            }
        }
    }

    fn emit_settle_outcome(
        &self,
        request_id: &str,
        task_id: &TaskId,
        result: &std::result::Result<SettleOutcome, skiff_task_control::TaskStoreError>,
    ) {
        match result {
            Ok(SettleOutcome::Settled(record)) | Ok(SettleOutcome::AlreadySettled(record)) => {
                let outcome = record
                    .terminal
                    .as_ref()
                    .map(|terminal| match &terminal.outcome {
                        TaskOutcome::Succeeded => "succeeded",
                        TaskOutcome::TargetFailed { .. } => "targetFailed",
                        TaskOutcome::PlatformFailed { .. } => "platformFailed",
                        TaskOutcome::Canceled => "canceled",
                    })
                    .unwrap_or("unknown");
                let level = match outcome {
                    "succeeded" => TelemetryLevel::Info,
                    _ => TelemetryLevel::Warn,
                };
                let mut event = task_event(
                    "task.settled",
                    level,
                    Some(record.task_id.as_str()),
                    json!({
                        "outcome": outcome,
                        "settledAtMs": record
                            .terminal
                            .as_ref()
                            .map(|terminal| terminal.settled_at.millis())
                            .unwrap_or_default(),
                    })
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
                );
                event.request_id = Some(request_id.to_string());
                self.telemetry.emit(event);
            }
            Ok(SettleOutcome::Conflict(_)) => {
                self.emit_settle_stale(request_id, task_id, "conflict");
            }
            Ok(SettleOutcome::StaleLease) => {
                self.emit_settle_stale(request_id, task_id, "staleLease");
            }
            Ok(SettleOutcome::ExpiredLease) => {
                self.emit_settle_stale(request_id, task_id, "expiredLease");
            }
            Ok(SettleOutcome::NotLeased) => {
                self.emit_settle_stale(request_id, task_id, "notLeased");
            }
            Ok(SettleOutcome::NotFound) => {
                self.emit_settle_stale(request_id, task_id, "notFound");
            }
            Err(_) => {
                let mut attrs = Map::new();
                attrs.insert(
                    "reason".to_string(),
                    Value::String("storeUnavailable".to_string()),
                );
                let mut event = task_event(
                    "task.settle.uncertain",
                    TelemetryLevel::Warn,
                    Some(task_id.as_str()),
                    attrs,
                );
                event.request_id = Some(request_id.to_string());
                self.telemetry.emit(event);
            }
        }
    }

    fn emit_settle_stale(&self, request_id: &str, task_id: &TaskId, reason: &str) {
        let mut attrs = Map::new();
        attrs.insert("reason".to_string(), Value::String(reason.to_string()));
        let mut event = task_event(
            "task.settle.stale",
            TelemetryLevel::Warn,
            Some(task_id.as_str()),
            attrs,
        );
        event.request_id = Some(request_id.to_string());
        self.telemetry.emit(event);
    }

    fn emit_uncertain_attempt(&self, request_id: &str, task_id: &TaskId, reason: &str) {
        let mut attrs = Map::new();
        attrs.insert("reason".to_string(), Value::String(reason.to_string()));
        let mut event = task_event(
            "task.attempt.uncertain",
            TelemetryLevel::Warn,
            Some(task_id.as_str()),
            attrs,
        );
        event.request_id = Some(request_id.to_string());
        self.telemetry.emit(event);
    }

    fn emit_lease_released(&self, task_id: &TaskId, lease_id: &LeaseId, retry_after_ms: u64) {
        let mut attrs = Map::new();
        attrs.insert("leaseId".to_string(), Value::String(lease_id.as_str().to_string()));
        attrs.insert("retryAfterMs".to_string(), json!(retry_after_ms));
        self.telemetry.emit(task_event(
            "task.lease.released",
            TelemetryLevel::Info,
            Some(task_id.as_str()),
            attrs,
        ));
    }

    fn enqueue_settle(
        &self,
        request_id: &str,
        task_id: &TaskId,
        lease_id: &LeaseId,
        outcome: TaskOutcome,
    ) {
        self.pending.lock().expect("pending lock").remove(request_id);
        let _ = self.events.try_send(ControlEvent::Settle {
            request_id: request_id.to_string(),
            task_id: task_id.clone(),
            lease_id: lease_id.clone(),
            outcome,
        });
    }

    fn enqueue_release(
        &self,
        request_id: &str,
        task_id: &TaskId,
        lease_id: &LeaseId,
        retry_after_ms: u64,
    ) {
        self.pending.lock().expect("pending lock").remove(request_id);
        self.emit_lease_released(task_id, lease_id, retry_after_ms);
        if let Some(scheduler) = self
            .scheduler
            .lock()
            .expect("deferred scheduler lock")
            .clone()
        {
            scheduler.forget_active_lease(task_id, lease_id);
        }
        let _ = self.events.try_send(ControlEvent::Release {
            request_id: request_id.to_string(),
            task_id: task_id.clone(),
            lease_id: lease_id.clone(),
            retry_after_ms,
        });
    }

    fn forget_now(&self, request_id: &str, task_id: &TaskId, lease_id: &LeaseId) {
        self.pending.lock().expect("pending lock").remove(request_id);
        self.emit_uncertain_attempt(
            request_id,
            task_id,
            "attempt terminal uncertain; lease expiry drives infrastructure recovery",
        );
        // Stop renewing so store-authority lease expiry drives recovery; the
        // attempt is neither settled nor released.
        if let Some(scheduler) = self
            .scheduler
            .lock()
            .expect("deferred scheduler lock")
            .clone()
        {
            scheduler.forget_active_lease(task_id, lease_id);
        }
    }
}

impl TaskAttemptTerminalSink for DurableTaskControl {
    fn on_terminal(
        &self,
        request_id: &str,
        task_id: &str,
        _attempt_id: &str,
        lease_id: &str,
        outcome: TaskAttemptTerminalOutcome,
    ) {
        let task_id = TaskId::new(task_id);
        let lease_id = LeaseId::new(lease_id);
        match outcome {
            TaskAttemptTerminalOutcome::Succeeded => {
                self.counters
                    .settlements_succeeded
                    .fetch_add(1, Ordering::Relaxed);
                self.enqueue_settle(request_id, &task_id, &lease_id, TaskOutcome::Succeeded);
            }
            TaskAttemptTerminalOutcome::Failed { message } => {
                self.counters
                    .settlements_failed
                    .fetch_add(1, Ordering::Relaxed);
                self.enqueue_settle(
                    request_id,
                    &task_id,
                    &lease_id,
                    TaskOutcome::TargetFailed { error: message },
                );
            }
            TaskAttemptTerminalOutcome::Uncertain { reason } => {
                self.counters
                    .settlements_uncertain
                    .fetch_add(1, Ordering::Relaxed);
                tracing_or_eprintln(format!(
                    "task attempt {request_id} terminal uncertain: {reason}"
                ));
                self.forget_now(request_id, &task_id, &lease_id);
            }
        }
    }
}

impl ActorAttemptTerminalSink for DurableTaskControl {
    fn on_actor_terminal(
        &self,
        request_id: &str,
        task_id: &str,
        _attempt_id: &str,
        lease_id: &str,
        terminal: ActorAttemptTerminal,
    ) {
        let task_id = TaskId::new(task_id);
        let lease_id = LeaseId::new(lease_id);
        match terminal {
            ActorAttemptTerminal::Succeeded => {
                self.counters
                    .settlements_succeeded
                    .fetch_add(1, Ordering::Relaxed);
                self.enqueue_settle(request_id, &task_id, &lease_id, TaskOutcome::Succeeded);
            }
            ActorAttemptTerminal::TargetFailed { message } => {
                self.counters
                    .settlements_failed
                    .fetch_add(1, Ordering::Relaxed);
                self.enqueue_settle(
                    request_id,
                    &task_id,
                    &lease_id,
                    TaskOutcome::TargetFailed { error: message },
                );
            }
            ActorAttemptTerminal::VersionRejected { message } => {
                self.counters
                    .settlements_platform_failed
                    .fetch_add(1, Ordering::Relaxed);
                self.enqueue_settle(
                    request_id,
                    &task_id,
                    &lease_id,
                    TaskOutcome::PlatformFailed { reason: message },
                );
            }
            ActorAttemptTerminal::Upgrading { retry_after_ms } => {
                self.counters
                    .settlements_upgrading
                    .fetch_add(1, Ordering::Relaxed);
                self.enqueue_release(request_id, &task_id, &lease_id, retry_after_ms);
            }
            ActorAttemptTerminal::Uncertain { reason } => {
                self.counters
                    .settlements_uncertain
                    .fetch_add(1, Ordering::Relaxed);
                tracing_or_eprintln(format!(
                    "task attempt {request_id} actor terminal uncertain: {reason}"
                ));
                self.forget_now(request_id, &task_id, &lease_id);
            }
        }
    }
}

fn tracing_or_eprintln(message: String) {
    if std::env::var("SKIFF_ROUTER_TASK_DEBUG").is_ok() {
        eprintln!("[task-control] {message}");
    }
}
