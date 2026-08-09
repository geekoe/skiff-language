use std::{fmt, num::NonZeroU32};

use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Terminal reason returned by the trusted execution-budget owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmBudgetError {
    InstructionLimitExceeded,
    DeadlineExceeded,
    Cancelled,
    InternalStop,
    AccountingFailure,
}

impl fmt::Display for VmBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InstructionLimitExceeded => "VM instruction limit exceeded",
            Self::DeadlineExceeded => "VM execution deadline exceeded",
            Self::Cancelled => "VM execution cancelled",
            Self::InternalStop => "VM execution stopped by the runtime",
            Self::AccountingFailure => "VM execution budget accounting failed",
        })
    }
}

impl std::error::Error for VmBudgetError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmSemanticChargeKind<'a> {
    FunctionEntry,
    Statement { statement_id: &'a str },
    LocalCall,
    TailCall,
    BudgetCheckpoint,
}

/// One stable language-level charge, independent of decoded micro-op count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmSemanticCharge<'a> {
    function: FunctionIndex,
    instruction: InstructionIndex,
    kind: VmSemanticChargeKind<'a>,
}

impl<'a> VmSemanticCharge<'a> {
    pub const fn new(
        function: FunctionIndex,
        instruction: InstructionIndex,
        kind: VmSemanticChargeKind<'a>,
    ) -> Self {
        Self {
            function,
            instruction,
            kind,
        }
    }

    pub const fn function(self) -> FunctionIndex {
        self.function
    }

    pub const fn instruction(self) -> InstructionIndex {
        self.instruction
    }

    pub const fn kind(self) -> VmSemanticChargeKind<'a> {
        self.kind
    }
}

/// Narrow synchronous budget port used by the VM dispatch loop.
///
/// `replenish_raw_fuel` owns the finite instruction limit. A grant must be in
/// `1..=maximum`; the VM rejects any larger grant instead of allowing policy
/// to weaken its trusted polling quantum. Replenishment also polls deadline
/// and internal stop before returning fuel.
pub trait VmBudget {
    fn replenish_raw_fuel(&mut self, maximum: NonZeroU32) -> Result<NonZeroU32, VmBudgetError>;

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetError>;

    fn charge_semantic(&mut self, charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetError>;
}
