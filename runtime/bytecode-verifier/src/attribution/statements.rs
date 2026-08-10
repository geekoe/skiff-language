mod canonical;
mod schedule;
#[cfg(test)]
mod tests;

use skiff_artifact_model::{
    contract_for_opcode, default_statement_charge_kind_for_attribution, StatementChargeKind,
    StatementContract, FRAME_ENTRY_STATEMENT_CONTRACT,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFunction,
};

use super::source::SourceAttributionFacts;
use crate::{
    admission::ExactStatementBinding, control_flow::ControlFlowFacts, VerificationError,
    VerificationLimit, VerificationLimits, VerificationLocation, VerificationObligation,
};

use schedule::VerifiedFunctionStatementSchedule;
pub use schedule::{VerifiedStatementEvent, VerifiedStatementSchedule};

pub(crate) fn prove_statement_attribution(
    candidate: &LinkedBytecodeCandidate,
    binding: &ExactStatementBinding,
    source: &SourceAttributionFacts,
    flow: &ControlFlowFacts,
    limits: &VerificationLimits,
) -> Result<VerifiedStatementSchedule, VerificationError> {
    preflight(candidate, binding, source, limits)?;
    let mut functions = Vec::with_capacity(candidate.functions().len());
    let mut total_event_count = 0_usize;

    for (ordinal, function) in candidate.functions().iter().enumerate() {
        prove_dense_function(function, ordinal)?;
        let exact = binding.function(function.index()).ok_or_else(|| {
            violation(
                function_location(function),
                "P1 statement binding is not dense with the candidate function",
            )
        })?;
        if !flow.proves_function_shape(function.index(), function.instructions().len()) {
            return Err(violation(
                function_location(function),
                "control-flow facts do not prove this exact instruction shape",
            ));
        }
        prove_all_instructions_reachable(
            function.index(),
            function.instructions().len(),
            |instruction| flow.proves_reachable_instruction(function.index(), instruction),
        )?;
        canonical::prove_canonical(exact.entries(), function, limits)?;
        let schedule = build_function_schedule(function, exact.entries())?;
        total_event_count = total_event_count
            .checked_add(schedule.events.len())
            .ok_or_else(|| resource_overflow(function_location(function)))?;
        functions.push(schedule);
    }

    Ok(VerifiedStatementSchedule {
        functions: functions.into_boxed_slice(),
        total_event_count,
    })
}

fn preflight(
    candidate: &LinkedBytecodeCandidate,
    binding: &ExactStatementBinding,
    source: &SourceAttributionFacts,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    if candidate.functions().len() != binding.functions().len() {
        return Err(violation(
            VerificationLocation::Image,
            "P1 statement binding function coverage differs from the candidate",
        ));
    }
    let mut total = 0_u64;
    for function in candidate.functions() {
        let location = function_location(function);
        let exact = binding
            .function(function.index())
            .ok_or_else(|| violation(location, "P1 statement binding is missing a function"))?;
        let events = count(exact.entries().len(), location)?;
        check_limit(
            VerificationLimit::StatementEventsPerFunction,
            events,
            limits.max_statement_events_per_function,
            location,
        )?;
        let source_maps = count(function.source_map().len(), location)?;
        check_limit(
            VerificationLimit::SourceMapEntriesPerFunction,
            source_maps,
            limits.max_source_map_entries_per_function,
            location,
        )?;
        if !source.proves_function(
            function.index(),
            function.instructions().len(),
            function.source_map().len(),
        ) {
            return Err(violation(
                location,
                "source proof facts do not cover this exact function shape",
            ));
        }
        total = total
            .checked_add(events)
            .ok_or_else(|| resource_overflow(location))?;
        check_limit(
            VerificationLimit::TotalStatementEvents,
            total,
            limits.max_total_statement_events,
            location,
        )?;
    }
    check_limit(
        VerificationLimit::TotalStatementEvents,
        total,
        limits.max_total_statement_events,
        VerificationLocation::Image,
    )
}

fn build_function_schedule(
    function: &LinkedFunction,
    entries: &[crate::admission::ExactStatementEntry],
) -> Result<VerifiedFunctionStatementSchedule, VerificationError> {
    let offset_capacity = function
        .instructions()
        .len()
        .checked_add(1)
        .ok_or_else(|| resource_overflow(function_location(function)))?;
    let mut instruction_offsets = Vec::with_capacity(offset_capacity);
    let mut events = Vec::with_capacity(entries.len());
    let mut cursor = 0_usize;

    for (ordinal, instruction) in function.instructions().iter().enumerate() {
        instruction_offsets.push(events.len());
        let index = instruction_index(function, ordinal)?;
        let start = cursor;
        while entries
            .get(cursor)
            .is_some_and(|entry| entry.instruction() == index)
        {
            cursor = cursor
                .checked_add(1)
                .ok_or_else(|| resource_overflow(function_location(function)))?;
        }
        append_instruction_events(
            function.index(),
            index,
            instruction.opcode(),
            &entries[start..cursor],
            &mut events,
        )?;
    }
    if cursor != entries.len() {
        return Err(violation(
            function_location(function),
            "authenticated statement rows are not covered by dense instructions",
        ));
    }
    instruction_offsets.push(events.len());
    Ok(VerifiedFunctionStatementSchedule {
        frame_entry_charge_kind: FRAME_ENTRY_STATEMENT_CONTRACT.charge_kind,
        instruction_offsets: instruction_offsets.into_boxed_slice(),
        events: events.into_boxed_slice(),
    })
}

fn append_instruction_events(
    function: FunctionIndex,
    instruction: InstructionIndex,
    opcode: skiff_artifact_model::Opcode,
    entries: &[crate::admission::ExactStatementEntry],
    events: &mut Vec<VerifiedStatementEvent>,
) -> Result<(), VerificationError> {
    let contract = contract_for_opcode(opcode);
    let matching = match contract.statement {
        StatementContract::None => 0,
        StatementContract::RequiredEvent { attribution, .. } => entries
            .iter()
            .filter(|entry| entry.attribution_id().class() == attribution)
            .count(),
    };
    let required = required_reclassification(
        contract.statement,
        matching,
        contract.mnemonic,
        instruction_location_at(function, instruction),
    )?;
    for entry in entries {
        let class = entry.attribution_id().class();
        let charge_kind = required
            .filter(|(required_class, _)| *required_class == class)
            .map_or_else(
                || default_statement_charge_kind_for_attribution(class),
                |(_, charge_kind)| charge_kind,
            );
        events.push(VerifiedStatementEvent {
            sequence_ordinal: entry.sequence_ordinal(),
            attribution_id: entry.attribution_id(),
            site: entry.site().clone(),
            charge_kind,
        });
    }
    Ok(())
}

fn required_reclassification(
    contract: StatementContract,
    matching: usize,
    mnemonic: &str,
    location: VerificationLocation,
) -> Result<
    Option<(
        skiff_artifact_model::StatementAttributionClass,
        StatementChargeKind,
    )>,
    VerificationError,
> {
    Ok(match contract {
        StatementContract::None => None,
        StatementContract::RequiredEvent {
            charge_kind,
            attribution,
        } => {
            if matching != 1 {
                return Err(violation(
                    location,
                    format!(
                        "{} requires exactly one {} event at this instruction; found {matching}",
                        mnemonic,
                        attribution.name(),
                    ),
                ));
            }
            Some((attribution, charge_kind))
        }
    })
}

fn prove_all_instructions_reachable(
    function: FunctionIndex,
    instruction_count: usize,
    mut reachable: impl FnMut(InstructionIndex) -> bool,
) -> Result<(), VerificationError> {
    for ordinal in 0..instruction_count {
        let instruction = u32::try_from(ordinal)
            .map(InstructionIndex::new)
            .map_err(|_| resource_overflow(function_location_at(function)))?;
        if !reachable(instruction) {
            return Err(violation(
                instruction_location_at(function, instruction),
                "statement schedule cannot cover an instruction without reachable flow facts",
            ));
        }
    }
    Ok(())
}

fn prove_dense_function(
    function: &LinkedFunction,
    ordinal: usize,
) -> Result<(), VerificationError> {
    let expected = u32::try_from(ordinal)
        .map(FunctionIndex::new)
        .map_err(|_| resource_overflow(VerificationLocation::Image))?;
    if function.index() != expected {
        return Err(violation(
            function_location(function),
            "candidate functions are not dense for statement scheduling",
        ));
    }
    Ok(())
}

fn instruction_index(
    function: &LinkedFunction,
    ordinal: usize,
) -> Result<InstructionIndex, VerificationError> {
    u32::try_from(ordinal)
        .map(InstructionIndex::new)
        .map_err(|_| resource_overflow(function_location(function)))
}

fn count(len: usize, location: VerificationLocation) -> Result<u64, VerificationError> {
    u64::try_from(len).map_err(|_| resource_overflow(location))
}

fn check_limit(
    limit: VerificationLimit,
    actual: u64,
    max: u64,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if actual > max {
        return Err(VerificationError::LimitExceeded {
            limit,
            actual,
            max,
            location,
        });
    }
    Ok(())
}

const fn function_location(function: &LinkedFunction) -> VerificationLocation {
    function_location_at(function.index())
}

const fn function_location_at(function: FunctionIndex) -> VerificationLocation {
    VerificationLocation::Function { function }
}

const fn instruction_location_at(
    function: FunctionIndex,
    instruction: InstructionIndex,
) -> VerificationLocation {
    VerificationLocation::Instruction {
        function,
        instruction,
    }
}

fn resource_overflow(location: VerificationLocation) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ResourceAccounting,
        location,
        detail: "statement schedule arithmetic overflowed".to_string(),
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::SourceAndStatementAttribution,
        location,
        detail: detail.into(),
    }
}
