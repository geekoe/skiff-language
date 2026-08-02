use super::capabilities::{
    ExecutionControl, StreamCapabilityContext as EvalStreamCapabilityContext,
    StreamSink as EvalStreamSink, TypedStreamSink as EvalTypedStreamSink,
};
use super::type_descriptor::TypeSubstitutions;
use crate::error::{Result, RuntimeError};
use serde_json::Value;
use skiff_runtime_capability_context::SupervisedStreamConsumptionChild;
use skiff_runtime_linked_program::LinkedExecutable;
use skiff_runtime_model::{runtime_value::RuntimeValueCarrier, type_plan::RuntimeTypePlan};

mod slot_store;

use slot_store::SlotStoreRollbackCheckpoint;
use slot_store::{program_parameter_slot, program_slot_layout, RuntimeSlotLayout};
pub use slot_store::{SlotDebugBinding, SlotStore};

#[derive(Clone, Debug)]
pub enum Flow {
    Continue,
    Return(RuntimeValueCarrier),
    Break,
    LoopContinue,
    Parked,
    ContinueConsumer,
}

#[derive(Clone, Debug)]
pub struct Env {
    storage: SlotStore,
    stream_consumer_supervision: Option<EvalStreamConsumerSupervision>,
    pub stream_sink: Option<EvalStreamSink>,
    pub current_module: Option<String>,
    pub current_stream_item_type: Option<RuntimeTypePlan>,
    pub response_stream_sink: Option<EvalTypedStreamSink>,
    pub current_assembly_index: usize,
    pub type_substitutions: TypeSubstitutions,
}

#[derive(Clone, Debug)]
pub(crate) struct EnvRollbackCheckpoint {
    storage: SlotStoreRollbackCheckpoint,
}

impl Env {
    pub fn new() -> Self {
        Self {
            storage: SlotStore::empty(),
            stream_consumer_supervision: None,
            stream_sink: None,
            current_module: None,
            current_stream_item_type: None,
            response_stream_sink: None,
            current_assembly_index: 0,
            type_substitutions: TypeSubstitutions::new(),
        }
    }

    pub fn for_program_executable(
        executable: &LinkedExecutable,
        current_module: Option<String>,
        current_assembly_index: usize,
    ) -> Result<Self> {
        let layout = program_slot_layout(&executable.slots, executable)?;
        let mut env = Self::with_slot_layout(&layout);
        env.current_module = current_module;
        env.current_assembly_index = current_assembly_index;
        Ok(env)
    }

    fn with_slot_layout(layout: &RuntimeSlotLayout) -> Self {
        Self {
            storage: SlotStore {
                values: vec![None; layout.count],
                debug_bindings: layout
                    .bindings
                    .iter()
                    .map(|binding| SlotDebugBinding {
                        slot: binding.slot,
                        name: binding.name.clone(),
                        kind: binding.kind.clone(),
                    })
                    .collect(),
                self_slot: layout.self_slot,
            },
            stream_consumer_supervision: None,
            stream_sink: None,
            current_module: None,
            current_stream_item_type: None,
            response_stream_sink: None,
            current_assembly_index: 0,
            type_substitutions: TypeSubstitutions::new(),
        }
    }

    pub fn push(&mut self) {}

    pub fn pop(&mut self) {}

    pub fn declare_binding(
        &mut self,
        name: &str,
        slot: Option<usize>,
        value: impl Into<RuntimeValueCarrier>,
    ) -> Result<()> {
        self.storage.declare(name, slot, value.into())
    }

    pub fn assign_binding(
        &mut self,
        name: &str,
        slot: Option<usize>,
        value: impl Into<RuntimeValueCarrier>,
    ) -> Result<()> {
        self.storage.assign(name, slot, value.into())
    }

    pub fn get_binding(&self, name: &str, slot: Option<usize>) -> Result<RuntimeValueCarrier> {
        self.storage.get(name, slot)
    }

    pub fn get_slot(&self, slot: usize) -> Result<RuntimeValueCarrier> {
        self.storage.get_slot(slot)
    }

    #[allow(dead_code)]
    pub fn get_binding_mut(
        &mut self,
        name: &str,
        slot: Option<usize>,
    ) -> Result<&mut RuntimeValueCarrier> {
        self.storage.get_mut(name, slot)
    }

    pub fn clear_slots(&mut self, slots: &[usize]) {
        self.storage.clear(slots);
    }

    pub(crate) fn rollback_checkpoint(&self) -> EnvRollbackCheckpoint {
        EnvRollbackCheckpoint {
            storage: self.storage.rollback_checkpoint(),
        }
    }

    pub(crate) fn rollback_root_carriers(
        &self,
        checkpoint: &EnvRollbackCheckpoint,
    ) -> Result<Vec<(usize, RuntimeValueCarrier)>> {
        self.storage.rollback_root_carriers(&checkpoint.storage)
    }

    pub(crate) fn rebased_for_rollback(
        &self,
        checkpoint: &EnvRollbackCheckpoint,
        roots: &[(usize, RuntimeValueCarrier)],
    ) -> Result<Self> {
        let mut candidate = self.clone();
        candidate.storage = self
            .storage
            .rebased_for_rollback(&checkpoint.storage, roots)?;
        Ok(candidate)
    }

    pub fn self_value(&self) -> Option<RuntimeValueCarrier> {
        self.storage
            .self_slot
            .and_then(|slot| self.storage.values.get(slot))
            .and_then(Option::as_ref)
            .cloned()
    }

    /// Copies execution metadata for work that will run against an independent
    /// request heap. Slot layout/debug metadata remains available, but no value
    /// from the source heap is retained.
    pub(crate) fn detached_for_independent_heap(&self) -> Self {
        let mut detached = self.clone();
        detached.storage = self.storage.detached();
        detached
    }

    pub(crate) fn restore_detached_self(&mut self, value: RuntimeValueCarrier) -> Result<()> {
        self.storage.restore_self(value)
    }

    pub fn declare_program_self(
        &mut self,
        _executable: &LinkedExecutable,
        value: impl Into<RuntimeValueCarrier>,
    ) -> Result<()> {
        let slot = self.storage.self_slot;
        if slot.is_none() {
            return Ok(());
        }
        self.declare_binding("self", slot, value)
    }

    pub fn declare_program_parameter(
        &mut self,
        executable: &LinkedExecutable,
        name: &str,
        value: impl Into<RuntimeValueCarrier>,
    ) -> Result<()> {
        let slot = program_parameter_slot(executable, name);
        self.declare_binding(name, slot, value)
    }

    pub fn stream_capability_context(&self) -> EvalStreamCapabilityContext {
        EvalStreamCapabilityContext::new(
            self.stream_sink.clone(),
            self.response_stream_sink.clone(),
        )
    }

    pub(crate) fn supervise_stream_consumer(
        &mut self,
        stream_value: Value,
        supervision: SupervisedStreamConsumptionChild,
    ) {
        self.stream_consumer_supervision = Some(EvalStreamConsumerSupervision {
            stream_value,
            supervision,
        });
    }

    pub(crate) fn inherit_stream_consumer_supervision_from(&mut self, caller: &Self) {
        self.stream_consumer_supervision = caller.stream_consumer_supervision.clone();
    }

    pub(crate) fn stream_consumer_supervision_for(
        &self,
        stream_value: &Value,
    ) -> Option<SupervisedStreamConsumptionChild> {
        self.stream_consumer_supervision
            .as_ref()
            .filter(|supervision| supervision.stream_value == *stream_value)
            .map(|supervision| supervision.supervision.clone())
    }
}

#[derive(Clone)]
struct EvalStreamConsumerSupervision {
    stream_value: Value,
    supervision: SupervisedStreamConsumptionChild,
}

impl std::fmt::Debug for EvalStreamConsumerSupervision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EvalStreamConsumerSupervision")
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

pub fn check_cancelled(execution: &ExecutionControl<'_>, env: &Env) -> Result<()> {
    execution.check_cancelled()?;
    if env
        .stream_sink
        .as_ref()
        .is_some_and(|sink| sink.is_cancelled())
        || env
            .response_stream_sink
            .as_ref()
            .is_some_and(|typed_sink| typed_sink.sink.is_cancelled())
    {
        return Err(RuntimeError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
