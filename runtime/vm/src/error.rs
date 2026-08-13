use std::{fmt, sync::Arc};

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, CandidateTable, FrameSlotIndex, FunctionIndex, InstructionIndex, TypeIndex,
};
use skiff_runtime_model::{
    service_error::RequestException,
    vm_heap::VmHeapError,
    vm_value::ValueKind,
};

use crate::{fiber::VmFiberState, VmBudgetClosed, VmInternalTerminal};

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
    ProgramPointSlotCount,
    ParameterMode,
    ParameterTransferPlan,
    ParameterType,
    ResultType,
    ResultTransferPlan,
    ExternalInOutParameter,
    FrameLayoutOverflow,
    StatementScheduleFunctionMissing {
        function: FunctionIndex,
    },
    StatementScheduleInstructionMissing {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    StatementScheduleFrameEntryKind,
    StatementScheduleEventKind,
    StatementScheduleEventCursor,
    ChildFrameResumeMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmValueLocation {
    EntryArgument(usize),
    FrameSlot(FrameSlotIndex),
    Operand(usize),
}

/// Structured, fail-closed VM failure.
#[derive(Debug, Clone, PartialEq)]
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
    StreamEndResumeUnavailable,
    LinkedTableRowMissing {
        table: CandidateTable,
        row: u32,
    },
    AssertionFailed {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    /// Root-level uncaught throw. This is a typed outcome, not a VM failure:
    /// the scheduler and request driver project it to `ResumeOutcome::Throw`
    /// and the canonical user error respectively instead of a terminal
    /// failure. The envelope owns the payload slot.
    Thrown(Arc<RequestException>),
    /// The throw site could not construct the opaque exception envelope from
    /// the runtime value facts. This is a VmFailure; there is no static type
    /// fallback.
    ThrowEnvelopeUnavailable {
        function: FunctionIndex,
        instruction: InstructionIndex,
        reason: String,
    },
    /// A rethrow site has no caught envelope to reuse. Fail closed.
    RethrowEnvelopeUnavailable {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    /// A resume throw delivered an envelope that cannot unwind (missing opaque
    /// payload or actual identity).
    ResumeThrowEnvelopeUnavailable {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    RegionLeaveMismatch {
        function: FunctionIndex,
        instruction: InstructionIndex,
        expected: ActiveRegionIndex,
        actual: ActiveRegionIndex,
    },
    InternalTerminal(VmInternalTerminal),
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
        function: FunctionIndex,
        instruction: InstructionIndex,
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
    ExpectedNumber {
        function: FunctionIndex,
        instruction: InstructionIndex,
        actual: Option<ValueKind>,
    },
    ExpectedComparablePair {
        function: FunctionIndex,
        instruction: InstructionIndex,
        left: Option<ValueKind>,
        right: Option<ValueKind>,
    },
    ScalarNonFinite {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    DivideByZero {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    LocalCallTargetMismatch {
        function: FunctionIndex,
        instruction: InstructionIndex,
        target: FunctionIndex,
        expected_arguments: usize,
        actual_arguments: usize,
        expected_results: usize,
        actual_results: usize,
    },
    TailCallTargetMismatch {
        function: FunctionIndex,
        instruction: InstructionIndex,
        target: FunctionIndex,
        expected_arguments: usize,
        actual_arguments: usize,
        expected_results: usize,
        actual_results: usize,
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
    BudgetClosed(VmBudgetClosed),
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
            Self::StreamEndResumeUnavailable => formatter.write_str(
                "VM StreamEnd resume requires an end resume PC",
            ),
            Self::LinkedTableRowMissing { table, row } => write!(
                formatter,
                "verified VM references missing linked {table:?} row {row}"
            ),
            Self::AssertionFailed {
                function,
                instruction,
            } => write!(
                formatter,
                "VM function {} instruction {} assertion failed",
                function.get(),
                instruction.get()
            ),
            Self::Thrown(_) => {
                formatter.write_str("VM threw an uncaught request-local exception")
            }
            Self::ThrowEnvelopeUnavailable {
                function,
                instruction,
                reason,
            } => write!(
                formatter,
                "VM function {} instruction {} cannot construct the throw envelope: {reason}",
                function.get(),
                instruction.get()
            ),
            Self::RethrowEnvelopeUnavailable {
                function,
                instruction,
            } => write!(
                formatter,
                "VM function {} instruction {} rethrows without a caught envelope",
                function.get(),
                instruction.get()
            ),
            Self::ResumeThrowEnvelopeUnavailable {
                function,
                instruction,
            } => write!(
                formatter,
                "VM function {} instruction {} resumed with a throw envelope that cannot unwind",
                function.get(),
                instruction.get()
            ),
            Self::RegionLeaveMismatch {
                function,
                instruction,
                expected,
                actual,
            } => write!(
                formatter,
                "VM function {} instruction {} attempted to leave region {} while {} is active",
                function.get(),
                instruction.get(),
                expected.get(),
                actual.get()
            ),
            Self::InternalTerminal(reason) => {
                write!(formatter, "VM continuation terminated internally: {reason:?}")
            }
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
            Self::LiveDestination {
                function,
                instruction,
                location,
            } => write!(
                formatter,
                "VM function {} instruction {} requires a dead destination at {location:?}",
                function.get(),
                instruction.get()
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
            Self::ExpectedNumber {
                function,
                instruction,
                actual,
            } => write!(
                formatter,
                "VM function {} instruction {} expected number but found {actual:?}",
                function.get(),
                instruction.get()
            ),
            Self::ExpectedComparablePair {
                function,
                instruction,
                left,
                right,
            } => write!(
                formatter,
                "VM function {} instruction {} expected one exact comparable pair but found {left:?} and {right:?}",
                function.get(),
                instruction.get()
            ),
            Self::ScalarNonFinite {
                function,
                instruction,
            } => write!(
                formatter,
                "VM function {} instruction {} produced a non-finite scalar",
                function.get(),
                instruction.get()
            ),
            Self::DivideByZero {
                function,
                instruction,
            } => write!(
                formatter,
                "VM function {} instruction {} divided by zero",
                function.get(),
                instruction.get()
            ),
            Self::LocalCallTargetMismatch {
                function,
                instruction,
                target,
                expected_arguments,
                actual_arguments,
                expected_results,
                actual_results,
            } => write!(
                formatter,
                "VM function {} instruction {} local call target {} expects {expected_arguments} arguments and {expected_results} results but found {actual_arguments} arguments and {actual_results} results",
                function.get(),
                instruction.get(),
                target.get()
            ),
            Self::TailCallTargetMismatch {
                function,
                instruction,
                target,
                expected_arguments,
                actual_arguments,
                expected_results,
                actual_results,
            } => write!(
                formatter,
                "VM function {} instruction {} tail call target {} expects {expected_arguments} arguments and {expected_results} results but found {actual_arguments} arguments and {actual_results} results",
                function.get(),
                instruction.get(),
                target.get()
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
            Self::BudgetClosed(error) => write!(formatter, "VM budget is closed: {error}"),
        }
    }
}

impl std::error::Error for VmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Heap(error) => Some(error),
            Self::BudgetClosed(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VmHeapError> for VmError {
    fn from(error: VmHeapError) -> Self {
        Self::Heap(error)
    }
}

impl From<VmBudgetClosed> for VmError {
    fn from(error: VmBudgetClosed) -> Self {
        Self::BudgetClosed(error)
    }
}
