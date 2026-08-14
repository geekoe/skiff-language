use std::collections::BTreeSet;
use std::fmt;

use crate::{
    ActiveRegionIndex, FrameSlotIndex, InstructionIndex, LinkedValueTransferPlan, TypeIndex,
    WritablePathIndex,
};

/// One linker-computed typed operand-stack state at a function program point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedStackValue {
    ty: TypeIndex,
    plan: LinkedValueTransferPlan,
}

impl LinkedStackValue {
    pub fn new(ty: TypeIndex, plan: LinkedValueTransferPlan) -> Self {
        Self { ty, plan }
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn plan(&self) -> &LinkedValueTransferPlan {
        &self.plan
    }
}

/// Claimed slot liveness before an instruction. `Moved` is distinct from a
/// never-initialized slot so diagnostics retain the exact ownership state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedSlotState {
    Uninitialized,
    Moved,
    Live(LinkedStackValue),
}

/// Claimed exclusive writable loan at one program point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedWritableLoanState {
    root_slot: FrameSlotIndex,
    path: WritablePathIndex,
}

impl LinkedWritableLoanState {
    pub const fn new(root_slot: FrameSlotIndex, path: WritablePathIndex) -> Self {
        Self { root_slot, path }
    }

    pub const fn root_slot(&self) -> FrameSlotIndex {
        self.root_slot
    }

    pub const fn path(&self) -> WritablePathIndex {
        self.path
    }
}

/// Linker-produced state at instruction entry after bounded instruction
/// transfer and exact CFG predecessor/merge checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedProgramPointState {
    instruction: InstructionIndex,
    stack_before: Box<[LinkedStackValue]>,
    slots_before: Box<[LinkedSlotState]>,
    active_regions: Box<[ActiveRegionIndex]>,
    writable_loans: Box<[LinkedWritableLoanState]>,
}

impl LinkedProgramPointState {
    pub fn new(
        instruction: InstructionIndex,
        stack_before: Box<[LinkedStackValue]>,
        slots_before: Box<[LinkedSlotState]>,
        active_regions: Box<[ActiveRegionIndex]>,
        writable_loans: Box<[LinkedWritableLoanState]>,
    ) -> Self {
        Self {
            instruction,
            stack_before,
            slots_before,
            active_regions,
            writable_loans,
        }
    }

    pub const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub fn stack_before(&self) -> &[LinkedStackValue] {
        &self.stack_before
    }

    pub fn slots_before(&self) -> &[LinkedSlotState] {
        &self.slots_before
    }

    pub fn active_regions(&self) -> &[ActiveRegionIndex] {
        &self.active_regions
    }

    pub fn writable_loans(&self) -> &[LinkedWritableLoanState] {
        &self.writable_loans
    }
}

/// Dense untrusted per-instruction state sidecar for one linked function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedStackMapCandidate {
    entries: Box<[LinkedProgramPointState]>,
}

impl LinkedStackMapCandidate {
    pub fn try_new(
        entries: Box<[LinkedProgramPointState]>,
        instruction_count: usize,
        slot_count: usize,
        max_operand_depth: u32,
    ) -> Result<Self, LinkedStackMapCandidateError> {
        if entries.len() != instruction_count {
            return Err(LinkedStackMapCandidateError::InstructionCountMismatch {
                instruction_count,
                state_count: entries.len(),
            });
        }
        for (position, entry) in entries.iter().enumerate() {
            let expected = u32::try_from(position).map_err(|_| {
                LinkedStackMapCandidateError::InstructionCountExceedsU32 { instruction_count }
            })?;
            if entry.instruction().get() != expected {
                return Err(LinkedStackMapCandidateError::NonDenseInstruction {
                    position,
                    expected,
                    actual: entry.instruction().get(),
                });
            }
            if entry.slots_before().len() != slot_count {
                return Err(LinkedStackMapCandidateError::SlotCountMismatch {
                    instruction: entry.instruction(),
                    slot_count,
                    state_count: entry.slots_before().len(),
                });
            }
            if entry.stack_before().len() > max_operand_depth as usize {
                return Err(LinkedStackMapCandidateError::OperandDepthExceeded {
                    instruction: entry.instruction(),
                    declared_max: max_operand_depth,
                    actual: entry.stack_before().len(),
                });
            }
            validate_strictly_ascending_regions(entry)?;
            validate_strictly_ascending_loans(entry)?;
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[LinkedProgramPointState] {
        &self.entries
    }
}

fn validate_strictly_ascending_regions(
    entry: &LinkedProgramPointState,
) -> Result<(), LinkedStackMapCandidateError> {
    let mut previous = None;
    for region in entry.active_regions() {
        if let Some(previous) = previous {
            if previous >= *region {
                return Err(
                    LinkedStackMapCandidateError::NonCanonicalActiveRegionOrder {
                        instruction: entry.instruction(),
                        previous,
                        current: *region,
                    },
                );
            }
        }
        previous = Some(*region);
    }
    Ok(())
}

fn validate_strictly_ascending_loans(
    entry: &LinkedProgramPointState,
) -> Result<(), LinkedStackMapCandidateError> {
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for loan in entry.writable_loans() {
        if !seen.insert(*loan) || previous.is_some_and(|previous| previous >= *loan) {
            return Err(
                LinkedStackMapCandidateError::NonCanonicalWritableLoanOrder {
                    instruction: entry.instruction(),
                    current: *loan,
                },
            );
        }
        previous = Some(*loan);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedStackMapCandidateError {
    InstructionCountMismatch {
        instruction_count: usize,
        state_count: usize,
    },
    InstructionCountExceedsU32 {
        instruction_count: usize,
    },
    NonDenseInstruction {
        position: usize,
        expected: u32,
        actual: u32,
    },
    SlotCountMismatch {
        instruction: InstructionIndex,
        slot_count: usize,
        state_count: usize,
    },
    OperandDepthExceeded {
        instruction: InstructionIndex,
        declared_max: u32,
        actual: usize,
    },
    NonCanonicalActiveRegionOrder {
        instruction: InstructionIndex,
        previous: ActiveRegionIndex,
        current: ActiveRegionIndex,
    },
    NonCanonicalWritableLoanOrder {
        instruction: InstructionIndex,
        current: LinkedWritableLoanState,
    },
}

impl fmt::Display for LinkedStackMapCandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstructionCountMismatch {
                instruction_count,
                state_count,
            } => write!(
                formatter,
                "function has {instruction_count} instructions but {state_count} program-point states"
            ),
            Self::InstructionCountExceedsU32 { instruction_count } => write!(
                formatter,
                "function instruction count {instruction_count} exceeds u32"
            ),
            Self::NonDenseInstruction {
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "program-point state {position} names instruction {actual}; expected {expected}"
            ),
            Self::SlotCountMismatch {
                instruction,
                slot_count,
                state_count,
            } => write!(
                formatter,
                "instruction {} has {state_count} slot states but frame has {slot_count} slots",
                instruction.get()
            ),
            Self::OperandDepthExceeded {
                instruction,
                declared_max,
                actual,
            } => write!(
                formatter,
                "instruction {} claims operand depth {actual}, exceeding declared max {declared_max}",
                instruction.get()
            ),
            Self::NonCanonicalActiveRegionOrder {
                instruction,
                previous,
                current,
            } => write!(
                formatter,
                "instruction {} active region {} must sort after {}",
                instruction.get(),
                current.get(),
                previous.get()
            ),
            Self::NonCanonicalWritableLoanOrder {
                instruction,
                current,
            } => write!(
                formatter,
                "instruction {} writable loan ({}, {}) is duplicate or noncanonical",
                instruction.get(),
                current.root_slot().get(),
                current.path().get()
            ),
        }
    }
}

impl std::error::Error for LinkedStackMapCandidateError {}
