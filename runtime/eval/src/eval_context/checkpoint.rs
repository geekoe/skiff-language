use super::*;
use crate::program_execution::{ExecutionCheckpoint, ExecutionCheckpointKind};

pub(super) fn actual_pending_checkpoint(context: &ProgramExecutionContext<'_>) -> Result<()> {
    context.poll_execution_scope()
}

impl EvalContext<'_> {
    pub(super) fn checkpoint_function_entry(&self) -> Result<()> {
        self.context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::FunctionEntry,
            1,
        ))
    }

    pub(super) fn checkpoint_loop_condition(&self, units: u64) -> Result<()> {
        self.context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::LoopCondition,
            units,
        ))
    }

    pub(super) fn checkpoint_loop_backedge(&self, units: u64) -> Result<()> {
        self.context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::LoopBackedge,
            units,
        ))
    }

    pub(super) fn checkpoint_generated_chunk(&self, units: u64) -> Result<()> {
        self.context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::GeneratedChunk,
            units,
        ))
    }
}
