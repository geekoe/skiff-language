use std::collections::HashMap;

use skiff_runtime_linked_program::{LinkedExecutable, SlotLayoutIr};
use skiff_runtime_model::runtime_value::RuntimeValueCarrier;

use crate::error::{Result, RuntimeError};

#[derive(Clone, Debug)]
pub struct SlotStore {
    pub(super) values: Vec<Option<RuntimeValueCarrier>>,
    pub(super) debug_bindings: Vec<SlotDebugBinding>,
    pub(super) self_slot: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct SlotDebugBinding {
    pub slot: usize,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeSlotLayout {
    pub(super) count: usize,
    pub(super) bindings: Vec<RuntimeSlotBinding>,
    pub(super) self_slot: Option<usize>,
    pub(super) parameter_slots: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeSlotBinding {
    pub(super) slot: usize,
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) scope: Option<usize>,
}

impl SlotStore {
    pub(super) fn empty() -> Self {
        Self {
            values: Vec::new(),
            debug_bindings: Vec::new(),
            self_slot: None,
        }
    }

    pub(super) fn declare(
        &mut self,
        name: &str,
        slot: Option<usize>,
        value: RuntimeValueCarrier,
    ) -> Result<()> {
        let slot = self.required_slot(name, slot, "binding")?;
        self.values[slot] = Some(value);
        Ok(())
    }

    pub(super) fn assign(
        &mut self,
        name: &str,
        slot: Option<usize>,
        value: RuntimeValueCarrier,
    ) -> Result<()> {
        let slot = self.required_slot(name, slot, "assignment target")?;
        if self.values[slot].is_none() {
            return Err(RuntimeError::Decode(format!("unknown variable {name}")));
        }
        self.values[slot] = Some(value);
        Ok(())
    }

    pub(super) fn get(&self, name: &str, slot: Option<usize>) -> Result<RuntimeValueCarrier> {
        let slot = self.required_slot(name, slot, "identifier")?;
        self.values[slot]
            .clone()
            .ok_or_else(|| RuntimeError::Decode(format!("unknown variable {name}")))
    }

    pub(super) fn get_slot(&self, slot: usize) -> Result<RuntimeValueCarrier> {
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

    pub(super) fn get_mut(
        &mut self,
        name: &str,
        slot: Option<usize>,
    ) -> Result<&mut RuntimeValueCarrier> {
        let slot = self.required_slot(name, slot, "mutable target")?;
        self.values[slot]
            .as_mut()
            .ok_or_else(|| RuntimeError::Decode(format!("unknown variable {name}")))
    }

    pub(super) fn clear(&mut self, slots: &[usize]) {
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

pub(super) fn program_slot_layout(
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
    let mut parameter_slots = HashMap::new();
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
    parameter_slots: &HashMap<String, usize>,
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

pub(super) fn program_parameter_slot(executable: &LinkedExecutable, name: &str) -> Option<usize> {
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
