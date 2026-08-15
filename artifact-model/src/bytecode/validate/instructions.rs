use crate::bytecode::decode::DecodedInstruction;
use crate::bytecode::dto::{
    limits, BytecodeArtifact, BytecodePoolEntry, BytecodePools, RelocatableBytecodeFunction,
};
use crate::bytecode::opcodes::{
    pool_operand_category, table_operand_category, Opcode, OperandKind, OperandRole, TableCategory,
    TrapFailureKind,
};

use super::{descriptor_mismatch, limit_error, StructuralValidationError};

/// C5: operand indices in bounds, pool/table category fixed by position,
/// count-class immediates bounded, relocation kind compatible with the
/// opcode's allowed set.
pub(super) fn validate_operands(
    key: &str,
    function: &RelocatableBytecodeFunction,
    instructions: &[DecodedInstruction],
    artifact: &BytecodeArtifact,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let slot_count = function.frame_layout.slot_count;
    for instruction in instructions {
        let descriptor = instruction.descriptor;
        for (position, kind) in descriptor.operand_layout.iter().enumerate() {
            let word = instruction.operand_words[position];
            let role = descriptor.operand_roles[position];
            let location = || format!("functions[{key}] pc {} operand[{position}]", instruction.pc);
            match kind {
                OperandKind::Immediate => {
                    if role == OperandRole::FailureKind {
                        if TrapFailureKind::from_encoded(word).is_none() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!("{} unknown trap failure kind {word}", location()),
                            });
                        }
                        continue;
                    }
                    let (limit, max) = match role {
                        OperandRole::ResultCount => {
                            ("MAX_RESULTS_PER_CALL", limits::MAX_RESULTS_PER_CALL)
                        }
                        _ => ("MAX_ARITY", limits::MAX_ARITY),
                    };
                    if word as u64 > max {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} {role:?} immediate {word} exceeds {limit} {max}",
                                location(),
                            ),
                        });
                    }
                }
                OperandKind::Slot => {
                    if word >= slot_count {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} slot index {word} out of bounds: slotCount {slot_count}",
                                location()
                            ),
                        });
                    }
                }
                OperandKind::Pool => {
                    let Some(category) = pool_operand_category(descriptor.opcode, position) else {
                        return Err(descriptor_mismatch(key, instruction.pc, location()));
                    };
                    if word as u64 >= pools.len(category) {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} pool index {word} out of bounds: {} pool has {} entries",
                                location(),
                                category.name(),
                                pools.len(category)
                            ),
                        });
                    }
                    let Some(entry) = pools.entry(category, word) else {
                        return Err(descriptor_mismatch(key, instruction.pc, location()));
                    };
                    if entry.category() != category {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} pool entry kind mismatch: expected {} entry, got {}",
                                location(),
                                category.name(),
                                entry.category().name()
                            ),
                        });
                    }
                }
                OperandKind::Table => {
                    let Some(category) = table_operand_category(descriptor.opcode, position) else {
                        return Err(descriptor_mismatch(key, instruction.pc, location()));
                    };
                    let table_len = match category {
                        TableCategory::ExceptionRegions => function.exception_regions.len(),
                        TableCategory::SwitchTables => function.switch_tables.len(),
                        TableCategory::ActiveRegions => function.active_regions.len(),
                        TableCategory::CallLoanLayouts => function.call_loan_layouts.len(),
                    };
                    if word as usize >= table_len {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} table index {word} out of bounds: {} table has {table_len} entries",
                                location(),
                                category.name()
                            ),
                        });
                    }
                }
                OperandKind::Reloc => {
                    if word as usize >= function.relocations.len() {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} relocation index {word} out of bounds: {} relocations",
                                location(),
                                function.relocations.len()
                            ),
                        });
                    }
                    let relocation = &function.relocations[word as usize];
                    let declared_kind = relocation.kind();
                    if !descriptor.allowed_relocations.contains(&declared_kind) {
                        return Err(StructuralValidationError::Operand {
                            function_key: key.to_string(),
                            pc: instruction.pc,
                            message: format!(
                                "{} relocation kind {} not allowed for {}",
                                location(),
                                declared_kind.name(),
                                descriptor.mnemonic
                            ),
                        });
                    }
                    if let crate::bytecode::dto::BytecodeRelocation::LocalExecutableRef {
                        function_key: target_key,
                        specialization,
                    } = relocation
                    {
                        let Some(target) = artifact.image.functions.get(target_key) else {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} local target {target_key:?} is missing",
                                    location()
                                ),
                            });
                        };
                        if specialization.type_arguments.len() != target.type_parameters.len() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} local specialization arity {} does not match target {target_key:?} declaration arity {}",
                                    location(),
                                    specialization.type_arguments.len(),
                                    target.type_parameters.len(),
                                ),
                            });
                        }
                    }
                    if let crate::bytecode::dto::BytecodeRelocation::HostEffectRef(effect) =
                        relocation
                    {
                        let Some(argument_count) = descriptor
                            .operand_word(OperandRole::ArgCount, &instruction.operand_words)
                        else {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} host effect opcode does not declare ArgCount",
                                    location()
                                ),
                            });
                        };
                        if argument_count as usize != effect.signature.parameter_types.len() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} ArgCount {argument_count} does not match host signature parameter count {}",
                                    location(),
                                    effect.signature.parameter_types.len()
                                ),
                            });
                        }
                        let Some(result_count) = descriptor
                            .operand_word(OperandRole::ResultCount, &instruction.operand_words)
                        else {
                            return Err(descriptor_mismatch(key, instruction.pc, location()));
                        };
                        if result_count as usize != effect.signature.result_types.len() {
                            return Err(StructuralValidationError::Operand {
                                function_key: key.to_string(),
                                pc: instruction.pc,
                                message: format!(
                                    "{} ResultCount {result_count} does not match host signature result count {}",
                                    location(),
                                    effect.signature.result_types.len()
                                ),
                            });
                        }
                    }
                }
                OperandKind::Branch => {
                    // Target range/header membership is C6.
                }
            }
        }
        validate_instruction_contract(key, function, instruction, artifact, pools)?;
    }
    Ok(())
}

fn validate_instruction_contract(
    key: &str,
    function: &RelocatableBytecodeFunction,
    instruction: &DecodedInstruction,
    artifact: &BytecodeArtifact,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let descriptor = instruction.descriptor;
    let operand_error = |message: String| StructuralValidationError::Operand {
        function_key: key.to_string(),
        pc: instruction.pc,
        message,
    };
    match descriptor.kind {
        Opcode::CallLocalInOut => {
            let input_count = descriptor
                .operand_word(OperandRole::InputCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "InputCount".to_string())
                })?;
            let layout_index = descriptor
                .operand_word(OperandRole::CallLoanLayout, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CallLoanLayout".to_string())
                })?;
            let Some(layout) = function.call_loan_layouts.get(layout_index as usize) else {
                return Err(descriptor_mismatch(
                    key,
                    instruction.pc,
                    "CallLoanLayout table row".to_string(),
                ));
            };
            let mut selector_count = 0_u64;
            for loan in &layout.loans {
                let Some(BytecodePoolEntry::WritablePath(path)) =
                    pools.writable_paths.get(loan.writable_path_ref as usize)
                else {
                    return Err(descriptor_mismatch(
                        key,
                        instruction.pc,
                        "CallLoanLayout writable path".to_string(),
                    ));
                };
                selector_count = selector_count
                    .checked_add(u64::from(path.selector_count()))
                    .ok_or_else(|| StructuralValidationError::Arithmetic {
                        context: format!(
                            "functions[{key}] pc {} call loan selector count",
                            instruction.pc
                        ),
                    })?;
            }
            if selector_count > limits::MAX_ARITY {
                return Err(limit_error(
                    "MAX_ARITY",
                    limits::MAX_ARITY,
                    selector_count,
                    &format!(
                        "functions[{key}] pc {} call loan selector count",
                        instruction.pc
                    ),
                ));
            }
            if u64::from(input_count) < selector_count {
                return Err(operand_error(format!(
                    "InputCount {input_count} is smaller than call loan selector count {selector_count}"
                )));
            }
        }
        Opcode::NewRecord => {
            let shape_index = descriptor
                .operand_word(OperandRole::ShapeRef, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "ShapeRef".to_string()))?;
            let field_count = descriptor
                .operand_word(OperandRole::FieldCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "FieldCount".to_string())
                })?;
            let Some(BytecodePoolEntry::ShapeRef { shape }) =
                pools.shapes.get(shape_index as usize)
            else {
                return Err(operand_error(
                    "ShapeRef does not select a shape".to_string(),
                ));
            };
            if field_count as usize != shape.fields.len() {
                return Err(operand_error(format!(
                    "FieldCount {field_count} does not match shape field count {}",
                    shape.fields.len()
                )));
            }
            if shape.privileged_affine_composite.is_some() {
                return Err(operand_error(
                    "NewRecord may not construct a registry-owned privileged affine composite"
                        .to_string(),
                ));
            }
        }
        Opcode::GetDenseField | Opcode::TakeDenseField => {
            let shape_index = descriptor
                .operand_word(OperandRole::ShapeRef, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "ShapeRef".to_string()))?;
            let ordinal = descriptor
                .operand_word(OperandRole::FieldOrdinal, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "FieldOrdinal".to_string())
                })?;
            let Some(BytecodePoolEntry::ShapeRef { shape }) =
                pools.shapes.get(shape_index as usize)
            else {
                return Err(operand_error(
                    "ShapeRef does not select a shape".to_string(),
                ));
            };
            if ordinal as usize >= shape.fields.len() {
                return Err(operand_error(format!(
                    "FieldOrdinal {ordinal} is outside shape field count {}",
                    shape.fields.len()
                )));
            }
            let access = super::plans::privileged_field_access(shape, ordinal as usize);
            match (descriptor.kind, access) {
                (Opcode::GetDenseField, Some(crate::PrivilegedAffineFieldAccess::AffineTake)) => {
                    return Err(operand_error(
                        "GetDenseField may not share a privileged affine field".to_string(),
                    ));
                }
                (Opcode::TakeDenseField, Some(crate::PrivilegedAffineFieldAccess::AffineTake)) => {}
                (Opcode::TakeDenseField, _) => {
                    return Err(operand_error(
                        "TakeDenseField requires the exact affine-take field of a privileged composite"
                            .to_string(),
                    ));
                }
                (Opcode::GetDenseField, _) => {}
                _ => unreachable!("match is limited to dense-field opcodes"),
            }
        }
        Opcode::SetWritablePath => {
            let root_slot = descriptor
                .operand_word(OperandRole::Slot, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "Slot".to_string()))?;
            let path_index = descriptor
                .operand_word(OperandRole::WritablePathRef, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "WritablePathRef".to_string())
                })?;
            let selector_count = descriptor
                .operand_word(OperandRole::SelectorCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "SelectorCount".to_string())
                })?;
            let Some(BytecodePoolEntry::WritablePath(path)) =
                pools.writable_paths.get(path_index as usize)
            else {
                return Err(operand_error(
                    "WritablePathRef does not select a writable path".to_string(),
                ));
            };
            if function.frame_layout.slot_type_refs[root_slot as usize] != path.root_type_ref {
                return Err(operand_error(format!(
                    "root slot type {} does not match writable path rootTypeRef {}",
                    function.frame_layout.slot_type_refs[root_slot as usize], path.root_type_ref
                )));
            }
            if selector_count != path.selector_count() {
                return Err(operand_error(format!(
                    "SelectorCount {selector_count} does not match writable path selector count {}",
                    path.selector_count()
                )));
            }
        }
        Opcode::MakeCallback => {
            let relocation_index = descriptor
                .operand_word(OperandRole::CallbackTarget, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CallbackTarget".to_string())
                })?;
            let layout_index = descriptor
                .operand_word(OperandRole::CaptureLayoutRef, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CaptureLayoutRef".to_string())
                })?;
            let capture_count = descriptor
                .operand_word(OperandRole::CaptureCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "CaptureCount".to_string())
                })?;
            let Some(crate::bytecode::dto::BytecodeRelocation::SyntheticCallbackRef {
                function_key: target_key,
            }) = function.relocations.get(relocation_index as usize)
            else {
                return Err(operand_error(
                    "CallbackTarget does not select a synthetic callback".to_string(),
                ));
            };
            let Some(BytecodePoolEntry::CallbackCaptureLayout(layout)) =
                pools.callback_capture.get(layout_index as usize)
            else {
                return Err(operand_error(
                    "CaptureLayoutRef does not select a capture layout".to_string(),
                ));
            };
            if layout.function_key != *target_key || layout.captures.len() != capture_count as usize
            {
                return Err(operand_error(format!(
                    "callback layout target/count ({:?}, {}) does not match ({target_key:?}, {capture_count})",
                    layout.function_key,
                    layout.captures.len()
                )));
            }
        }
        Opcode::InterfaceBoxLocal => {
            let relocation_index = descriptor
                .operand_word(OperandRole::InterfaceTarget, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "InterfaceTarget".to_string())
                })?;
            let Some(crate::bytecode::dto::BytecodeRelocation::LocalInterfaceRef { interface }) =
                function.relocations.get(relocation_index as usize)
            else {
                return Err(operand_error(
                    "InterfaceTarget does not select a local interface table".to_string(),
                ));
            };
            for method in &interface.methods {
                if !artifact.image.functions.contains_key(&method.function_key) {
                    return Err(operand_error(format!(
                        "local interface method references missing function {:?}",
                        method.function_key
                    )));
                }
            }
        }
        Opcode::InvokeIntrinsic => {
            let relocation_index = descriptor
                .operand_word(OperandRole::IntrinsicTarget, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "IntrinsicTarget".to_string())
                })?;
            let argument_count = descriptor
                .operand_word(OperandRole::ArgCount, &instruction.operand_words)
                .ok_or_else(|| descriptor_mismatch(key, instruction.pc, "ArgCount".to_string()))?;
            let result_count = descriptor
                .operand_word(OperandRole::ResultCount, &instruction.operand_words)
                .ok_or_else(|| {
                    descriptor_mismatch(key, instruction.pc, "ResultCount".to_string())
                })?;
            match function.relocations.get(relocation_index as usize) {
                Some(crate::bytecode::dto::BytecodeRelocation::IntrinsicRef { intrinsic }) => {
                    if argument_count as usize != intrinsic.signature.parameter_types.len()
                        || result_count as usize != intrinsic.signature.result_types.len()
                    {
                        return Err(operand_error(format!(
                            "intrinsic arg/result counts ({argument_count}, {result_count}) do not match signature ({}, {})",
                            intrinsic.signature.parameter_types.len(),
                            intrinsic.signature.result_types.len()
                        )));
                    }
                }
                Some(crate::bytecode::dto::BytecodeRelocation::TaskSubmitRef { .. }) => {}
                _ => {
                    return Err(operand_error(
                        "IntrinsicTarget does not select an intrinsic reference".to_string(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}
