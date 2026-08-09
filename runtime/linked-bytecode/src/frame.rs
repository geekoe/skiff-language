use std::collections::BTreeSet;
use std::fmt;

use skiff_artifact_model::ValueTransferPlanKind;

use crate::{FrameSlotIndex, TypeIndex};

/// Concrete frame types and declarative transfer plans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFrameLayout {
    slot_types: Box<[TypeIndex]>,
    parameter_slots: Box<[FrameSlotIndex]>,
    result_types: Box<[TypeIndex]>,
    slot_plans: Box<[ValueTransferPlanKind]>,
    result_plans: Box<[ValueTransferPlanKind]>,
}

impl LinkedFrameLayout {
    pub fn new(
        slot_types: Box<[TypeIndex]>,
        parameter_slots: Box<[FrameSlotIndex]>,
        result_types: Box<[TypeIndex]>,
        slot_plans: Box<[ValueTransferPlanKind]>,
        result_plans: Box<[ValueTransferPlanKind]>,
    ) -> Result<Self, LinkedFrameLayoutError> {
        if slot_types.len() != slot_plans.len() {
            return Err(LinkedFrameLayoutError::SlotPlanCountMismatch {
                slot_type_count: slot_types.len(),
                slot_plan_count: slot_plans.len(),
            });
        }
        if result_types.len() != result_plans.len() {
            return Err(LinkedFrameLayoutError::ResultPlanCountMismatch {
                result_type_count: result_types.len(),
                result_plan_count: result_plans.len(),
            });
        }

        let mut seen = BTreeSet::new();
        for (parameter_ordinal, slot) in parameter_slots.iter().copied().enumerate() {
            if slot.get() as usize >= slot_types.len() {
                return Err(LinkedFrameLayoutError::ParameterSlotOutOfBounds {
                    parameter_ordinal,
                    slot,
                    slot_count: slot_types.len(),
                });
            }
            if !seen.insert(slot) {
                return Err(LinkedFrameLayoutError::DuplicateParameterSlot { slot });
            }
        }

        Ok(Self {
            slot_types,
            parameter_slots,
            result_types,
            slot_plans,
            result_plans,
        })
    }

    pub fn slot_types(&self) -> &[TypeIndex] {
        &self.slot_types
    }

    pub fn parameter_slots(&self) -> &[FrameSlotIndex] {
        &self.parameter_slots
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn slot_plans(&self) -> &[ValueTransferPlanKind] {
        &self.slot_plans
    }

    pub fn result_plans(&self) -> &[ValueTransferPlanKind] {
        &self.result_plans
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedFrameLayoutError {
    SlotPlanCountMismatch {
        slot_type_count: usize,
        slot_plan_count: usize,
    },
    ResultPlanCountMismatch {
        result_type_count: usize,
        result_plan_count: usize,
    },
    ParameterSlotOutOfBounds {
        parameter_ordinal: usize,
        slot: FrameSlotIndex,
        slot_count: usize,
    },
    DuplicateParameterSlot {
        slot: FrameSlotIndex,
    },
}

impl fmt::Display for LinkedFrameLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SlotPlanCountMismatch {
                slot_type_count,
                slot_plan_count,
            } => write!(
                formatter,
                "frame has {slot_type_count} slot types but {slot_plan_count} slot plans"
            ),
            Self::ResultPlanCountMismatch {
                result_type_count,
                result_plan_count,
            } => write!(
                formatter,
                "frame has {result_type_count} result types but {result_plan_count} result plans"
            ),
            Self::ParameterSlotOutOfBounds {
                parameter_ordinal,
                slot,
                slot_count,
            } => write!(
                formatter,
                "parameter {parameter_ordinal} uses slot {} but frame has {slot_count} slots",
                slot.get()
            ),
            Self::DuplicateParameterSlot { slot } => {
                write!(formatter, "parameter slot {} is declared twice", slot.get())
            }
        }
    }
}

impl std::error::Error for LinkedFrameLayoutError {}
