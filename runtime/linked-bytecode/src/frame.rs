use std::collections::BTreeSet;
use std::fmt;

use skiff_artifact_model::ParamModeIr;

use crate::{FrameSlotIndex, LinkedValueTransferPlan, TypeIndex};

/// One parameter's exact frame slot, calling mode and concrete lifecycle plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedParameterSlot {
    slot: FrameSlotIndex,
    mode: ParamModeIr,
    plan: LinkedValueTransferPlan,
}

impl LinkedParameterSlot {
    pub fn new(slot: FrameSlotIndex, mode: ParamModeIr, plan: LinkedValueTransferPlan) -> Self {
        Self { slot, mode, plan }
    }

    pub const fn slot(&self) -> FrameSlotIndex {
        self.slot
    }

    pub const fn mode(&self) -> ParamModeIr {
        self.mode
    }

    pub const fn plan(&self) -> &LinkedValueTransferPlan {
        &self.plan
    }
}

/// Concrete frame types and complete declarative lifecycle plans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFrameLayout {
    slot_types: Box<[TypeIndex]>,
    parameters: Box<[LinkedParameterSlot]>,
    writable_local_slots: Box<[FrameSlotIndex]>,
    result_types: Box<[TypeIndex]>,
    slot_plans: Box<[LinkedValueTransferPlan]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    stream_result_type_ref: Option<TypeIndex>,
}

impl LinkedFrameLayout {
    pub fn new(
        slot_types: Box<[TypeIndex]>,
        parameters: Box<[LinkedParameterSlot]>,
        writable_local_slots: Box<[FrameSlotIndex]>,
        result_types: Box<[TypeIndex]>,
        slot_plans: Box<[LinkedValueTransferPlan]>,
        result_plans: Box<[LinkedValueTransferPlan]>,
        stream_result_type_ref: Option<TypeIndex>,
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
        for (parameter_ordinal, parameter) in parameters.iter().enumerate() {
            let slot = parameter.slot();
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
            if &slot_plans[slot.get() as usize] != parameter.plan() {
                return Err(LinkedFrameLayoutError::ParameterPlanMismatch {
                    parameter_ordinal,
                    slot,
                });
            }
        }

        let mut previous = None;
        for slot in &writable_local_slots {
            if slot.get() as usize >= slot_types.len() {
                return Err(LinkedFrameLayoutError::WritableLocalSlotOutOfBounds {
                    slot: *slot,
                    slot_count: slot_types.len(),
                });
            }
            if seen.contains(slot) {
                return Err(LinkedFrameLayoutError::WritableLocalIsParameter { slot: *slot });
            }
            if let Some(previous) = previous {
                if previous >= *slot {
                    return Err(LinkedFrameLayoutError::NonCanonicalWritableLocalSlotOrder {
                        previous,
                        current: *slot,
                    });
                }
            }
            previous = Some(*slot);
        }

        Ok(Self {
            slot_types,
            parameters,
            writable_local_slots,
            result_types,
            slot_plans,
            result_plans,
            stream_result_type_ref,
        })
    }

    pub fn slot_types(&self) -> &[TypeIndex] {
        &self.slot_types
    }

    pub fn parameters(&self) -> &[LinkedParameterSlot] {
        &self.parameters
    }

    pub fn writable_local_slots(&self) -> &[FrameSlotIndex] {
        &self.writable_local_slots
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn slot_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.slot_plans
    }

    pub fn result_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.result_plans
    }

    /// Explicit stream producer authority. It is never inferred from ordinary
    /// result slots; a producer frame also carries zero ordinary results.
    pub const fn stream_result_type_ref(&self) -> Option<TypeIndex> {
        self.stream_result_type_ref
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
    ParameterPlanMismatch {
        parameter_ordinal: usize,
        slot: FrameSlotIndex,
    },
    WritableLocalSlotOutOfBounds {
        slot: FrameSlotIndex,
        slot_count: usize,
    },
    WritableLocalIsParameter {
        slot: FrameSlotIndex,
    },
    NonCanonicalWritableLocalSlotOrder {
        previous: FrameSlotIndex,
        current: FrameSlotIndex,
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
            Self::ParameterPlanMismatch {
                parameter_ordinal,
                slot,
            } => write!(
                formatter,
                "parameter {parameter_ordinal} plan does not match slot {} plan",
                slot.get()
            ),
            Self::WritableLocalSlotOutOfBounds { slot, slot_count } => write!(
                formatter,
                "writable local slot {} is outside a frame with {slot_count} slots",
                slot.get()
            ),
            Self::WritableLocalIsParameter { slot } => write!(
                formatter,
                "writable local slot {} is also an incoming parameter slot",
                slot.get()
            ),
            Self::NonCanonicalWritableLocalSlotOrder { previous, current } => write!(
                formatter,
                "writable local slot {} must sort after {}",
                current.get(),
                previous.get()
            ),
        }
    }
}

impl std::error::Error for LinkedFrameLayoutError {}
