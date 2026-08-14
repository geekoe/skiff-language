use skiff_artifact_model::{
    contract_for_opcode, default_statement_charge_kind_for_attribution, InstructionSourceSite,
    StatementAttributionId, StatementChargeKind, StatementContract, FRAME_ENTRY_STATEMENT_CONTRACT,
};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedBytecodeCandidate};

use super::ExecutionImageConstructionError;

/// Dense image-local index over compiler-emitted statement rows.
#[derive(Debug)]
pub struct ExecutionStatementSchedule {
    functions: Box<[FunctionStatementSchedule]>,
    total_event_count: usize,
}

#[derive(Debug)]
struct FunctionStatementSchedule {
    frame_entry_charge_kind: StatementChargeKind,
    instruction_offsets: Box<[usize]>,
    events: Box<[ExecutionStatementEvent]>,
}

#[derive(Debug)]
pub struct ExecutionStatementEvent {
    sequence_ordinal: u32,
    attribution_id: StatementAttributionId,
    site: InstructionSourceSite,
    charge_kind: StatementChargeKind,
}

impl ExecutionStatementSchedule {
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
    ) -> Option<&[ExecutionStatementEvent]> {
        self.function(function)
            .map(|schedule| schedule.events.as_ref())
    }

    pub fn events_at(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Option<&[ExecutionStatementEvent]> {
        let schedule = self.function(function)?;
        let ordinal = instruction.get() as usize;
        let start = *schedule.instruction_offsets.get(ordinal)?;
        let end = *schedule.instruction_offsets.get(ordinal.checked_add(1)?)?;
        schedule.events.get(start..end)
    }

    fn function(&self, function: FunctionIndex) -> Option<&FunctionStatementSchedule> {
        self.functions.get(function.get() as usize)
    }
}

impl ExecutionStatementEvent {
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

pub(in crate::bytecode) fn build_statement_schedule(
    linked: &LinkedBytecodeCandidate,
) -> Result<ExecutionStatementSchedule, ExecutionImageConstructionError> {
    let mut functions = Vec::with_capacity(linked.functions().len());
    let mut total_event_count = 0_usize;
    for function in linked.functions() {
        let offset_capacity = function.instructions().len().checked_add(1).ok_or(
            ExecutionImageConstructionError::StatementScheduleOverflow {
                function: function.index(),
            },
        )?;
        let mut instruction_offsets = Vec::with_capacity(offset_capacity);
        let mut events = Vec::with_capacity(function.statement_entries().len());
        let mut cursor = 0_usize;
        for (ordinal, instruction) in function.instructions().iter().enumerate() {
            instruction_offsets.push(events.len());
            let instruction_index =
                u32::try_from(ordinal)
                    .map(InstructionIndex::new)
                    .map_err(
                        |_| ExecutionImageConstructionError::StatementScheduleOverflow {
                            function: function.index(),
                        },
                    )?;
            let start = cursor;
            while function
                .statement_entries()
                .get(cursor)
                .is_some_and(|entry| entry.instruction() == instruction_index)
            {
                cursor = cursor.checked_add(1).ok_or(
                    ExecutionImageConstructionError::StatementScheduleOverflow {
                        function: function.index(),
                    },
                )?;
            }
            append_instruction_events(
                function.index(),
                instruction_index,
                instruction.opcode(),
                &function.statement_entries()[start..cursor],
                &mut events,
            )?;
        }
        if cursor != function.statement_entries().len() {
            return Err(
                ExecutionImageConstructionError::StatementInstructionOutOfBounds {
                    function: function.index(),
                    instruction: function.statement_entries()[cursor].instruction(),
                },
            );
        }
        instruction_offsets.push(events.len());
        total_event_count = total_event_count.checked_add(events.len()).ok_or(
            ExecutionImageConstructionError::StatementScheduleOverflow {
                function: function.index(),
            },
        )?;
        functions.push(FunctionStatementSchedule {
            frame_entry_charge_kind: FRAME_ENTRY_STATEMENT_CONTRACT.charge_kind,
            instruction_offsets: instruction_offsets.into_boxed_slice(),
            events: events.into_boxed_slice(),
        });
    }
    Ok(ExecutionStatementSchedule {
        functions: functions.into_boxed_slice(),
        total_event_count,
    })
}

fn append_instruction_events(
    function: FunctionIndex,
    instruction: InstructionIndex,
    opcode: skiff_artifact_model::Opcode,
    entries: &[skiff_runtime_linked_bytecode::LinkedStatementEntry],
    events: &mut Vec<ExecutionStatementEvent>,
) -> Result<(), ExecutionImageConstructionError> {
    let contract = contract_for_opcode(opcode);
    let required = match contract.statement {
        StatementContract::None => None,
        StatementContract::RequiredEvent {
            charge_kind,
            attribution,
        } => {
            let matching = entries
                .iter()
                .filter(|entry| entry.attribution_id().class() == attribution)
                .count();
            if matching != 1 {
                return Err(ExecutionImageConstructionError::StatementContractMismatch {
                    function,
                    instruction,
                    opcode,
                    expected_attribution: attribution,
                    matching,
                });
            }
            Some((attribution, charge_kind))
        }
    };
    events.extend(entries.iter().map(|entry| {
        let class = entry.attribution_id().class();
        let charge_kind = required
            .filter(|(required_class, _)| *required_class == class)
            .map_or_else(
                || default_statement_charge_kind_for_attribution(class),
                |(_, charge_kind)| charge_kind,
            );
        ExecutionStatementEvent {
            sequence_ordinal: entry.sequence_ordinal(),
            attribution_id: entry.attribution_id(),
            site: entry.site().clone(),
            charge_kind,
        }
    }));
    Ok(())
}
