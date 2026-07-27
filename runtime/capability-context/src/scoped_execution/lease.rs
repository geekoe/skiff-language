use std::{
    future,
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        Arc,
    },
};

use tokio::sync::Notify;

use super::{EffectiveDeadline, ExecutionScope, ExecutionScopeTerminal};
use crate::{CancellationSource, CancellationToken};

const LEASE_PENDING: u8 = 0;
const LEASE_COMPLETED: u8 = 1;
const LEASE_ANCESTOR_CANCELLED: u8 = 2;
const LEASE_LOCAL_DEADLINE: u8 = 3;
const LEASE_INHERITED_DEADLINE: u8 = 4;
const LEASE_DROPPED: u8 = 5;

#[derive(Default)]
pub(super) struct ExecutionScopeLifecycle {
    active_leases: AtomicUsize,
    active_waiters: AtomicUsize,
    active_timers: AtomicUsize,
}

impl ExecutionScopeLifecycle {
    pub(super) fn snapshot(&self) -> ExecutionScopeLifecycleSnapshot {
        ExecutionScopeLifecycleSnapshot {
            active_leases: self.active_leases.load(Ordering::Acquire),
            active_waiters: self.active_waiters.load(Ordering::Acquire),
            active_timers: self.active_timers.load(Ordering::Acquire),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutionScopeLifecycleSnapshot {
    pub active_leases: usize,
    pub active_waiters: usize,
    pub active_timers: usize,
}

pub struct ExecutionScopeLease {
    state: Arc<ExecutionScopeLeaseState>,
}

#[derive(Clone)]
pub struct ExecutionScopeLeaseCompletion {
    state: Arc<ExecutionScopeLeaseState>,
}

struct ExecutionScopeLeaseState {
    terminal: AtomicU8,
    notify: Notify,
    child_cancellation: CancellationSource,
    scope: ExecutionScope,
    lifecycle: Arc<ExecutionScopeLifecycle>,
    lease_active: AtomicBool,
    waiter_active: AtomicBool,
    timer_active: AtomicBool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionScopeLeaseTerminal {
    Completed,
    Control(ExecutionScopeTerminal),
}

pub(super) fn acquire(
    scope: &ExecutionScope,
) -> (ExecutionScopeLease, ExecutionScopeLeaseCompletion) {
    let timer_active = scope.effective_deadline.is_some();
    scope.lifecycle.active_leases.fetch_add(1, Ordering::AcqRel);
    scope
        .lifecycle
        .active_waiters
        .fetch_add(1, Ordering::AcqRel);
    if timer_active {
        scope.lifecycle.active_timers.fetch_add(1, Ordering::AcqRel);
    }

    let state = Arc::new(ExecutionScopeLeaseState {
        terminal: AtomicU8::new(LEASE_PENDING),
        notify: Notify::new(),
        child_cancellation: CancellationSource::new(),
        scope: scope.clone(),
        lifecycle: scope.lifecycle.clone(),
        lease_active: AtomicBool::new(true),
        waiter_active: AtomicBool::new(true),
        timer_active: AtomicBool::new(timer_active),
    });
    (
        ExecutionScopeLease {
            state: state.clone(),
        },
        ExecutionScopeLeaseCompletion { state },
    )
}

impl ExecutionScopeLease {
    pub fn child_cancellation_token(&self) -> CancellationToken {
        self.state.child_cancellation.token()
    }

    pub async fn wait(self) -> ExecutionScopeLeaseTerminal {
        loop {
            if let Some(terminal) = self.state.current_terminal() {
                return terminal;
            }
            if let Some(terminal) = self
                .state
                .scope
                .terminal_at(tokio::time::Instant::now().into_std())
            {
                self.state.settle_control(terminal);
                continue;
            }

            let ancestor_cancellation = self.state.scope.ancestor_cancellation_signals();
            let local_cancellation = self.state.scope.local_cancellation.token();
            let deadline = self
                .state
                .scope
                .effective_deadline()
                .map(EffectiveDeadline::at);
            let deadline_wait = async move {
                match deadline {
                    Some(deadline) => {
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                    }
                    None => future::pending::<()>().await,
                }
            };
            tokio::pin!(deadline_wait);

            tokio::select! {
                biased;
                _ = ancestor_cancellation.wait_cancelled() => {
                    self.state.settle_control(ExecutionScopeTerminal::AncestorCancelled);
                }
                _ = local_cancellation.wait_cancelled() => {
                    if let Some(terminal) = self
                        .state
                        .scope
                        .terminal_at(tokio::time::Instant::now().into_std())
                    {
                        self.state.settle_control(terminal);
                    }
                }
                _ = &mut deadline_wait => {
                    if let Some(terminal) = self
                        .state
                        .scope
                        .terminal_at(tokio::time::Instant::now().into_std())
                    {
                        self.state.settle_control(terminal);
                    }
                }
                _ = self.state.notify.notified() => {}
            }
        }
    }
}

impl Drop for ExecutionScopeLease {
    fn drop(&mut self) {
        self.state.settle(LEASE_DROPPED);
    }
}

impl ExecutionScopeLeaseCompletion {
    pub fn complete(&self) -> bool {
        if let Some(terminal) = self
            .state
            .scope
            .terminal_at(tokio::time::Instant::now().into_std())
        {
            self.state.settle_control(terminal);
            return false;
        }
        self.state.settle(LEASE_COMPLETED)
    }
}

impl ExecutionScopeLeaseState {
    fn settle_control(&self, terminal: ExecutionScopeTerminal) -> bool {
        let state = match terminal {
            ExecutionScopeTerminal::AncestorCancelled => LEASE_ANCESTOR_CANCELLED,
            ExecutionScopeTerminal::LocalDeadlineExceeded(_) => LEASE_LOCAL_DEADLINE,
            ExecutionScopeTerminal::InheritedDeadlineExceeded(_) => LEASE_INHERITED_DEADLINE,
        };
        self.settle(state)
    }

    fn settle(&self, terminal: u8) -> bool {
        if self
            .terminal
            .compare_exchange(LEASE_PENDING, terminal, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        if terminal != LEASE_COMPLETED {
            self.child_cancellation.cancel();
        }
        if self.lease_active.swap(false, Ordering::AcqRel) {
            self.lifecycle.active_leases.fetch_sub(1, Ordering::AcqRel);
        }
        if self.waiter_active.swap(false, Ordering::AcqRel) {
            self.lifecycle.active_waiters.fetch_sub(1, Ordering::AcqRel);
        }
        if self.timer_active.swap(false, Ordering::AcqRel) {
            self.lifecycle.active_timers.fetch_sub(1, Ordering::AcqRel);
        }
        self.notify.notify_waiters();
        true
    }

    fn current_terminal(&self) -> Option<ExecutionScopeLeaseTerminal> {
        match self.terminal.load(Ordering::Acquire) {
            LEASE_PENDING => None,
            LEASE_COMPLETED => Some(ExecutionScopeLeaseTerminal::Completed),
            LEASE_ANCESTOR_CANCELLED => Some(ExecutionScopeLeaseTerminal::Control(
                ExecutionScopeTerminal::AncestorCancelled,
            )),
            LEASE_LOCAL_DEADLINE => Some(ExecutionScopeLeaseTerminal::Control(
                ExecutionScopeTerminal::LocalDeadlineExceeded(
                    self.state_deadline()
                        .expect("local deadline terminal requires an effective deadline"),
                ),
            )),
            LEASE_INHERITED_DEADLINE => Some(ExecutionScopeLeaseTerminal::Control(
                ExecutionScopeTerminal::InheritedDeadlineExceeded(
                    self.state_deadline()
                        .expect("inherited deadline terminal requires an effective deadline"),
                ),
            )),
            LEASE_DROPPED => {
                unreachable!("a dropped execution scope lease cannot still be awaited")
            }
            _ => unreachable!("execution scope lease terminal state is internal and finite"),
        }
    }

    fn state_deadline(&self) -> Option<EffectiveDeadline> {
        self.scope.effective_deadline().cloned()
    }
}
