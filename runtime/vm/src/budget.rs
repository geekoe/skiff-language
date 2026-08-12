use std::fmt;

use skiff_artifact_model::{InstructionSourceSite, StatementAttributionId, StatementChargeKind};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Frozen request terminal already selected by the trusted budget owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmBudgetTerminal {
    Succeeded,
    Failed,
    Cancelled,
    InstructionLimitExceeded,
    DeadlineExceeded,
    InternalStop,
    AccountingFailure,
}

/// A closed budget result. Direct variants were selected by the current VM
/// call; `AlreadySettled` preserves the request winner selected elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmBudgetClosed {
    DeadlineExceeded,
    InstructionLimitExceeded,
    AccountingFailure,
    AlreadySettled(VmBudgetTerminal),
}

impl fmt::Display for VmBudgetClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InstructionLimitExceeded => "VM instruction limit exceeded",
            Self::DeadlineExceeded => "VM execution deadline exceeded",
            Self::AccountingFailure => "VM execution budget accounting failed",
            Self::AlreadySettled(VmBudgetTerminal::Succeeded) => {
                "VM execution already completed successfully"
            }
            Self::AlreadySettled(VmBudgetTerminal::Failed) => "VM execution already failed",
            Self::AlreadySettled(VmBudgetTerminal::Cancelled) => "VM execution already cancelled",
            Self::AlreadySettled(VmBudgetTerminal::DeadlineExceeded) => {
                "VM execution deadline already exceeded"
            }
            Self::AlreadySettled(VmBudgetTerminal::InstructionLimitExceeded) => {
                "VM instruction limit was already exceeded"
            }
            Self::AlreadySettled(VmBudgetTerminal::InternalStop) => {
                "VM execution was already stopped by the runtime"
            }
            Self::AlreadySettled(VmBudgetTerminal::AccountingFailure) => {
                "VM execution budget accounting already failed"
            }
        })
    }
}

impl std::error::Error for VmBudgetClosed {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmSemanticChargeKind<'a> {
    FunctionEntry,
    SourceEvent {
        sequence_ordinal: u32,
        attribution_id: StatementAttributionId,
        site: &'a InstructionSourceSite,
        charge_kind: StatementChargeKind,
    },
}

/// One stable language-level charge, independent of decoded micro-op count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmSemanticCharge<'a> {
    function: FunctionIndex,
    instruction: InstructionIndex,
    kind: VmSemanticChargeKind<'a>,
}

impl<'a> VmSemanticCharge<'a> {
    pub(crate) const fn function_entry(
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Self {
        Self {
            function,
            instruction,
            kind: VmSemanticChargeKind::FunctionEntry,
        }
    }

    pub(crate) const fn source_event(
        function: FunctionIndex,
        instruction: InstructionIndex,
        sequence_ordinal: u32,
        attribution_id: StatementAttributionId,
        site: &'a InstructionSourceSite,
        charge_kind: StatementChargeKind,
    ) -> Self {
        Self {
            function,
            instruction,
            kind: VmSemanticChargeKind::SourceEvent {
                sequence_ordinal,
                attribution_id,
                site,
                charge_kind,
            },
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
/// An error from `charge_semantic` means that semantic unit was not committed.
/// The VM retains its same-PC event cursor so a permitted retry starts at that
/// exact event rather than replaying an already committed prefix.
pub trait VmBudget {
    /// Atomically authorizes and charges exactly one attempted dispatch.
    fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed>;

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed>;

    fn charge_semantic(&mut self, charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed>;
}
