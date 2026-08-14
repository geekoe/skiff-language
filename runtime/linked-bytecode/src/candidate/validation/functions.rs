use skiff_artifact_model::TypeRefIr;

use crate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts, LinkedInstructionTarget, LinkedSlotState,
};

use super::{check_boundary, check_index, plans::validate_plan, position_u32};

pub(super) fn validate_function(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let function_location = CandidateLocation::Function {
        function: function.index(),
    };
    for ty in function.frame().slot_types() {
        check_index(
            function_location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        )?;
    }
    for ty in function.frame().result_types() {
        check_index(
            function_location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        )?;
    }
    for plan in function.frame().slot_plans() {
        validate_plan(plan, function_location, parts)?;
    }
    for plan in function.frame().result_plans() {
        validate_plan(plan, function_location, parts)?;
    }
    for parameter in function.frame().parameters() {
        validate_plan(parameter.plan(), function_location, parts)?;
        if let Some(shape) = parameter.dense_record_shape() {
            check_index(
                function_location,
                CandidateReferenceKind::Shape,
                shape.get(),
                parts.shapes.len(),
            )?;
        }
    }
    validate_stream_producer_authority(function, parts, function_location)?;

    for (position, instruction) in function.instructions().iter().enumerate() {
        let instruction_index =
            u32::try_from(position).map_err(|_| LinkedBytecodeCandidateError::TableTooLarge {
                table: CandidateTable::Functions,
                len: function.instructions().len(),
            })?;
        let location = CandidateLocation::Instruction {
            function: function.index(),
            instruction: crate::InstructionIndex::new(instruction_index),
        };
        for operand in instruction.resolved_operands() {
            validate_instruction_target(operand.target(), location, function, parts)?;
        }
    }
    validate_function_tables(function, parts)?;
    validate_stack_map(function, parts)
}

fn validate_stream_producer_authority(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
    location: CandidateLocation,
) -> Result<(), LinkedBytecodeCandidateError> {
    let Some(stream_type) = function.stream_result_type_ref() else {
        return Ok(());
    };
    check_index(
        location,
        CandidateReferenceKind::Type,
        stream_type.get(),
        parts.types.len(),
    )?;
    let result_count = function.frame().result_types().len();
    if result_count != 0 {
        return Err(
            LinkedBytecodeCandidateError::StreamProducerResultCountNotZero {
                function: function.index(),
                result_count,
            },
        );
    }
    let Some(row) = parts.types.get(stream_type.get() as usize) else {
        return Ok(());
    };
    let TypeRefIr::Builtin { name, args } = row.type_ref() else {
        return Err(LinkedBytecodeCandidateError::StreamProducerTypeMismatch {
            function: function.index(),
            stream_type,
        });
    };
    if name.as_str() != "Stream" || args.len() != 1 {
        return Err(LinkedBytecodeCandidateError::StreamProducerTypeMismatch {
            function: function.index(),
            stream_type,
        });
    }
    Ok(())
}

fn validate_instruction_target(
    target: LinkedInstructionTarget,
    location: CandidateLocation,
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let (kind, index, len) = match target {
        LinkedInstructionTarget::FrameSlot(index) => (
            CandidateReferenceKind::FrameSlot,
            index.get(),
            function.frame().slot_types().len(),
        ),
        LinkedInstructionTarget::Branch(index) => (
            CandidateReferenceKind::Instruction,
            index.get(),
            function.instructions().len(),
        ),
        LinkedInstructionTarget::SwitchTable(index) => (
            CandidateReferenceKind::SwitchTable,
            index.get(),
            function.switch_tables().len(),
        ),
        LinkedInstructionTarget::ActiveRegion(index) => (
            CandidateReferenceKind::ActiveRegion,
            index.get(),
            function.active_regions().len(),
        ),
        LinkedInstructionTarget::CallLoanLayout(index) => (
            CandidateReferenceKind::CallLoanLayout,
            index.get(),
            function.call_loan_layouts().len(),
        ),
        LinkedInstructionTarget::Function(index) => (
            CandidateReferenceKind::Function,
            index.get(),
            parts.functions.len(),
        ),
        LinkedInstructionTarget::ServiceOperation(index) => (
            CandidateReferenceKind::ServiceOperation,
            index.get(),
            parts.service_operations.len(),
        ),
        LinkedInstructionTarget::ActorMethod(index) => (
            CandidateReferenceKind::ActorMethod,
            index.get(),
            parts.actor_methods.len(),
        ),
        LinkedInstructionTarget::InterfaceTable(index) => (
            CandidateReferenceKind::InterfaceTable,
            index.get(),
            parts.interface_tables.len(),
        ),
        LinkedInstructionTarget::SyntheticCallback(index) => (
            CandidateReferenceKind::SyntheticCallback,
            index.get(),
            parts.synthetic_callbacks.len(),
        ),
        LinkedInstructionTarget::HostEffectAdapter(index) => (
            CandidateReferenceKind::HostEffectAdapter,
            index.get(),
            parts.host_effect_adapters.len(),
        ),
        LinkedInstructionTarget::Intrinsic(index) => (
            CandidateReferenceKind::Intrinsic,
            index.get(),
            parts.intrinsics.len(),
        ),
        LinkedInstructionTarget::Constant(index) => (
            CandidateReferenceKind::Constant,
            index.get(),
            parts.constants.len(),
        ),
        LinkedInstructionTarget::Type(index) => {
            (CandidateReferenceKind::Type, index.get(), parts.types.len())
        }
        LinkedInstructionTarget::Shape(index) => (
            CandidateReferenceKind::Shape,
            index.get(),
            parts.shapes.len(),
        ),
        LinkedInstructionTarget::WritablePath(index) => (
            CandidateReferenceKind::WritablePath,
            index.get(),
            parts.writable_paths.len(),
        ),
        LinkedInstructionTarget::CallbackCaptureLayout(index) => (
            CandidateReferenceKind::CallbackCaptureLayout,
            index.get(),
            parts.callback_capture_layouts.len(),
        ),
        LinkedInstructionTarget::ResumeSite(index) => (
            CandidateReferenceKind::ResumeSite,
            index.get(),
            parts.resume_sites.len(),
        ),
    };
    check_index(location, kind, index, len)
}

fn validate_function_tables(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_exception_regions(function, parts)?;
    validate_active_regions(function)?;
    validate_switch_tables(function, parts)?;
    validate_call_loan_layouts(function, parts)?;
    validate_statement_and_source_tables(function)
}

fn validate_call_loan_layouts(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let function_location = CandidateLocation::Function {
        function: function.index(),
    };
    for (position, layout) in function.call_loan_layouts().iter().enumerate() {
        let expected = position_u32(
            CandidateTable::Functions,
            position,
            function.call_loan_layouts().len(),
        )?;
        if layout.index().get() != expected {
            return Err(LinkedBytecodeCandidateError::NonDenseIndex {
                table: CandidateTable::Functions,
                position,
                expected,
                actual: layout.index().get(),
            });
        }
        for loan in layout.loans() {
            check_index(
                function_location,
                CandidateReferenceKind::FrameSlot,
                loan.root_slot().get(),
                function.frame().slot_types().len(),
            )?;
            if function
                .frame()
                .writable_local_slots()
                .binary_search(&loan.root_slot())
                .is_err()
            {
                return Err(LinkedBytecodeCandidateError::LoanRootNotWritable {
                    function: function.index(),
                    layout: layout.index(),
                    root_slot: loan.root_slot(),
                });
            }
            check_index(
                function_location,
                CandidateReferenceKind::WritablePath,
                loan.writable_path().get(),
                parts.writable_paths.len(),
            )?;
            let Some(path) = parts
                .writable_paths
                .get(loan.writable_path().get() as usize)
            else {
                continue;
            };
            if path.origin().specialization() != Some(function.key()) {
                return Err(
                    LinkedBytecodeCandidateError::LoanPathSpecializationMismatch {
                        function: function.index(),
                        layout: layout.index(),
                        path: loan.writable_path(),
                    },
                );
            }
            if function
                .frame()
                .slot_types()
                .get(loan.root_slot().get() as usize)
                != Some(&path.root_type())
            {
                return Err(LinkedBytecodeCandidateError::LoanRootTypeMismatch {
                    function: function.index(),
                    layout: layout.index(),
                    root_slot: loan.root_slot(),
                    path: loan.writable_path(),
                });
            }
        }
    }
    Ok(())
}

fn validate_exception_regions(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let instruction_len = function.instructions().len();
    let frame_len = function.frame().slot_types().len();
    let function_location = CandidateLocation::Function {
        function: function.index(),
    };
    for region in function.exception_regions() {
        check_index(
            function_location,
            CandidateReferenceKind::Instruction,
            region.start().get(),
            instruction_len,
        )?;
        check_boundary(function_location, region.end().get(), instruction_len)?;
        check_index(
            function_location,
            CandidateReferenceKind::Instruction,
            region.handler().get(),
            instruction_len,
        )?;
        if region.start().get() >= region.end().get() {
            return Err(LinkedBytecodeCandidateError::InvalidRange {
                location: function_location,
                start: region.start().get(),
                end: region.end().get(),
            });
        }
        check_index(
            function_location,
            CandidateReferenceKind::FrameSlot,
            region.catch_slot().get(),
            frame_len,
        )?;
        check_index(
            function_location,
            CandidateReferenceKind::Type,
            region.catch_slot_type().get(),
            parts.types.len(),
        )?;
        for matcher in region.catch_matchers() {
            if let crate::LinkedCatchMatcher::Type(ty) = matcher {
                check_index(
                    function_location,
                    CandidateReferenceKind::Type,
                    ty.get(),
                    parts.types.len(),
                )?;
            }
        }
    }
    Ok(())
}

fn validate_active_regions(
    function: &crate::LinkedFunction,
) -> Result<(), LinkedBytecodeCandidateError> {
    let instruction_len = function.instructions().len();
    let function_location = CandidateLocation::Function {
        function: function.index(),
    };
    for (position, region) in function.active_regions().iter().enumerate() {
        let expected = position_u32(
            CandidateTable::Functions,
            position,
            function.active_regions().len(),
        )?;
        if region.index().get() != expected {
            return Err(LinkedBytecodeCandidateError::NonDenseIndex {
                table: CandidateTable::Functions,
                position,
                expected,
                actual: region.index().get(),
            });
        }
        check_index(
            function_location,
            CandidateReferenceKind::Instruction,
            region.start().get(),
            instruction_len,
        )?;
        check_boundary(function_location, region.end().get(), instruction_len)?;
        if region.start().get() >= region.end().get() {
            return Err(LinkedBytecodeCandidateError::InvalidRange {
                location: function_location,
                start: region.start().get(),
                end: region.end().get(),
            });
        }
    }
    Ok(())
}

fn validate_switch_tables(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let instruction_len = function.instructions().len();
    let function_location = CandidateLocation::Function {
        function: function.index(),
    };
    for table in function.switch_tables() {
        check_index(
            function_location,
            CandidateReferenceKind::Instruction,
            table.default_target().get(),
            instruction_len,
        )?;
        for case in table.cases() {
            check_index(
                function_location,
                CandidateReferenceKind::Type,
                case.tag_type().get(),
                parts.types.len(),
            )?;
            check_index(
                function_location,
                CandidateReferenceKind::Instruction,
                case.target().get(),
                instruction_len,
            )?;
        }
    }
    Ok(())
}

fn validate_statement_and_source_tables(
    function: &crate::LinkedFunction,
) -> Result<(), LinkedBytecodeCandidateError> {
    let instruction_len = function.instructions().len();
    let function_location = CandidateLocation::Function {
        function: function.index(),
    };
    for statement in function.statement_entries() {
        check_index(
            function_location,
            CandidateReferenceKind::Instruction,
            statement.instruction().get(),
            instruction_len,
        )?;
    }
    let mut statement_entries = Vec::with_capacity(function.statement_entries().len());
    for statement in function.statement_entries() {
        statement_entries.push(skiff_artifact_model::StatementEntry {
            pc: statement.instruction().get(),
            sequence_ordinal: statement.sequence_ordinal(),
            attribution_id: statement.attribution_id(),
            site: statement.site().clone(),
        });
    }
    skiff_artifact_model::validate_statement_entries_canonical(&statement_entries).map_err(
        |source| LinkedBytecodeCandidateError::NonCanonicalStatementEntries {
            function: function.index(),
            source,
        },
    )?;
    for source in function.source_map() {
        check_index(
            function_location,
            CandidateReferenceKind::Instruction,
            source.start().get(),
            instruction_len,
        )?;
        check_boundary(function_location, source.end().get(), instruction_len)?;
        if source.start().get() >= source.end().get() {
            return Err(LinkedBytecodeCandidateError::InvalidRange {
                location: function_location,
                start: source.start().get(),
                end: source.end().get(),
            });
        }
    }
    Ok(())
}

fn validate_stack_map(
    function: &crate::LinkedFunction,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let entries = function.stack_map().entries();
    if entries.len() != function.instructions().len() {
        return Err(LinkedBytecodeCandidateError::ProgramPointCountMismatch {
            function: function.index(),
            instruction_count: function.instructions().len(),
            state_count: entries.len(),
        });
    }
    for (position, state) in entries.iter().enumerate() {
        let expected = position_u32(
            CandidateTable::Functions,
            position,
            function.instructions().len(),
        )?;
        let location = CandidateLocation::Instruction {
            function: function.index(),
            instruction: state.instruction(),
        };
        if state.instruction().get() != expected {
            return Err(LinkedBytecodeCandidateError::NonDenseIndex {
                table: CandidateTable::Functions,
                position,
                expected,
                actual: state.instruction().get(),
            });
        }
        if state.slots_before().len() != function.frame().slot_types().len() {
            return Err(
                LinkedBytecodeCandidateError::ProgramPointSlotCountMismatch {
                    function: function.index(),
                    instruction: state.instruction(),
                    slot_count: function.frame().slot_types().len(),
                    state_count: state.slots_before().len(),
                },
            );
        }
        for value in state.stack_before() {
            validate_stack_value(value, location, parts)?;
        }
        if state.stack_before().len() > function.max_operand_depth() as usize {
            return Err(
                LinkedBytecodeCandidateError::ProgramPointOperandDepthExceeded {
                    function: function.index(),
                    instruction: state.instruction(),
                    declared_max: function.max_operand_depth(),
                    actual: state.stack_before().len(),
                },
            );
        }
        for slot in state.slots_before() {
            if let LinkedSlotState::Live(value) = slot {
                validate_stack_value(value, location, parts)?;
            }
        }
        for region in state.active_regions() {
            check_index(
                location,
                CandidateReferenceKind::ActiveRegion,
                region.get(),
                function.active_regions().len(),
            )?;
        }
        for loan in state.writable_loans() {
            check_index(
                location,
                CandidateReferenceKind::FrameSlot,
                loan.root_slot().get(),
                function.frame().slot_types().len(),
            )?;
            check_index(
                location,
                CandidateReferenceKind::WritablePath,
                loan.path().get(),
                parts.writable_paths.len(),
            )?;
        }
    }
    Ok(())
}

fn validate_stack_value(
    value: &crate::LinkedStackValue,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    check_index(
        location,
        CandidateReferenceKind::Type,
        value.ty().get(),
        parts.types.len(),
    )?;
    validate_plan(value.plan(), location, parts)
}
