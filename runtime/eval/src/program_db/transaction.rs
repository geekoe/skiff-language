use skiff_runtime_model::request_heap::RequestHeap;

use crate::{
    capabilities::DbCapabilityStore,
    error::{Result, RuntimeError},
    program_execution::ProgramExecutionContext,
};

use super::wait::await_operation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransactionPhase {
    Body,
    CommitSelected,
    AbortSelected,
    Complete,
}

/// One evaluator-owned transaction lifecycle. `Drop` intentionally performs no
/// asynchronous terminal action: an internally stopped request releases this
/// owner and leaves session cleanup to the service-db/driver fallback.
pub(super) struct TransactionLifecycle {
    store: DbCapabilityStore,
    phase: TransactionPhase,
}

impl TransactionLifecycle {
    pub(super) async fn begin(
        store: DbCapabilityStore,
        context: &ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
    ) -> Result<Self> {
        let begin_store = store.clone();
        await_operation(context, heap, async move {
            begin_store.begin_transaction().await
        })
        .await??;
        Ok(Self {
            store,
            phase: TransactionPhase::Body,
        })
    }

    pub(super) async fn commit(
        mut self,
        context: &ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
    ) -> Result<()> {
        debug_assert_eq!(self.phase, TransactionPhase::Body);
        self.phase = TransactionPhase::CommitSelected;
        let commit_store = self.store.clone();
        let commit = await_operation(context, heap, async move {
            commit_store.commit_transaction().await
        })
        .await?;
        match commit {
            Ok(()) => {
                self.phase = TransactionPhase::Complete;
                Ok(())
            }
            Err(error) => {
                self.abort_selected(context, heap).await?;
                Err(RuntimeError::from(error))
            }
        }
    }

    pub(super) async fn abort(
        mut self,
        context: &ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
    ) -> Result<()> {
        debug_assert_eq!(self.phase, TransactionPhase::Body);
        self.abort_selected(context, heap).await
    }

    async fn abort_selected(
        &mut self,
        context: &ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
    ) -> Result<()> {
        debug_assert!(matches!(
            self.phase,
            TransactionPhase::Body | TransactionPhase::CommitSelected
        ));
        self.phase = TransactionPhase::AbortSelected;
        let abort_store = self.store.clone();
        await_operation(context, heap, async move {
            abort_store.abort_transaction().await;
        })
        .await?;
        self.phase = TransactionPhase::Complete;
        Ok(())
    }
}
