use skiff_artifact_model::{InstructionSourceSite, StatementAttributionId, StatementChargeKind};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Immutable verifier-owned semantic charging schedule.
///
/// Function ordinals and instruction ordinals are dense. Each function owns
/// an `I + 1` offset table, so an instruction lookup is O(1) and returns its
/// same-PC event range without scanning candidate rows.
#[derive(Debug)]
pub struct VerifiedStatementSchedule {
    pub(super) functions: Box<[VerifiedFunctionStatementSchedule]>,
    pub(super) total_event_count: usize,
}

#[derive(Debug)]
pub(super) struct VerifiedFunctionStatementSchedule {
    pub(super) frame_entry_charge_kind: StatementChargeKind,
    pub(super) instruction_offsets: Box<[usize]>,
    pub(super) events: Box<[VerifiedStatementEvent]>,
}

/// One authenticated source event with its verifier-derived semantic charge.
#[derive(Debug)]
pub struct VerifiedStatementEvent {
    pub(super) sequence_ordinal: u32,
    pub(super) attribution_id: StatementAttributionId,
    pub(super) site: InstructionSourceSite,
    pub(super) charge_kind: StatementChargeKind,
}

impl VerifiedStatementSchedule {
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    pub const fn total_event_count(&self) -> usize {
        self.total_event_count
    }

    pub fn instruction_count(&self, function: FunctionIndex) -> Option<usize> {
        self.function(function)
            .and_then(|schedule| schedule.instruction_offsets.len().checked_sub(1))
    }

    pub fn frame_entry_charge_kind(&self, function: FunctionIndex) -> Option<StatementChargeKind> {
        self.function(function)
            .map(|schedule| schedule.frame_entry_charge_kind)
    }

    pub fn events_for_function(
        &self,
        function: FunctionIndex,
    ) -> Option<&[VerifiedStatementEvent]> {
        self.function(function)
            .map(|schedule| schedule.events.as_ref())
    }

    pub fn events_at(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Option<&[VerifiedStatementEvent]> {
        let schedule = self.function(function)?;
        let ordinal = instruction.get() as usize;
        let start = *schedule.instruction_offsets.get(ordinal)?;
        let end = *schedule.instruction_offsets.get(ordinal.checked_add(1)?)?;
        schedule.events.get(start..end)
    }

    fn function(&self, function: FunctionIndex) -> Option<&VerifiedFunctionStatementSchedule> {
        self.functions.get(function.get() as usize)
    }
}

impl VerifiedStatementEvent {
    pub const fn sequence_ordinal(&self) -> u32 {
        self.sequence_ordinal
    }

    pub const fn attribution_id(&self) -> StatementAttributionId {
        self.attribution_id
    }

    pub const fn site(&self) -> &InstructionSourceSite {
        &self.site
    }

    pub const fn charge_kind(&self) -> StatementChargeKind {
        self.charge_kind
    }
}
