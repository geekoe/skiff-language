mod active_regions;
mod source_map;
mod statements;
mod switch_tables;

use crate::bytecode::decode::{decode_branch_target, DecodedFunction, DecodedInstruction};
use crate::bytecode::dto::{
    limits, BytecodeArtifact, BytecodePoolEntry, BytecodePools, RelocatableBytecodeFunction,
};
use crate::bytecode::opcodes::{Arity, Opcode, OperandKind, OperandRole, PoolCategory};

use super::{
    entry_is_kind, header_error, index_out_of_bounds, limit_error, table_error,
    validate_type_pool_ref, StructuralValidationError, ValidatedFunction, ValidatedResumeSite,
};

pub(super) fn validate_resume_sites(
    artifact: &BytecodeArtifact,
    functions: &[ValidatedFunction],
) -> Result<Vec<ValidatedResumeSite>, StructuralValidationError> {
    let mut reference_counts = vec![0u32; artifact.image.pools.resume.len()];
    let mut validated = Vec::new();
    for function in functions {
        for instruction in &function.instructions {
            let descriptor = instruction.descriptor;
            let Some(descriptor_index) =
                descriptor.operand_word(OperandRole::ResumeRef, &instruction.operand_words)
            else {
                continue;
            };
            let Some(BytecodePoolEntry::ResumeDescriptor(resume)) =
                artifact.image.pools.resume.get(descriptor_index as usize)
            else {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "ResumeRef {descriptor_index} does not select a resume descriptor"
                    ),
                });
            };
            reference_counts[descriptor_index as usize] =
                reference_counts[descriptor_index as usize].saturating_add(1);
            let resume_pc = instruction
                .pc
                .checked_add(descriptor.instruction_word_count())
                .ok_or_else(|| StructuralValidationError::Arithmetic {
                    context: format!(
                        "functions[{}] pc {} resume pc",
                        function.function_key, instruction.pc
                    ),
                })?;
            if resume.function_key != function.function_key
                || resume.site_pc != instruction.pc
                || resume.resume_pc != resume_pc
                || function.header_pcs.binary_search(&resume_pc).is_err()
            {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "resume[{descriptor_index}] must bind this function/site and the immediately following instruction pc {resume_pc}"
                    ),
                });
            }
            let end_resume_pc = match (descriptor.kind, resume.end_resume_pc) {
                (Opcode::StreamNext, Some(end_resume_pc)) => {
                    if end_resume_pc == resume_pc {
                        return Err(StructuralValidationError::Target {
                            function_key: function.function_key.clone(),
                            pc: instruction.pc,
                            message: format!(
                                "StreamNext resume[{descriptor_index}] endResumePc {end_resume_pc} must differ from item resumePc {resume_pc}"
                            ),
                        });
                    }
                    if function.header_pcs.binary_search(&end_resume_pc).is_err() {
                        return Err(StructuralValidationError::Target {
                            function_key: function.function_key.clone(),
                            pc: instruction.pc,
                            message: format!(
                                "StreamNext resume[{descriptor_index}] endResumePc {end_resume_pc} is not an instruction header"
                            ),
                        });
                    }
                    Some(end_resume_pc)
                }
                (Opcode::StreamNext, None) => {
                    return Err(StructuralValidationError::Target {
                        function_key: function.function_key.clone(),
                        pc: instruction.pc,
                        message: format!(
                            "StreamNext resume[{descriptor_index}] requires endResumePc for natural stream end"
                        ),
                    });
                }
                (_, Some(_)) => {
                    return Err(StructuralValidationError::Target {
                        function_key: function.function_key.clone(),
                        pc: instruction.pc,
                        message: format!(
                            "resume[{descriptor_index}] endResumePc is only valid for StreamNext"
                        ),
                    });
                }
                (_, None) => None,
            };
            let result_arity = resolve_stack_effect_arity(
                descriptor.stack_out,
                instruction,
                function.frame_layout.result_count,
            )?;
            if result_arity as usize != resume.result_type_refs.len() {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "resume[{descriptor_index}] result arity {} does not match opcode result arity {result_arity}",
                        resume.result_type_refs.len()
                    ),
                });
            }
            let resumed_height = resume
                .expected_stack_height_before_result
                .checked_add(result_arity)
                .ok_or_else(|| StructuralValidationError::Arithmetic {
                    context: format!("resume[{descriptor_index}] stack height"),
                })?;
            if resumed_height > function.max_operand_depth {
                return Err(StructuralValidationError::Target {
                    function_key: function.function_key.clone(),
                    pc: instruction.pc,
                    message: format!(
                        "resume[{descriptor_index}] stack height {resumed_height} exceeds maxOperandDepth {}",
                        function.max_operand_depth
                    ),
                });
            }
            let stream_item = if descriptor.kind == Opcode::EmitStream {
                Some(function.function_stream_item.clone().ok_or_else(|| {
                    StructuralValidationError::Target {
                        function_key: function.function_key.clone(),
                        pc: instruction.pc,
                        message: "EmitStream requires an exact function stream item authority"
                            .to_string(),
                    }
                })?)
            } else {
                None
            };
            match (descriptor.kind, resume.emit_stream_item_shape_ref) {
                (Opcode::EmitStream, Some(_)) => {}
                (Opcode::EmitStream, None) => {
                    return Err(StructuralValidationError::Target {
                        function_key: function.function_key.clone(),
                        pc: instruction.pc,
                        message: format!(
                            "EmitStream resume[{descriptor_index}] requires emitStreamItemShapeRef"
                        ),
                    });
                }
                (_, Some(_)) => {
                    return Err(StructuralValidationError::Target {
                        function_key: function.function_key.clone(),
                        pc: instruction.pc,
                        message: format!(
                            "resume[{descriptor_index}] emitStreamItemShapeRef is only valid for EmitStream"
                        ),
                    });
                }
                (_, None) => {}
            }
            validated.push(ValidatedResumeSite {
                function_key: function.function_key.clone(),
                descriptor_index,
                site_pc: instruction.pc,
                resume_pc,
                end_resume_pc,
                expected_stack_height_before_result: resume.expected_stack_height_before_result,
                result_type_refs: resume.result_type_refs.clone(),
                result_plans: resume.result_plans.clone(),
                result_materializations: resume.result_materializations.clone(),
                emit_stream_item_shape_ref: resume.emit_stream_item_shape_ref,
                error_mode: resume.error_mode,
                stream_item,
            });
        }
    }
    for (index, count) in reference_counts.into_iter().enumerate() {
        if count != 1 {
            return Err(header_error(format!(
                "image.pools.resume[{index}] must be referenced by exactly one pending site (found {count})"
            )));
        }
    }
    Ok(validated)
}

fn resolve_stack_effect_arity(
    effects: &[crate::bytecode::opcodes::StackEffect],
    instruction: &DecodedInstruction,
    function_result_count: u32,
) -> Result<u32, StructuralValidationError> {
    let mut total = 0u32;
    for effect in effects {
        let arity = match effect.arity {
            Arity::Fixed(value) => u32::from(value),
            Arity::Declared(role) => instruction
                .descriptor
                .operand_word(role, &instruction.operand_words)
                .ok_or_else(|| {
                    header_error(format!(
                        "opcode {} stack effect references absent operand role {role:?}",
                        instruction.descriptor.mnemonic
                    ))
                })?,
            Arity::FunctionResultCount => function_result_count,
        };
        total = total
            .checked_add(arity)
            .ok_or_else(|| StructuralValidationError::Arithmetic {
                context: format!(
                    "opcode {} stack effect arity",
                    instruction.descriptor.mnemonic
                ),
            })?;
    }
    Ok(total)
}

/// C6: branch targets and enter/leave region membership.
pub(super) fn validate_targets(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
) -> Result<(), StructuralValidationError> {
    let header_pcs = &decoded.header_pcs;
    for instruction in &decoded.instructions {
        let descriptor = instruction.descriptor;
        for (position, kind) in descriptor.operand_layout.iter().enumerate() {
            if *kind != OperandKind::Branch {
                continue;
            }
            let word = instruction.operand_words[position];
            let target =
                decode_branch_target(instruction.pc, descriptor.operand_word_count(), word)
                    .ok_or_else(|| StructuralValidationError::Arithmetic {
                        context: format!(
                            "functions[{key}] pc {} branch target decode",
                            instruction.pc
                        ),
                    })?;
            if header_pcs.binary_search(&target).is_err() {
                return Err(StructuralValidationError::Target {
                    function_key: key.to_string(),
                    pc: instruction.pc,
                    message: format!(
                        "branch target {target} does not point at an instruction header"
                    ),
                });
            }
        }
        if let Some(region_index) =
            descriptor.operand_word(OperandRole::ActiveRegion, &instruction.operand_words)
        {
            let region_index = region_index as usize;
            let region = &function.active_regions[region_index];
            let next_pc = instruction
                .pc
                .checked_add(descriptor.instruction_word_count())
                .ok_or_else(|| StructuralValidationError::Arithmetic {
                    context: format!("functions[{key}] active region instruction end"),
                })?;
            let valid = match descriptor.kind {
                Opcode::EnterRegion => instruction.pc == region.start_pc,
                Opcode::LeaveRegion => next_pc == region.end_pc,
                _ => false,
            };
            if !valid {
                return Err(StructuralValidationError::Target {
                    function_key: key.to_string(),
                    pc: instruction.pc,
                    message: format!(
                        "{} does not match active region [{}, {}) boundary",
                        descriptor.mnemonic, region.start_pc, region.end_pc
                    ),
                });
            }
        }
    }
    Ok(())
}

/// C7: exception regions, statement entries, source map and switch tables
/// structure (ordering, well-nested regions, header membership, tag kind).
pub(super) fn validate_tables(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let header_pcs = &decoded.header_pcs;
    validate_exception_regions(key, function, header_pcs, pools)?;
    active_regions::validate_active_regions(key, function, decoded)?;
    statements::validate_statement_entries(key, function, &decoded.instructions)?;
    source_map::validate_source_map(key, function, decoded)?;
    switch_tables::validate_switch_tables(key, function, header_pcs, pools)?;
    Ok(())
}

fn validate_exception_regions(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let slot_count = function.frame_layout.slot_count;
    let mut previous_region: Option<(u32, u32)> = None;
    let mut open_regions = Vec::<(usize, u32)>::new();
    for (index, region) in function.exception_regions.iter().enumerate() {
        let location = |field: &str| format!("functions[{key}].exceptionRegions[{index}].{field}");
        if region.start_pc >= region.end_pc {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] startPc {} >= endPc {}",
                    region.start_pc, region.end_pc
                ),
            ));
        }
        for (field, pc) in [
            ("startPc", region.start_pc),
            ("handlerPc", region.handler_pc),
        ] {
            if header_pcs.binary_search(&pc).is_err() {
                return Err(table_error(
                    key,
                    format!("exceptionRegions[{index}].{field} {pc} is not an instruction header"),
                ));
            }
        }
        if region.end_pc != function.words.len() as u32
            && header_pcs.binary_search(&region.end_pc).is_err()
        {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}].endPc {} is not an instruction boundary",
                    region.end_pc
                ),
            ));
        }
        if region.start_pc <= region.handler_pc && region.handler_pc < region.end_pc {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}].handlerPc {} must be outside its protected range [{}, {})",
                    region.handler_pc, region.start_pc, region.end_pc
                ),
            ));
        }
        if let Some((previous_start, previous_end)) = previous_region {
            if region.start_pc < previous_start
                || (region.start_pc == previous_start && region.end_pc >= previous_end)
            {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}] [{}, {}) is not in canonical start-ascending, outer-first order after [{previous_start}, {previous_end})",
                        region.start_pc, region.end_pc
                    ),
                ));
            }
        }
        while open_regions
            .last()
            .is_some_and(|(_, parent_end)| *parent_end <= region.start_pc)
        {
            open_regions.pop();
        }
        if let Some((parent_index, parent_end)) = open_regions.last() {
            if region.end_pc > *parent_end {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}] [{}, {}) crosses exceptionRegions[{parent_index}] ending at {parent_end}",
                        region.start_pc, region.end_pc
                    ),
                ));
            }
        }
        if region.catch_slot >= slot_count {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] catchSlot {} out of bounds: slotCount {slot_count}",
                    region.catch_slot
                ),
            ));
        }
        validate_type_pool_ref(
            pools,
            region.catch_slot_type_ref,
            &location("catchSlotTypeRef"),
        )?;
        if function
            .frame_layout
            .slot_type_refs
            .get(region.catch_slot as usize)
            .copied()
            != Some(region.catch_slot_type_ref)
        {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] catchSlotTypeRef {} does not match catch slot frame type",
                    region.catch_slot_type_ref
                ),
            ));
        }
        if region.handler_stack_height as u64 > limits::MAX_OPERAND_DEPTH {
            return Err(limit_error(
                "MAX_OPERAND_DEPTH",
                limits::MAX_OPERAND_DEPTH,
                region.handler_stack_height as u64,
                &location("handlerStackHeight"),
            ));
        }
        if region.handler_stack_height > function.max_operand_depth {
            return Err(table_error(
                key,
                format!(
                    "exceptionRegions[{index}] handlerStackHeight {} exceeds function maxOperandDepth {}",
                    region.handler_stack_height, function.max_operand_depth
                ),
            ));
        }
        if region.cleanup_depth as u64 > limits::MAX_OPERAND_DEPTH {
            return Err(limit_error(
                "MAX_OPERAND_DEPTH",
                limits::MAX_OPERAND_DEPTH,
                region.cleanup_depth as u64,
                &location("cleanupDepth"),
            ));
        }
        if region.catch_matchers.is_empty() {
            return Err(table_error(
                key,
                format!("exceptionRegions[{index}].catchMatchers must not be empty"),
            ));
        }
        let catch_all_only = matches!(
            region.catch_matchers.as_slice(),
            [crate::bytecode::dto::CatchMatcher::CatchAll]
        );
        let mut previous_matcher_type = None;
        for (matcher_index, matcher) in region.catch_matchers.iter().enumerate() {
            if let crate::bytecode::dto::CatchMatcher::TypeRef { type_ref } = matcher {
                if previous_matcher_type.is_some_and(|previous| previous >= *type_ref) {
                    return Err(table_error(
                        key,
                        format!(
                            "exceptionRegions[{index}].catchMatchers type refs must be strictly ascending"
                        ),
                    ));
                }
                if *type_ref as usize >= pools.types.len() {
                    return Err(index_out_of_bounds(
                        "types pool",
                        *type_ref,
                        &location(&format!("catchMatchers[{matcher_index}].typeRef")),
                    ));
                }
                if !entry_is_kind(&pools.types[*type_ref as usize], PoolCategory::Types) {
                    return Err(table_error(
                        key,
                        format!(
                            "exceptionRegions[{index}].catchMatchers[{matcher_index}] must reference a TypeRef entry"
                        ),
                    ));
                }
                previous_matcher_type = Some(*type_ref);
            } else if !catch_all_only {
                return Err(table_error(
                    key,
                    format!(
                        "exceptionRegions[{index}].catchMatchers must be either ascending TypeRef entries or a single CatchAll"
                    ),
                ));
            }
        }
        previous_region = Some((region.start_pc, region.end_pc));
        open_regions.push((index, region.end_pc));
    }
    Ok(())
}
