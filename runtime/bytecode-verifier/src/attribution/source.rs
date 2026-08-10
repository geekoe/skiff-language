use skiff_artifact_model::{
    contract_for_opcode, ExceptionBehavior, InstructionSourceSite, OpcodeContract, OperandRole,
    SourceContract, SourceOriginConstraint,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedActiveRegionKind, LinkedBytecodeCandidate,
    LinkedFunction, LinkedInstruction, LinkedInstructionTarget, LinkedSourceMapEntry,
};

use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Private token proving that source attribution was independently checked
/// for the same dense function/instruction shape consumed by statement P2.
#[derive(Debug)]
pub(crate) struct SourceAttributionFacts {
    functions: Box<[SourceFunctionFacts]>,
}

#[derive(Debug)]
struct SourceFunctionFacts {
    function: FunctionIndex,
    instruction_count: usize,
    source_map_entries: usize,
}

impl SourceAttributionFacts {
    pub(super) fn proves_function(
        &self,
        function: FunctionIndex,
        instruction_count: usize,
        source_map_entries: usize,
    ) -> bool {
        self.functions
            .get(function.get() as usize)
            .is_some_and(|facts| {
                facts.function == function
                    && facts.instruction_count == instruction_count
                    && facts.source_map_entries == source_map_entries
            })
    }
}

/// Independently proves the canonical source route for every linked PC.
///
/// This proof deliberately ignores linker stack-map claims and does not treat
/// structural validation as semantic source authority. Function-local source
/// ranges are checked again before the canonical opcode contract is applied.
pub(crate) fn prove_source_attribution(
    candidate: &LinkedBytecodeCandidate,
) -> Result<SourceAttributionFacts, VerificationError> {
    let mut functions = Vec::with_capacity(candidate.functions().len());
    for function in candidate.functions() {
        prove_function(function)?;
        functions.push(SourceFunctionFacts {
            function: function.index(),
            instruction_count: function.instructions().len(),
            source_map_entries: function.source_map().len(),
        });
    }
    Ok(SourceAttributionFacts {
        functions: functions.into_boxed_slice(),
    })
}

fn prove_function(function: &LinkedFunction) -> Result<(), VerificationError> {
    let instruction_count = u32::try_from(function.instructions().len()).map_err(|_| {
        function_violation(
            function,
            "linked instruction count does not fit the source-map index domain",
        )
    })?;
    prove_source_ranges(function, instruction_count)?;

    let ranges = function.source_map();
    let mut range_ordinal = 0_usize;
    for (ordinal, instruction) in function.instructions().iter().enumerate() {
        let index = instruction_index(function, ordinal)?;
        while ranges
            .get(range_ordinal)
            .is_some_and(|range| range.end().get() <= index.get())
        {
            range_ordinal = range_ordinal.checked_add(1).ok_or_else(|| {
                function_violation(function, "source-map cursor overflowed usize")
            })?;
        }
        let covering = ranges
            .get(range_ordinal)
            .filter(|range| range.start().get() <= index.get() && index.get() < range.end().get());
        prove_instruction(function, index, instruction, covering)?;
    }
    Ok(())
}

fn prove_source_ranges(
    function: &LinkedFunction,
    instruction_count: u32,
) -> Result<(), VerificationError> {
    let mut previous_end = None;
    for (row, range) in function.source_map().iter().enumerate() {
        let start = range.start().get();
        let end = range.end().get();
        if start >= end {
            return Err(function_violation(
                function,
                format!("linked source-map row {row} has an empty or reversed range"),
            ));
        }
        if end > instruction_count {
            return Err(function_violation(
                function,
                format!("linked source-map row {row} ends outside the dense instruction slice"),
            ));
        }
        if previous_end.is_some_and(|previous_end| previous_end > start) {
            return Err(function_violation(
                function,
                format!("linked source-map row {row} overlaps or precedes its predecessor"),
            ));
        }
        previous_end = Some(end);
    }
    Ok(())
}

fn prove_instruction(
    function: &LinkedFunction,
    index: InstructionIndex,
    instruction: &LinkedInstruction,
    covering: Option<&LinkedSourceMapEntry>,
) -> Result<(), VerificationError> {
    let contract = contract_for_opcode(instruction.opcode());
    let location = instruction_location(function, index);
    match contract.source {
        SourceContract::None => Ok(()),
        SourceContract::Required { origin, .. } => {
            prove_required_source(contract, origin, covering, location)
        }
        SourceContract::PreserveOriginal => {
            prove_preserved_source(function, instruction, contract, location)
        }
        SourceContract::ActiveRegion { operand } => {
            prove_active_region_source(function, instruction, contract, operand, location)
        }
    }
}

fn prove_required_source(
    contract: &OpcodeContract,
    origin: SourceOriginConstraint,
    covering: Option<&LinkedSourceMapEntry>,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let source = covering.ok_or_else(|| {
        semantic_violation(
            location,
            format!(
                "{} requires exactly one current source or synthetic site",
                contract.mnemonic
            ),
        )
    })?;
    if origin == SourceOriginConstraint::SyntheticOnly
        && !matches!(source.site(), InstructionSourceSite::Synthetic { .. })
    {
        return Err(semantic_violation(
            location,
            format!(
                "{} requires a synthetic current source site",
                contract.mnemonic
            ),
        ));
    }
    Ok(())
}

fn prove_preserved_source(
    function: &LinkedFunction,
    instruction: &LinkedInstruction,
    contract: &OpcodeContract,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let ExceptionBehavior::PreserveOriginal { source_slot } = contract.exception.behavior else {
        return Err(semantic_violation(
            location,
            "canonical preserve-original source contract lacks an exception source slot",
        ));
    };
    let slot = match resolved_target(instruction, contract, source_slot, location)? {
        LinkedInstructionTarget::FrameSlot(slot) => slot,
        _ => {
            return Err(semantic_violation(
                location,
                "preserve-original source operand did not resolve to a frame slot",
            ));
        }
    };
    if slot.get() as usize >= function.frame().slot_types().len() {
        return Err(semantic_violation(
            location,
            "preserve-original source slot is outside the linked frame",
        ));
    }
    Ok(())
}

fn prove_active_region_source(
    function: &LinkedFunction,
    instruction: &LinkedInstruction,
    contract: &OpcodeContract,
    operand: OperandRole,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let region_index = match resolved_target(instruction, contract, operand, location)? {
        LinkedInstructionTarget::ActiveRegion(region) => region,
        _ => {
            return Err(semantic_violation(
                location,
                "active-region source operand did not resolve to an active region",
            ));
        }
    };
    let region = function
        .active_regions()
        .get(region_index.get() as usize)
        .ok_or_else(|| {
            semantic_violation(location, "active-region source target is out of bounds")
        })?;
    if region.index() != region_index {
        return Err(semantic_violation(
            location,
            "active-region source target does not match its dense table position",
        ));
    }
    let _site = match region.kind() {
        LinkedActiveRegionKind::Timeout { site, .. } => site,
    };
    Ok(())
}

fn resolved_target(
    instruction: &LinkedInstruction,
    contract: &OpcodeContract,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<LinkedInstructionTarget, VerificationError> {
    let ordinal = contract.operand_position(role).ok_or_else(|| {
        semantic_violation(
            location,
            "canonical source role is absent from the opcode operands",
        )
    })?;
    let ordinal = u32::try_from(ordinal).map_err(|_| {
        semantic_violation(
            location,
            "canonical source operand ordinal does not fit u32",
        )
    })?;
    instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
        .ok_or_else(|| {
            semantic_violation(
                location,
                "canonical source operand has no typed resolved target",
            )
        })
}

fn instruction_index(
    function: &LinkedFunction,
    ordinal: usize,
) -> Result<InstructionIndex, VerificationError> {
    u32::try_from(ordinal)
        .map(InstructionIndex::new)
        .map_err(|_| function_violation(function, "instruction ordinal does not fit u32"))
}

fn instruction_location(
    function: &LinkedFunction,
    instruction: InstructionIndex,
) -> VerificationLocation {
    VerificationLocation::Instruction {
        function: function.index(),
        instruction,
    }
}

fn function_violation(function: &LinkedFunction, detail: impl Into<String>) -> VerificationError {
    semantic_violation(
        VerificationLocation::Function {
            function: function.index(),
        },
        detail,
    )
}

fn semantic_violation(
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::SourceAndStatementAttribution,
        location,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
