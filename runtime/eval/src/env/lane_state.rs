use std::collections::HashSet;

use skiff_runtime_capability_context::OwnedExecutionControl;
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value_carrier_between_heaps, RequestHeap},
    runtime_value::RuntimeValueCarrier,
};

use crate::{
    error::{Result, RuntimeError},
    program_execution::ProgramExecutionContext,
};

use super::{lane_control::execution_control_for_lane, Env};

#[derive(Clone)]
pub(super) struct ConcurrentBaseline {
    env: Env,
    heap: RequestHeap,
}

pub(crate) struct LaneExecutionState {
    env: Env,
    heap: RequestHeap,
    execution: OwnedExecutionControl,
    imported_slots: HashSet<usize>,
}

pub(crate) struct LaneCompletion {
    state: LaneExecutionState,
    outcome: Result<Option<RuntimeValueCarrier>>,
}

pub(super) struct LaneExport {
    pub(super) source_order: usize,
    pub(super) slot: usize,
    pub(super) source_heap: RequestHeap,
    pub(super) carrier: RuntimeValueCarrier,
}

impl ConcurrentBaseline {
    pub(super) fn freeze(env: &Env, heap: &RequestHeap) -> Self {
        Self {
            env: env.clone(),
            heap: heap.clone(),
        }
    }

    pub(super) fn lane_state(
        &self,
        parent: OwnedExecutionControl,
        scope: skiff_runtime_capability_context::ExecutionScope,
        lane_cancellation: skiff_runtime_capability_context::CancellationToken,
    ) -> LaneExecutionState {
        LaneExecutionState {
            env: self.env.clone(),
            heap: self.heap.clone(),
            execution: execution_control_for_lane(parent, scope, lane_cancellation),
            imported_slots: HashSet::new(),
        }
    }
}

impl LaneExecutionState {
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    pub(crate) fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    pub(crate) fn heap(&self) -> &RequestHeap {
        &self.heap
    }

    pub(crate) fn heap_mut(&mut self) -> &mut RequestHeap {
        &mut self.heap
    }

    pub(crate) fn env_and_heap_mut(&mut self) -> (&mut Env, &mut RequestHeap) {
        (&mut self.env, &mut self.heap)
    }

    pub(crate) fn execution_control(&self) -> OwnedExecutionControl {
        self.execution.clone()
    }

    pub(crate) fn program_context<'a>(
        &self,
        parent: &ProgramExecutionContext<'a>,
    ) -> ProgramExecutionContext<'a> {
        parent
            .clone()
            .with_execution_control(self.execution.clone())
    }

    pub(crate) fn complete(self, outcome: Result<Option<RuntimeValueCarrier>>) -> LaneCompletion {
        LaneCompletion {
            state: self,
            outcome,
        }
    }

    pub(super) fn import_export(&mut self, export: &LaneExport) -> Result<()> {
        if export.slot >= self.env.storage.values.len() {
            return Err(invalid_handoff(format!(
                "dependency lane {} export slot {} is out of bounds",
                export.source_order, export.slot
            )));
        }
        if !self.imported_slots.insert(export.slot) {
            return Err(invalid_handoff(format!(
                "dependency import repeats destination slot {}",
                export.slot
            )));
        }
        let carrier = deep_clone_runtime_value_carrier_between_heaps(
            &export.source_heap,
            &mut self.heap,
            &export.carrier,
        )
        .map_err(|error| invalid_handoff(format!("dependency carrier clone failed: {error}")))?;
        self.env.storage.values[export.slot] = Some(carrier);
        Ok(())
    }

    pub(super) fn into_export(self, source_order: usize, slot: usize) -> Result<LaneExport> {
        let carrier = self
            .env
            .storage
            .values
            .get(slot)
            .ok_or_else(|| {
                invalid_handoff(format!(
                    "lane {source_order} export slot {slot} is out of bounds"
                ))
            })?
            .clone()
            .ok_or_else(|| {
                invalid_handoff(format!(
                    "lane {source_order} completed without export slot {slot}"
                ))
            })?;
        Ok(LaneExport {
            source_order,
            slot,
            source_heap: self.heap,
            carrier,
        })
    }

    pub(super) fn into_heap(self) -> RequestHeap {
        self.heap
    }

    pub(super) fn into_heap_and_outcome(
        self,
        value: RuntimeValueCarrier,
    ) -> (RequestHeap, RuntimeValueCarrier) {
        (self.heap, value)
    }
}

impl LaneCompletion {
    pub(crate) fn normal(state: LaneExecutionState) -> Self {
        state.complete(Ok(None))
    }

    pub(crate) fn value(state: LaneExecutionState, value: RuntimeValueCarrier) -> Self {
        state.complete(Ok(Some(value)))
    }

    pub(crate) fn error(state: LaneExecutionState, error: RuntimeError) -> Self {
        state.complete(Err(error))
    }

    pub(super) fn into_parts(self) -> (LaneExecutionState, Result<Option<RuntimeValueCarrier>>) {
        (self.state, self.outcome)
    }
}

fn invalid_handoff(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "invalid concurrent lane handoff: {}",
        message.into()
    ))
}
