use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
};

use skiff_runtime_capability_context::{
    DbCapabilityError, DbCapabilityLeaseHandle, DbCapabilityResult, DbDocument,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::program_db::tests) enum DbPhase {
    RawCreate,
    PreparedCreateWait,
    PreparedCreateFinalize,
    Begin,
    BodyCreate,
    Commit,
    Abort,
    Claim,
    Renew,
    LeaseLost,
    Release,
    Read,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::program_db::tests) enum DbEventKind {
    Constructed,
    Poll,
    Pending,
    Ready,
    DropBeforeTerminal,
    DropAfterTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::program_db::tests) struct DbEvent {
    pub phase: DbPhase,
    pub kind: DbEventKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::program_db::tests) struct OperationMetrics {
    pub constructed: usize,
    pub polls: usize,
    pub pending_returns: usize,
    pub ready_returns: usize,
    pub dropped_before_terminal: usize,
    pub dropped_after_terminal: usize,
}

#[derive(Clone, Default)]
pub(in crate::program_db::tests) struct Gate {
    sender: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    released: Arc<AtomicBool>,
}

impl Gate {
    fn pair() -> (Self, oneshot::Receiver<()>) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                sender: Arc::new(Mutex::new(Some(sender))),
                released: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        )
    }

    pub(in crate::program_db::tests) fn release(&self) {
        let sender = self.sender.lock().expect("DB gate lock poisoned").take();
        assert!(sender.is_some(), "DB gate may only be released once");
        self.released.store(true, Ordering::SeqCst);
        let _ = sender.expect("checked above").send(());
    }

    pub(in crate::program_db::tests) fn is_released(&self) -> bool {
        self.released.load(Ordering::SeqCst)
    }
}

struct ScriptStep<T> {
    gate: Option<oneshot::Receiver<()>>,
    terminal: DbCapabilityResult<T>,
}

pub(in crate::program_db::tests) struct Script<T> {
    steps: Mutex<VecDeque<ScriptStep<T>>>,
}

impl<T> Default for Script<T> {
    fn default() -> Self {
        Self {
            steps: Mutex::new(VecDeque::new()),
        }
    }
}

impl<T> Script<T> {
    pub(in crate::program_db::tests) fn push_ready(&self, terminal: DbCapabilityResult<T>) {
        self.steps
            .lock()
            .expect("DB script lock poisoned")
            .push_back(ScriptStep {
                gate: None,
                terminal,
            });
    }

    pub(in crate::program_db::tests) fn push_pending(
        &self,
        terminal: DbCapabilityResult<T>,
    ) -> Gate {
        let (gate, receiver) = Gate::pair();
        self.steps
            .lock()
            .expect("DB script lock poisoned")
            .push_back(ScriptStep {
                gate: Some(receiver),
                terminal,
            });
        gate
    }

    pub(in crate::program_db::tests) fn take(
        &self,
        state: &Arc<FakeDbState>,
        phase: DbPhase,
    ) -> ScriptedFuture<T> {
        let step = self
            .steps
            .lock()
            .expect("DB script lock poisoned")
            .pop_front()
            .unwrap_or_else(|| panic!("DB phase {phase:?} has no scripted outcome"));
        state.record(phase, DbEventKind::Constructed);
        ScriptedFuture {
            state: Arc::clone(state),
            phase,
            gate: step.gate,
            terminal: Some(step.terminal),
            terminal_returned: false,
        }
    }

    pub(in crate::program_db::tests) fn remaining(&self) -> usize {
        self.steps.lock().expect("DB script lock poisoned").len()
    }
}

pub(in crate::program_db::tests) struct ScriptedFuture<T> {
    state: Arc<FakeDbState>,
    phase: DbPhase,
    gate: Option<oneshot::Receiver<()>>,
    terminal: Option<DbCapabilityResult<T>>,
    terminal_returned: bool,
}

impl<T> Unpin for ScriptedFuture<T> {}

impl<T> Future for ScriptedFuture<T> {
    type Output = DbCapabilityResult<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.state.record(self.phase, DbEventKind::Poll);
        if let Some(gate) = self.gate.as_mut() {
            match Pin::new(gate).poll(context) {
                Poll::Pending => {
                    self.state.record(self.phase, DbEventKind::Pending);
                    return Poll::Pending;
                }
                Poll::Ready(Ok(())) => {
                    self.gate = None;
                }
                Poll::Ready(Err(_)) => {
                    self.gate = None;
                    self.terminal = Some(Err(db_error(format!(
                        "{:?} gate sender dropped",
                        self.phase
                    ))));
                }
            }
        }
        self.terminal_returned = true;
        self.state.record(self.phase, DbEventKind::Ready);
        Poll::Ready(
            self.terminal
                .take()
                .expect("scripted DB terminal may only be returned once"),
        )
    }
}

impl<T> Drop for ScriptedFuture<T> {
    fn drop(&mut self) {
        self.state.record(
            self.phase,
            if self.terminal_returned {
                DbEventKind::DropAfterTerminal
            } else {
                DbEventKind::DropBeforeTerminal
            },
        );
    }
}

pub(in crate::program_db::tests) struct PreparedFinalize {
    finalize:
        Box<dyn FnOnce(&mut RequestHeap) -> DbCapabilityResult<RuntimeValue> + Send + 'static>,
}

impl PreparedFinalize {
    pub(in crate::program_db::tests) fn new<F>(finalize: F) -> Self
    where
        F: FnOnce(&mut RequestHeap) -> DbCapabilityResult<RuntimeValue> + Send + 'static,
    {
        Self {
            finalize: Box::new(finalize),
        }
    }

    pub(in crate::program_db::tests) fn value(value: RuntimeValue) -> Self {
        Self::new(move |_heap| Ok(value))
    }

    pub(in crate::program_db::tests) fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(move |_heap| Err(db_error(message)))
    }

    pub(in crate::program_db::tests) fn finalize(
        self,
        heap: &mut RequestHeap,
    ) -> DbCapabilityResult<RuntimeValue> {
        (self.finalize)(heap)
    }
}

#[derive(Default)]
pub(in crate::program_db::tests) struct FakeDbState {
    pub raw_create: Script<DbDocument>,
    pub prepared_create: Script<PreparedFinalize>,
    pub begin: Script<()>,
    pub body_create: Script<DbDocument>,
    pub commit: Script<()>,
    pub abort: Script<()>,
    pub claim: Script<Option<DbCapabilityLeaseHandle>>,
    pub renew: Script<bool>,
    pub lease_lost: Script<bool>,
    pub release: Script<()>,
    pub read: Script<Option<serde_json::Value>>,
    metrics: Mutex<HashMap<DbPhase, OperationMetrics>>,
    events: Mutex<Vec<DbEvent>>,
    context_require_calls: AtomicUsize,
    legacy_runtime_calls: AtomicUsize,
}

impl FakeDbState {
    pub(in crate::program_db::tests) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(in crate::program_db::tests) fn record(&self, phase: DbPhase, event: DbEventKind) {
        {
            let mut metrics = self.metrics.lock().expect("DB metrics lock poisoned");
            let metrics = metrics.entry(phase).or_default();
            match event {
                DbEventKind::Constructed => metrics.constructed += 1,
                DbEventKind::Poll => metrics.polls += 1,
                DbEventKind::Pending => metrics.pending_returns += 1,
                DbEventKind::Ready => metrics.ready_returns += 1,
                DbEventKind::DropBeforeTerminal => metrics.dropped_before_terminal += 1,
                DbEventKind::DropAfterTerminal => metrics.dropped_after_terminal += 1,
            }
        }
        self.events
            .lock()
            .expect("DB event trace lock poisoned")
            .push(DbEvent { phase, kind: event });
    }

    pub(in crate::program_db::tests) fn metrics(&self, phase: DbPhase) -> OperationMetrics {
        self.metrics
            .lock()
            .expect("DB metrics lock poisoned")
            .get(&phase)
            .copied()
            .unwrap_or_default()
    }

    pub(in crate::program_db::tests) fn events(&self) -> Vec<DbEvent> {
        self.events
            .lock()
            .expect("DB event trace lock poisoned")
            .clone()
    }

    pub(in crate::program_db::tests) fn phases(&self) -> Vec<DbPhase> {
        self.events()
            .into_iter()
            .filter_map(|event| (event.kind == DbEventKind::Constructed).then_some(event.phase))
            .collect()
    }

    pub(in crate::program_db::tests) fn probe(self: &Arc<Self>, phase: DbPhase) -> OperationProbe {
        OperationProbe {
            state: Arc::clone(self),
            phase,
        }
    }

    pub(in crate::program_db::tests) fn assert_completed_once(&self, phase: DbPhase) {
        assert_eq!(
            self.metrics(phase),
            OperationMetrics {
                constructed: 1,
                polls: 1,
                pending_returns: 0,
                ready_returns: 1,
                dropped_before_terminal: 0,
                dropped_after_terminal: 1,
            },
            "unexpected {phase:?} metrics"
        );
    }

    pub(in crate::program_db::tests) fn context_require_calls(&self) -> usize {
        self.context_require_calls.load(Ordering::SeqCst)
    }

    pub(in crate::program_db::tests) fn record_context_require(&self) {
        self.context_require_calls.fetch_add(1, Ordering::SeqCst);
    }

    pub(in crate::program_db::tests) fn legacy_runtime_calls(&self) -> usize {
        self.legacy_runtime_calls.load(Ordering::SeqCst)
    }

    pub(in crate::program_db::tests) fn record_legacy_runtime_call(&self) {
        self.legacy_runtime_calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub(in crate::program_db::tests) struct OperationProbe {
    state: Arc<FakeDbState>,
    phase: DbPhase,
}

impl OperationProbe {
    pub(in crate::program_db::tests) fn metrics(&self) -> OperationMetrics {
        self.state.metrics(self.phase)
    }

    pub(in crate::program_db::tests) async fn wait_until_polled(&self) {
        for _ in 0..64 {
            if self.metrics().polls > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("DB phase {:?} was not polled", self.phase);
    }
}

pub(in crate::program_db::tests) fn db_error(message: impl Into<String>) -> DbCapabilityError {
    DbCapabilityError::decode(message)
}
