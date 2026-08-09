use std::{fmt, num::NonZeroU32};

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{FrameSlotIndex, FunctionIndex, InstructionIndex, TypeIndex};
use skiff_runtime_model::{vm_heap::VmHeapError, vm_value::ValueKind};

use crate::{fiber::VmFiberState, VmBudgetError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmEntryArgumentRejection {
    InvalidMetadata,
    ImageScopedConstant,
    HeapTypeProofUnavailable,
    ActorState,
    AffineResource,
    CallbackClosure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmVerifiedInvariant {
    EntryFunctionMissing,
    FunctionIndexMismatch,
    EntryParameterCount,
    ParameterSlotCount,
    DuplicateParameterSlot,
    FrameSlotPlanCount,
    ParameterMode,
    ParameterTransferPlan,
    ParameterType,
    ResultType,
    ResultTransferPlan,
    ExternalInOutParameter,
    FrameLayoutOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmValueLocation {
    EntryArgument(usize),
    FrameSlot(FrameSlotIndex),
    Operand(usize),
}

/// Structured, fail-closed VM failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmError {
    EntryArgumentCountMismatch {
        expected: usize,
        actual: usize,
    },
    EntryArgumentRejected {
        ordinal: usize,
        kind: Option<ValueKind>,
        reason: VmEntryArgumentRejection,
    },
    EntryArgumentTypeMismatch {
        ordinal: usize,
        expected: TypeIndex,
        actual: Option<ValueKind>,
    },
    VerifiedEntryInvariant {
        invariant: VmVerifiedInvariant,
    },
    FrameLimitExceeded {
        limit: usize,
    },
    ValueStackLimitExceeded {
        limit: usize,
        requested: usize,
    },
    InvalidFuelGrant {
        requested_maximum: NonZeroU32,
        granted: NonZeroU32,
    },
    FiberNotRunnable {
        state: VmFiberState,
    },
    DiscardRequiresTerminal {
        state: VmFiberState,
    },
    TerminalRootLifecycleUnavailable {
        index: usize,
        kind: Option<ValueKind>,
    },
    ResumeNotExpected,
    ResumeTokenMismatch,
    ResumeShapeMismatch {
        expected: usize,
        actual: usize,
    },
    InstructionPointerOutOfBounds {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    MalformedInstruction {
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
        expected_operands: usize,
        actual_operands: usize,
    },
    UnsupportedOpcode {
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    },
    FullValueLifecyclePlanUnavailable {
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    },
    SlotOutOfBounds {
        function: FunctionIndex,
        slot: FrameSlotIndex,
    },
    DeadValueRead {
        location: VmValueLocation,
    },
    LiveDestination {
        location: VmValueLocation,
    },
    OperandStackUnderflow {
        function: FunctionIndex,
        needed: usize,
        available: usize,
    },
    OperandStackOverflow {
        function: FunctionIndex,
        capacity: usize,
    },
    OperandStackShapeMismatch {
        function: FunctionIndex,
        expected: usize,
        actual: usize,
    },
    ExpectedBoolean {
        function: FunctionIndex,
        instruction: InstructionIndex,
        actual: Option<ValueKind>,
    },
    BranchTargetOutOfBounds {
        function: FunctionIndex,
        target: InstructionIndex,
    },
    ConstantIndexOutOfBounds {
        function: FunctionIndex,
        instruction: InstructionIndex,
        index: u32,
    },
    Heap(VmHeapError),
    Budget(VmBudgetError),
}

impl fmt::Display for VmError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntryArgumentCountMismatch { expected, actual } => write!(
                formatter,
                "VM entry expects {expected} arguments but received {actual}"
            ),
            Self::EntryArgumentRejected {
                ordinal,
                kind,
                reason,
            } => write!(
                formatter,
                "VM entry argument {ordinal} with kind {kind:?} was rejected: {reason:?}"
            ),
            Self::EntryArgumentTypeMismatch {
                ordinal,
                expected,
                actual,
            } => write!(
                formatter,
                "VM entry argument {ordinal} with kind {actual:?} does not exactly match verified type {}",
                expected.get()
            ),
            Self::VerifiedEntryInvariant { invariant } => {
                write!(formatter, "verified VM entry invariant failed: {invariant:?}")
            }
            Self::FrameLimitExceeded { limit } => {
                write!(formatter, "VM frame limit {limit} exceeded")
            }
            Self::ValueStackLimitExceeded { limit, requested } => write!(
                formatter,
                "VM value stack limit {limit} cannot satisfy requested length {requested}"
            ),
            Self::InvalidFuelGrant {
                requested_maximum,
                granted,
            } => write!(
                formatter,
                "VM budget granted {granted} raw instructions above requested maximum {requested_maximum}"
            ),
            Self::FiberNotRunnable { state } => {
                write!(formatter, "VM fiber is not runnable (state {state:?})")
            }
            Self::DiscardRequiresTerminal { state } => write!(
                formatter,
                "VM roots can only be discarded after terminal state (state {state:?})"
            ),
            Self::TerminalRootLifecycleUnavailable { index, kind } => write!(
                formatter,
                "VM terminal root {index} with kind {kind:?} cannot be discarded without its full lifecycle plan"
            ),
            Self::ResumeNotExpected => formatter.write_str("VM fiber has no pending resume"),
            Self::ResumeTokenMismatch => {
                formatter.write_str("VM resume token does not match the pending continuation")
            }
            Self::ResumeShapeMismatch { expected, actual } => write!(
                formatter,
                "VM resume expects {expected} values but received {actual}"
            ),
            Self::InstructionPointerOutOfBounds {
                function,
                instruction,
            } => write!(
                formatter,
                "VM function {} has no instruction {}",
                function.get(),
                instruction.get()
            ),
            Self::MalformedInstruction {
                function,
                instruction,
                opcode,
                expected_operands,
                actual_operands,
            } => write!(
                formatter,
                "VM function {} instruction {} ({opcode:?}) has {actual_operands} operands; expected {expected_operands}",
                function.get(),
                instruction.get()
            ),
            Self::UnsupportedOpcode {
                function,
                instruction,
                opcode,
            } => write!(
                formatter,
                "VM function {} instruction {} uses unsupported opcode {opcode:?}",
                function.get(),
                instruction.get()
            ),
            Self::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            } => write!(
                formatter,
                "VM function {} instruction {} ({opcode:?}) requires a full linked value lifecycle plan",
                function.get(),
                instruction.get()
            ),
            Self::SlotOutOfBounds { function, slot } => write!(
                formatter,
                "VM function {} has no frame slot {}",
                function.get(),
                slot.get()
            ),
            Self::DeadValueRead { location } => {
                write!(formatter, "VM attempted to read dead value at {location:?}")
            }
            Self::LiveDestination { location } => write!(
                formatter,
                "VM instruction requires a dead destination at {location:?}"
            ),
            Self::OperandStackUnderflow {
                function,
                needed,
                available,
            } => write!(
                formatter,
                "VM function {} needs {needed} operands but only {available} are live",
                function.get()
            ),
            Self::OperandStackOverflow { function, capacity } => write!(
                formatter,
                "VM function {} exceeded verified operand capacity {capacity}",
                function.get()
            ),
            Self::OperandStackShapeMismatch {
                function,
                expected,
                actual,
            } => write!(
                formatter,
                "VM function {} expected operand depth {expected} but found {actual}",
                function.get()
            ),
            Self::ExpectedBoolean {
                function,
                instruction,
                actual,
            } => write!(
                formatter,
                "VM function {} instruction {} expected bool but found {actual:?}",
                function.get(),
                instruction.get()
            ),
            Self::BranchTargetOutOfBounds { function, target } => write!(
                formatter,
                "VM function {} branch target {} is out of bounds",
                function.get(),
                target.get()
            ),
            Self::ConstantIndexOutOfBounds {
                function,
                instruction,
                index,
            } => write!(
                formatter,
                "VM function {} instruction {} references missing constant {index}",
                function.get(),
                instruction.get()
            ),
            Self::Heap(error) => write!(formatter, "VM heap operation failed: {error}"),
            Self::Budget(error) => write!(formatter, "VM budget operation failed: {error}"),
        }
    }
}

impl std::error::Error for VmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Heap(error) => Some(error),
            Self::Budget(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VmHeapError> for VmError {
    fn from(error: VmHeapError) -> Self {
        Self::Heap(error)
    }
}

impl From<VmBudgetError> for VmError {
    fn from(error: VmBudgetError) -> Self {
        Self::Budget(error)
    }
}
