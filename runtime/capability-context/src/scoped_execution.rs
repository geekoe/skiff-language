use std::{sync::Arc, time::Instant};

use skiff_artifact_model::InstructionSourceSite;

use crate::{CancellationSignals, CancellationSource, CancellationToken};

mod deadline;
mod lease;

pub use deadline::{
    EffectiveDeadline, ExecutionDeadlineSource, ExecutionScopeAccessError,
    ExecutionScopeDeriveError, ExecutionScopeTerminal,
};
use lease::ExecutionScopeLifecycle;
pub use lease::{
    ExecutionScopeLease, ExecutionScopeLeaseCompletion, ExecutionScopeLeaseTerminal,
    ExecutionScopeLifecycleSnapshot,
};

#[derive(Clone)]
pub struct ExecutionScope {
    ancestor_cancellation: Arc<Vec<CancellationToken>>,
    local_cancellation: CancellationSource,
    effective_deadline: Option<EffectiveDeadline>,
    nesting: u32,
    lifecycle: Arc<ExecutionScopeLifecycle>,
}

impl ExecutionScope {
    pub fn request(
        request_cancellation: CancellationToken,
        request_deadline: Option<Instant>,
    ) -> Self {
        Self {
            ancestor_cancellation: Arc::new(vec![request_cancellation]),
            local_cancellation: CancellationSource::new(),
            effective_deadline: request_deadline.map(EffectiveDeadline::request),
            nesting: 0,
            lifecycle: Arc::new(ExecutionScopeLifecycle::default()),
        }
    }

    pub fn derive(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<Self, ExecutionScopeDeriveError> {
        let nesting = self
            .nesting
            .checked_add(1)
            .ok_or(ExecutionScopeDeriveError)?;
        let local_deadline = EffectiveDeadline::scope(local_deadline, site, nesting);
        let effective_deadline = match &self.effective_deadline {
            Some(parent) if parent.at() <= local_deadline.at() => parent.clone(),
            _ => local_deadline,
        };
        let mut ancestor_cancellation = self.ancestor_cancellation.as_ref().clone();
        ancestor_cancellation.push(self.local_cancellation.token());

        Ok(Self {
            ancestor_cancellation: Arc::new(ancestor_cancellation),
            local_cancellation: CancellationSource::new(),
            effective_deadline: Some(effective_deadline),
            nesting,
            lifecycle: self.lifecycle.clone(),
        })
    }

    pub fn nesting(&self) -> u32 {
        self.nesting
    }

    pub fn effective_deadline(&self) -> Option<&EffectiveDeadline> {
        self.effective_deadline.as_ref()
    }

    pub fn cancellation_signals(&self) -> CancellationSignals<'static> {
        let mut tokens = self.ancestor_cancellation.as_ref().clone();
        tokens.push(self.local_cancellation.token());
        CancellationSignals::from_tokens(tokens)
    }

    pub fn is_ancestor_cancelled(&self) -> bool {
        self.ancestor_cancellation
            .iter()
            .any(CancellationToken::is_cancelled)
    }

    pub fn terminal_at(&self, now: Instant) -> Option<ExecutionScopeTerminal> {
        if self.is_ancestor_cancelled() {
            return Some(ExecutionScopeTerminal::AncestorCancelled);
        }

        let deadline = self.effective_deadline.as_ref()?;
        if self.local_cancellation.is_cancelled() {
            return Some(ExecutionScopeTerminal::LocalDeadlineExceeded(
                deadline.clone(),
            ));
        }
        if now < deadline.at() {
            return None;
        }
        if self.owns_effective_deadline(deadline) {
            self.local_cancellation.cancel();
            Some(ExecutionScopeTerminal::LocalDeadlineExceeded(
                deadline.clone(),
            ))
        } else {
            Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(
                deadline.clone(),
            ))
        }
    }

    pub fn acquire_lease(&self) -> (ExecutionScopeLease, ExecutionScopeLeaseCompletion) {
        lease::acquire(self)
    }

    pub fn lifecycle_snapshot(&self) -> ExecutionScopeLifecycleSnapshot {
        self.lifecycle.snapshot()
    }

    fn owns_effective_deadline(&self, deadline: &EffectiveDeadline) -> bool {
        matches!(deadline.source(), ExecutionDeadlineSource::Scope { .. })
            && deadline.nesting() == self.nesting
    }

    fn ancestor_cancellation_signals(&self) -> CancellationSignals<'static> {
        CancellationSignals::from_tokens(self.ancestor_cancellation.as_ref().clone())
    }

    fn with_lease_child_cancellation(&self, child_cancellation: CancellationToken) -> Self {
        let mut ancestor_cancellation = self.ancestor_cancellation.as_ref().clone();
        ancestor_cancellation.push(child_cancellation);
        Self {
            ancestor_cancellation: Arc::new(ancestor_cancellation),
            local_cancellation: self.local_cancellation.clone(),
            effective_deadline: self.effective_deadline.clone(),
            nesting: self.nesting,
            lifecycle: self.lifecycle.clone(),
        }
    }
}
