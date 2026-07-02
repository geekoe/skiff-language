use super::capabilities::{
    ExecutionControl, StreamCapabilityContext as EvalStreamCapabilityContext,
    StreamSink as EvalStreamSink, TypedStreamSink as EvalTypedStreamSink,
};
use super::type_descriptor::TypeSubstitutions;
use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::{LinkedExecutable, SlotLayoutIr};
use skiff_runtime_model::{runtime_value::RuntimeValue, type_plan::RuntimeTypePlan};

#[derive(Clone, Debug)]
pub enum Flow {
    Continue,
    Return(RuntimeValue),
    Break,
    LoopContinue,
    Parked,
    ContinueConsumer,
}

#[derive(Clone, Debug)]
pub struct Env {
    storage: SlotStore,
    pub stream_sink: Option<EvalStreamSink>,
    pub current_module: Option<String>,
    pub current_stream_item_type: Option<RuntimeTypePlan>,
    pub response_stream_sink: Option<EvalTypedStreamSink>,
    pub current_assembly_index: usize,
    pub type_substitutions: TypeSubstitutions,
}

#[derive(Clone, Debug)]
pub struct SlotStore {
    values: Vec<Option<RuntimeValue>>,
    debug_bindings: Vec<SlotDebugBinding>,
    self_slot: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct SlotDebugBinding {
    pub slot: usize,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSlotLayout {
    count: usize,
    bindings: Vec<RuntimeSlotBinding>,
    self_slot: Option<usize>,
    parameter_slots: std::collections::HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeSlotBinding {
    slot: usize,
    name: String,
    kind: String,
    scope: Option<usize>,
}

impl Env {
    pub fn new() -> Self {
        Self {
            storage: SlotStore::empty(),
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
        value: RuntimeValue,
    ) -> Result<()> {
        self.storage.declare(name, slot, value)
    }

    pub fn assign_binding(
        &mut self,
        name: &str,
        slot: Option<usize>,
        value: RuntimeValue,
    ) -> Result<()> {
        self.storage.assign(name, slot, value)
    }

    pub fn get_binding(&self, name: &str, slot: Option<usize>) -> Result<RuntimeValue> {
        self.storage.get(name, slot)
    }

    pub fn get_slot(&self, slot: usize) -> Result<RuntimeValue> {
        self.storage.get_slot(slot)
    }

    #[allow(dead_code)]
    pub fn get_binding_mut(
        &mut self,
        name: &str,
        slot: Option<usize>,
    ) -> Result<&mut RuntimeValue> {
        self.storage.get_mut(name, slot)
    }

    pub fn clear_slots(&mut self, slots: &[usize]) {
        self.storage.clear(slots);
    }

    pub fn self_value(&self) -> Option<RuntimeValue> {
        self.storage
            .self_slot
            .and_then(|slot| self.storage.values.get(slot))
            .and_then(Option::as_ref)
            .cloned()
    }

    pub fn declare_program_self(
        &mut self,
        _executable: &LinkedExecutable,
        value: RuntimeValue,
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
        value: RuntimeValue,
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
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotStore {
    fn empty() -> Self {
        Self {
            values: Vec::new(),
            debug_bindings: Vec::new(),
            self_slot: None,
        }
    }

    fn declare(&mut self, name: &str, slot: Option<usize>, value: RuntimeValue) -> Result<()> {
        let slot = self.required_slot(name, slot, "binding")?;
        self.values[slot] = Some(value);
        Ok(())
    }

    fn assign(&mut self, name: &str, slot: Option<usize>, value: RuntimeValue) -> Result<()> {
        let slot = self.required_slot(name, slot, "assignment target")?;
        if self.values[slot].is_none() {
            return Err(RuntimeError::Decode(format!("unknown variable {name}")));
        }
        self.values[slot] = Some(value);
        Ok(())
    }

    fn get(&self, name: &str, slot: Option<usize>) -> Result<RuntimeValue> {
        let slot = self.required_slot(name, slot, "identifier")?;
        self.values[slot]
            .clone()
            .ok_or_else(|| RuntimeError::Decode(format!("unknown variable {name}")))
    }

    fn get_slot(&self, slot: usize) -> Result<RuntimeValue> {
        if slot >= self.values.len() {
            return Err(RuntimeError::InvalidArtifact(format!(
                "slot {slot} for identifier is out of bounds{}",
                self.debug_binding_suffix(slot)
            )));
        }
        self.values[slot]
            .clone()
            .ok_or_else(|| RuntimeError::Decode(self.unknown_slot_message(slot)))
    }

    #[allow(dead_code)]
    fn get_mut(&mut self, name: &str, slot: Option<usize>) -> Result<&mut RuntimeValue> {
        let slot = self.required_slot(name, slot, "mutable target")?;
        self.values[slot]
            .as_mut()
            .ok_or_else(|| RuntimeError::Decode(format!("unknown variable {name}")))
    }

    fn clear(&mut self, slots: &[usize]) {
        for slot in slots {
            if let Some(value) = self.values.get_mut(*slot) {
                *value = None;
            }
        }
    }

    fn required_slot(&self, name: &str, slot: Option<usize>, context: &str) -> Result<usize> {
        let slot = slot.ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!("slotted IR {context} {name} missing slot"))
        })?;
        if slot >= self.values.len() {
            return Err(RuntimeError::InvalidArtifact(format!(
                "slot {slot} for {context} {name} is out of bounds{}",
                self.debug_binding_suffix(slot)
            )));
        }
        Ok(slot)
    }

    fn debug_binding_suffix(&self, slot: usize) -> String {
        self.debug_bindings
            .iter()
            .find(|binding| binding.slot == slot)
            .map(|binding| format!("; binding {} ({})", binding.name, binding.kind))
            .unwrap_or_default()
    }

    fn unknown_slot_message(&self, slot: usize) -> String {
        self.debug_bindings
            .iter()
            .find(|binding| binding.slot == slot)
            .map(|binding| format!("unknown variable {}", binding.name))
            .unwrap_or_else(|| format!("unknown slot {slot}"))
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

fn program_slot_layout(
    slots: &SlotLayoutIr,
    executable: &LinkedExecutable,
) -> Result<RuntimeSlotLayout> {
    let count = slots.frame_size;
    if count == 0 && !executable.params.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "executable {} has parameters but an empty slot layout",
            executable.symbol
        )));
    }

    let bindings = slots
        .slots
        .iter()
        .map(|slot| RuntimeSlotBinding {
            slot: slot.index,
            name: slot.name.clone(),
            kind: slot.kind.clone(),
            scope: None,
        })
        .collect::<Vec<_>>();

    let mut self_slot = None;
    let mut parameter_slots = std::collections::HashMap::new();
    for parameter in &executable.params {
        parameter_slots
            .entry(parameter.name.clone())
            .or_insert(parameter.slot);
    }
    for binding in &bindings {
        if binding.name == "self" || binding.kind == "selfValue" {
            self_slot.get_or_insert(binding.slot);
        }
        if binding.kind == "param" {
            parameter_slots
                .entry(binding.name.clone())
                .or_insert(binding.slot);
        }
    }

    validate_program_slots(
        &executable.symbol,
        count,
        self_slot,
        &parameter_slots,
        &bindings,
    )?;
    Ok(RuntimeSlotLayout {
        count,
        bindings,
        self_slot,
        parameter_slots,
    })
}

fn validate_program_slots(
    executable: &str,
    count: usize,
    self_slot: Option<usize>,
    parameter_slots: &std::collections::HashMap<String, usize>,
    bindings: &[RuntimeSlotBinding],
) -> Result<()> {
    if let Some(slot) = self_slot {
        validate_program_slot(executable, "self", slot, count)?;
    }
    for (name, slot) in parameter_slots {
        validate_program_slot(executable, name, *slot, count)?;
    }
    for binding in bindings {
        validate_program_slot(executable, &binding.name, binding.slot, count)?;
    }
    Ok(())
}

fn validate_program_slot(executable: &str, name: &str, slot: usize, count: usize) -> Result<()> {
    if slot >= count {
        return Err(RuntimeError::InvalidArtifact(format!(
            "executable {executable} slot {slot} for {name} is out of bounds for frame size {count}"
        )));
    }
    Ok(())
}

fn program_parameter_slot(executable: &LinkedExecutable, name: &str) -> Option<usize> {
    executable
        .params
        .iter()
        .find(|parameter| parameter.name == name)
        .map(|parameter| parameter.slot)
        .or_else(|| {
            executable
                .slots
                .slots
                .iter()
                .find(|slot| slot.name == name && slot.kind == "param")
                .map(|slot| slot.index)
        })
}
