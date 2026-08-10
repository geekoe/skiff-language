use skiff_runtime_linked_bytecode::{LinkedBytecodeCandidate, LinkedFunction};

use crate::{
    VerificationError, VerificationLimit, VerificationLimits, VerificationLocation,
    VerificationObligation,
};

pub(super) fn check_attribution_budgets(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let mut total_events = 0_u64;
    for function in candidate.functions() {
        let location = VerificationLocation::Function {
            function: function.index(),
        };
        check_limit(
            VerificationLimit::StatementEventsPerFunction,
            count(function.statement_entries().len(), location)?,
            limits.max_statement_events_per_function,
            location,
        )?;
        check_limit(
            VerificationLimit::SourceMapEntriesPerFunction,
            count(function.source_map().len(), location)?,
            limits.max_source_map_entries_per_function,
            location,
        )?;
        check_events_per_pc(function, limits)?;
        total_events = checked_add(
            total_events,
            count(function.statement_entries().len(), location)?,
            VerificationLimit::TotalStatementEvents,
            limits.max_total_statement_events,
            location,
        )?;
    }
    check_limit(
        VerificationLimit::TotalStatementEvents,
        total_events,
        limits.max_total_statement_events,
        VerificationLocation::Image,
    )
}

fn check_events_per_pc(
    function: &LinkedFunction,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let mut previous = None;
    let mut at_pc = 0_u64;
    for entry in function.statement_entries() {
        let instruction = entry.instruction();
        at_pc = if previous == Some(instruction) {
            at_pc.checked_add(1).ok_or_else(|| {
                overflow(VerificationLocation::Instruction {
                    function: function.index(),
                    instruction,
                })
            })?
        } else {
            1
        };
        previous = Some(instruction);
        check_limit(
            VerificationLimit::StatementEventsPerPc,
            at_pc,
            limits.max_statement_events_per_pc,
            VerificationLocation::Instruction {
                function: function.index(),
                instruction,
            },
        )?;
    }
    Ok(())
}

fn checked_add(
    current: u64,
    additional: u64,
    limit: VerificationLimit,
    max: u64,
    location: VerificationLocation,
) -> Result<u64, VerificationError> {
    let actual = current
        .checked_add(additional)
        .ok_or_else(|| overflow(location))?;
    check_limit(limit, actual, max, location)?;
    Ok(actual)
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

fn count(len: usize, location: VerificationLocation) -> Result<u64, VerificationError> {
    u64::try_from(len).map_err(|_| overflow(location))
}

fn overflow(location: VerificationLocation) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ResourceAccounting,
        location,
        detail: "statement attribution budget arithmetic overflowed".to_string(),
    }
}
