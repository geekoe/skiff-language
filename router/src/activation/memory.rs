//! Direct-test in-memory activation state repository.
//!
//! This is the test double for the frozen port seam (C-router-activation-state
//! §10): it executes the same pure reducer and audit rules as the Mongo
//! adapter but without a driver. Injection knobs (`fail_next_audit_inserts`,
//! `fail_next_transient_operations`) exist for sequence tests only and have no
//! effect in production composition.

use std::sync::Arc;

use async_trait::async_trait;
use skiff_deployment::activation_state::{
    abort, commit, prepare, ActivationAuditEvent, ActivationAuditOperation, ProfileActivationState,
};
use tokio::sync::RwLock;

use super::{
    error::{cas_mismatch, invalid_record, map_reducer_error, RepositoryError},
    health::{ActivationRepositoryHealth, RepositoryMutationOutcome},
    repository::{AbortInput, ActivationStateRepository, CommitInput, PrepareInput},
    retry::{RetryPolicy, SystemClock},
};

#[derive(Debug)]
struct MemoryState {
    current: Option<ProfileActivationState>,
    audit: Vec<ActivationAuditEvent>,
    fail_audit_inserts: u64,
    transient_failures: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryActivationStateRepository {
    inner: Arc<RwLock<MemoryState>>,
    retry: RetryPolicy,
}

impl Default for MemoryActivationStateRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryActivationStateRepository {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryState {
                current: None,
                audit: Vec::new(),
                fail_audit_inserts: 0,
                transient_failures: 0,
            })),
            retry: RetryPolicy {
                max_attempts: 5,
                base_delay: std::time::Duration::from_millis(1),
                max_delay: std::time::Duration::from_millis(4),
                total_deadline: std::time::Duration::from_secs(2),
            },
        }
    }

    pub async fn fail_next_audit_inserts(&self, count: u64) {
        self.inner.write().await.fail_audit_inserts = count;
    }

    pub async fn fail_next_transient_operations(&self, count: u64) {
        self.inner.write().await.transient_failures = count;
    }

    pub async fn audit_events(&self) -> Vec<ActivationAuditEvent> {
        self.inner.read().await.audit.clone()
    }

    async fn transient_gate(&self) -> Result<(), RepositoryError> {
        let mut state = self.inner.write().await;
        if state.transient_failures > 0 {
            state.transient_failures -= 1;
            return Err(RepositoryError::Transient {
                message: "injected transient operation failure".to_string(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl ActivationStateRepository for MemoryActivationStateRepository {
    async fn read(&self, profile: &str) -> Result<ProfileActivationState, RepositoryError> {
        let state = self.inner.read().await;
        state
            .current
            .clone()
            .ok_or_else(|| cas_mismatch(profile, "activation state does not exist"))
    }

    async fn initialize(
        &self,
        state: &ProfileActivationState,
    ) -> Result<ProfileActivationState, RepositoryError> {
        let (result, _outcome) = self
            .retry
            .run(&SystemClock, || self.initialize_once(state))
            .await;
        result
    }

    async fn prepare(
        &self,
        input: PrepareInput,
    ) -> Result<ProfileActivationState, RepositoryError> {
        self.mutate(ActivationAuditOperation::Prepare, |current| {
            prepare(current, &input).map_err(map_reducer_error)
        })
        .await
    }

    async fn commit(&self, input: CommitInput) -> Result<ProfileActivationState, RepositoryError> {
        self.mutate(ActivationAuditOperation::Commit, |current| {
            commit(current, &input).map_err(map_reducer_error)
        })
        .await
    }

    async fn abort(&self, input: AbortInput) -> Result<ProfileActivationState, RepositoryError> {
        self.mutate(ActivationAuditOperation::Abort, |current| {
            abort(current, &input).map_err(map_reducer_error)
        })
        .await
    }

    async fn append_audit(&self, event: &ActivationAuditEvent) -> Result<(), RepositoryError> {
        let mut inner = self.inner.write().await;
        if inner
            .audit
            .iter()
            .any(|existing| existing.event_id == event.event_id)
        {
            return Ok(());
        }
        inner.audit.push(event.clone());
        Ok(())
    }

    async fn ensure_indexes(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn health(&self) -> ActivationRepositoryHealth {
        let mut health = ActivationRepositoryHealth::default();
        if let Ok(state) = self.inner.try_read() {
            if let Some(current) = &state.current {
                health.profile = Some(current.profile.clone());
                health.committed_generation = Some(current.committed.generation);
                health.pending_activation_id = current
                    .pending
                    .as_ref()
                    .map(|pending| pending.activation_id.clone());
            }
            health.audit.last_event_id = state.audit.last().map(|event| event.event_id.clone());
        }
        health.last_outcome = Some(RepositoryMutationOutcome::Ok);
        health
    }

    async fn close(&self) -> Result<(), RepositoryError> {
        Ok(())
    }
}

impl MemoryActivationStateRepository {
    async fn mutate<F>(
        &self,
        operation: ActivationAuditOperation,
        reduce: F,
    ) -> Result<ProfileActivationState, RepositoryError>
    where
        F: Fn(&ProfileActivationState) -> Result<ProfileActivationState, RepositoryError>
            + Send
            + Sync,
    {
        let (result, _outcome) = self
            .retry
            .run(&SystemClock, || self.mutate_once_inner(operation, &reduce))
            .await;
        result
    }

    async fn initialize_once(
        &self,
        state: &ProfileActivationState,
    ) -> Result<ProfileActivationState, RepositoryError> {
        self.transient_gate().await?;
        if state.pending.is_some() {
            return Err(invalid_record(
                &state.profile,
                "initial activation state cannot contain pending",
            ));
        }
        let mut inner = self.inner.write().await;
        match &inner.current {
            Some(existing) if existing == state => Ok(existing.clone()),
            Some(_) => Err(cas_mismatch(
                &state.profile,
                "activation state already exists with a different tuple",
            )),
            None => {
                inner.current = Some(state.clone());
                Ok(state.clone())
            }
        }
    }

    async fn mutate_once_inner<F>(
        &self,
        operation: ActivationAuditOperation,
        reduce: &F,
    ) -> Result<ProfileActivationState, RepositoryError>
    where
        F: Fn(&ProfileActivationState) -> Result<ProfileActivationState, RepositoryError> + Sync,
    {
        self.transient_gate().await?;
        let mut inner = self.inner.write().await;
        let current = inner
            .current
            .clone()
            .ok_or_else(|| cas_mismatch("<unknown>", "activation state does not exist"))?;
        let next = reduce(&current)?;
        if next == current {
            return Ok(next);
        }
        let event = memory_audit_event(operation, &current, &next);
        if inner.fail_audit_inserts > 0 {
            inner.fail_audit_inserts -= 1;
            return Err(RepositoryError::Transient {
                message: "injected audit append failure".to_string(),
            });
        }
        inner.audit.push(event);
        inner.current = Some(next.clone());
        Ok(next)
    }
}

fn memory_audit_event(
    operation: ActivationAuditOperation,
    current: &ProfileActivationState,
    next: &ProfileActivationState,
) -> ActivationAuditEvent {
    let (activation_id, expected_generation, candidate_generation, participants) = match operation {
        ActivationAuditOperation::Prepare => {
            let pending = next.pending.as_ref().expect("prepare writes pending");
            (
                pending.activation_id.clone(),
                current.committed.generation,
                pending.candidate_generation,
                Some(pending.participant_replica_ids.clone()),
            )
        }
        ActivationAuditOperation::Commit => {
            let pending = current.pending.as_ref().expect("commit reads pending");
            (
                pending.activation_id.clone(),
                current.committed.generation,
                next.committed.generation,
                Some(pending.participant_replica_ids.clone()),
            )
        }
        ActivationAuditOperation::Abort => {
            let pending = current.pending.as_ref().expect("abort reads pending");
            (
                pending.activation_id.clone(),
                current.committed.generation,
                pending.candidate_generation,
                Some(pending.participant_replica_ids.clone()),
            )
        }
    };
    ActivationAuditEvent::new(
        current.profile.clone(),
        activation_id,
        operation,
        expected_generation,
        candidate_generation,
        skiff_deployment::activation_state::ActivationAuditOutcome::Ok,
        participants,
        0,
    )
}
