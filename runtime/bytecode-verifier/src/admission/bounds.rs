use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedBytecodeCandidate, LinkedCallableSignature, LinkedInterfaceTableKind,
    LinkedNativeCallableSignature,
};

use crate::{
    VerificationError, VerificationLimit, VerificationLimits, VerificationLocation,
    VerificationObligation,
};

pub(super) fn prove_checked_budgets(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    // This stage bounds only facts whose size can be counted without trusting
    // semantic claims. CFG edges and type nesting are deliberately left to
    // their independent recomputation stages, which remain fail-closed.
    check_image_tables(candidate, limits)?;
    check_limit(
        VerificationLimit::Functions,
        count(candidate.functions())?,
        limits.max_functions,
        VerificationLocation::Image,
    )?;

    let mut total_instructions = 0_u64;
    for function in candidate.functions() {
        let location = VerificationLocation::Function {
            function: function.index(),
        };
        let instructions = count(function.instructions())?;
        check_limit(
            VerificationLimit::InstructionsPerFunction,
            instructions,
            limits.max_instructions_per_function,
            location,
        )?;
        total_instructions = checked_add(
            total_instructions,
            instructions,
            VerificationLimit::TotalInstructions,
            limits.max_total_instructions,
            location,
        )?;
        check_function_budgets(function, limits)?;
    }
    check_limit(
        VerificationLimit::TotalInstructions,
        total_instructions,
        limits.max_total_instructions,
        VerificationLocation::Image,
    )?;
    check_data_budgets(candidate, limits)
}

fn check_image_tables(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let tables = [
        (CandidateTable::Packages, candidate.packages().len()),
        (CandidateTable::Functions, candidate.functions().len()),
        (
            CandidateTable::OperationEntries,
            candidate.operation_entries().len(),
        ),
        (
            CandidateTable::GatewayEntries,
            candidate.gateway_entries().len(),
        ),
        (
            CandidateTable::ExactLocalTargets,
            candidate.exact_local_targets().len(),
        ),
        (
            CandidateTable::ServiceOperations,
            candidate.service_operations().len(),
        ),
        (
            CandidateTable::ActorCreates,
            candidate.actor_creates().len(),
        ),
        (
            CandidateTable::ActorMethods,
            candidate.actor_methods().len(),
        ),
        (
            CandidateTable::InterfaceTables,
            candidate.interface_tables().len(),
        ),
        (
            CandidateTable::SyntheticCallbacks,
            candidate.synthetic_callbacks().len(),
        ),
        (
            CandidateTable::CallbackCaptureLayouts,
            candidate.callback_capture_layouts().len(),
        ),
        (
            CandidateTable::HostEffectAdapters,
            candidate.host_effect_adapters().len(),
        ),
        (CandidateTable::Intrinsics, candidate.intrinsics().len()),
        (CandidateTable::Types, candidate.types().len()),
        (CandidateTable::Shapes, candidate.shapes().len()),
        (CandidateTable::Constants, candidate.constants().len()),
        (
            CandidateTable::ConstantRoots,
            candidate.constant_roots().len(),
        ),
        (
            CandidateTable::FrozenConstantNodes,
            candidate.frozen_constant_nodes().len(),
        ),
        (CandidateTable::ResumeSites, candidate.resume_sites().len()),
        (
            CandidateTable::WritablePaths,
            candidate.writable_paths().len(),
        ),
    ];
    for (table, len) in tables {
        check_limit(
            VerificationLimit::ImageTableEntries,
            count_len(len)?,
            limits.max_image_table_entries,
            VerificationLocation::Table { table, row: 0 },
        )?;
    }
    Ok(())
}

fn check_function_budgets(
    function: &skiff_runtime_linked_bytecode::LinkedFunction,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let location = VerificationLocation::Function {
        function: function.index(),
    };
    check_limit(
        VerificationLimit::FrameSlotsPerFunction,
        count(function.frame().slot_types())?,
        limits.max_frame_slots_per_function,
        location,
    )?;
    check_limit(
        VerificationLimit::OperandDepth,
        u64::from(function.max_operand_depth()),
        limits.max_operand_depth,
        location,
    )?;
    check_limit(
        VerificationLimit::ExceptionRegionsPerFunction,
        count(function.exception_regions())?,
        limits.max_exception_regions_per_function,
        location,
    )?;
    check_switch_targets(function, limits, location)?;
    check_debug_entries(function, limits, location)?;
    check_arity(count(function.frame().parameters())?, limits, location)?;
    check_arity(count(function.frame().result_types())?, limits, location)?;
    for layout in function.call_loan_layouts() {
        check_arity(count(layout.loans())?, limits, location)?;
    }
    Ok(())
}

fn check_switch_targets(
    function: &skiff_runtime_linked_bytecode::LinkedFunction,
    limits: &VerificationLimits,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let mut targets = 0_u64;
    for table in function.switch_tables() {
        let row_targets = count(table.cases())?.checked_add(1).ok_or_else(|| {
            arithmetic_overflow(location, "counting a switch table default target")
        })?;
        targets = checked_add(
            targets,
            row_targets,
            VerificationLimit::SwitchTargetsPerFunction,
            limits.max_switch_targets_per_function,
            location,
        )?;
    }
    check_limit(
        VerificationLimit::SwitchTargetsPerFunction,
        targets,
        limits.max_switch_targets_per_function,
        location,
    )
}

fn check_debug_entries(
    function: &skiff_runtime_linked_bytecode::LinkedFunction,
    limits: &VerificationLimits,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let debug_entries = count(function.statement_entries())?
        .checked_add(count(function.source_map())?)
        .ok_or_else(|| arithmetic_overflow(location, "counting function debug entries"))?;
    check_limit(
        VerificationLimit::DebugEntriesPerFunction,
        debug_entries,
        limits.max_debug_entries_per_function,
        location,
    )
}

fn check_data_budgets(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    for shape in candidate.shapes() {
        check_arity(
            count(shape.fields())?,
            limits,
            table_location(CandidateTable::Shapes, shape.index().get()),
        )?;
    }
    for path in candidate.writable_paths() {
        check_arity(
            count(path.segments())?,
            limits,
            table_location(CandidateTable::WritablePaths, path.index().get()),
        )?;
    }
    for layout in candidate.callback_capture_layouts() {
        check_limit(
            VerificationLimit::CallbackCapturesPerCallback,
            count(layout.captures())?,
            limits.max_callback_captures_per_callback,
            table_location(CandidateTable::CallbackCaptureLayouts, layout.index().get()),
        )?;
    }
    check_entry_arities(candidate, limits)?;
    check_constant_edges(candidate, limits)
}

fn check_entry_arities(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    for (row, entry) in candidate.operation_entries().iter().enumerate() {
        check_callable_arity(
            entry.signature(),
            limits,
            table_location(CandidateTable::OperationEntries, row_u32(row)?),
        )?;
    }
    for (row, entry) in candidate.gateway_entries().iter().enumerate() {
        let location = table_location(CandidateTable::GatewayEntries, row_u32(row)?);
        for callable in entry.callables() {
            check_callable_arity(callable.signature(), limits, location)?;
        }
    }
    for target in candidate.service_operations() {
        check_callable_arity(
            target.signature(),
            limits,
            table_location(CandidateTable::ServiceOperations, target.index().get()),
        )?;
    }
    check_dispatch_arities(candidate, limits)
}

fn check_dispatch_arities(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    for target in candidate.actor_creates() {
        check_callable_arity(
            target.signature(),
            limits,
            table_location(CandidateTable::ActorCreates, target.index().get()),
        )?;
    }
    for target in candidate.actor_methods() {
        check_callable_arity(
            target.signature(),
            limits,
            table_location(CandidateTable::ActorMethods, target.index().get()),
        )?;
    }
    for target in candidate.synthetic_callbacks() {
        check_callable_arity(
            target.signature(),
            limits,
            table_location(CandidateTable::SyntheticCallbacks, target.index().get()),
        )?;
    }
    for target in candidate.host_effect_adapters() {
        check_native_arity(
            target.signature(),
            limits,
            table_location(CandidateTable::HostEffectAdapters, target.index().get()),
        )?;
    }
    for target in candidate.intrinsics() {
        check_native_arity(
            target.signature(),
            limits,
            table_location(CandidateTable::Intrinsics, target.index().get()),
        )?;
    }
    check_interface_arities(candidate, limits)
}

fn check_interface_arities(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    for table in candidate.interface_tables() {
        let location = table_location(CandidateTable::InterfaceTables, table.index().get());
        check_arity(
            count(table.interface().concrete_type_arguments())?,
            limits,
            location,
        )?;
        match table.kind() {
            LinkedInterfaceTableKind::Requirement(table)
            | LinkedInterfaceTableKind::Callback(table) => {
                check_arity(count(table.methods())?, limits, location)?;
                for method in table.methods() {
                    check_callable_arity(method.signature(), limits, location)?;
                }
            }
            LinkedInterfaceTableKind::Local(table) => {
                check_arity(count(table.methods())?, limits, location)?;
                for method in table.methods() {
                    check_callable_arity(method.signature(), limits, location)?;
                }
            }
            LinkedInterfaceTableKind::Remote(table) => {
                check_arity(count(table.methods())?, limits, location)?;
                for method in table.methods() {
                    check_callable_arity(method.signature(), limits, location)?;
                }
            }
        }
    }
    Ok(())
}

fn check_constant_edges(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let mut edges = 0_u64;
    for node in candidate.frozen_constant_nodes() {
        edges = checked_add(
            edges,
            count(node.value().children())?,
            VerificationLimit::ConstantGraphEdges,
            limits.max_constant_graph_edges,
            table_location(CandidateTable::FrozenConstantNodes, node.index().get()),
        )?;
    }
    check_limit(
        VerificationLimit::ConstantGraphEdges,
        edges,
        limits.max_constant_graph_edges,
        VerificationLocation::Image,
    )
}

fn check_callable_arity(
    signature: &LinkedCallableSignature,
    limits: &VerificationLimits,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    check_arity(count(signature.parameter_types())?, limits, location)?;
    check_arity(count(signature.result_types())?, limits, location)
}

fn check_native_arity(
    signature: &LinkedNativeCallableSignature,
    limits: &VerificationLimits,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    check_arity(count(signature.parameter_types())?, limits, location)?;
    check_arity(count(signature.result_types())?, limits, location)
}

fn check_arity(
    actual: u64,
    limits: &VerificationLimits,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    check_limit(VerificationLimit::Arity, actual, limits.max_arity, location)
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

fn checked_add(
    current: u64,
    additional: u64,
    limit: VerificationLimit,
    max: u64,
    location: VerificationLocation,
) -> Result<u64, VerificationError> {
    let value = current
        .checked_add(additional)
        .ok_or_else(|| arithmetic_overflow(location, "summing verifier budget counters"))?;
    check_limit(limit, value, max, location)?;
    Ok(value)
}

fn count<T>(slice: &[T]) -> Result<u64, VerificationError> {
    count_len(slice.len())
}

fn count_len(len: usize) -> Result<u64, VerificationError> {
    u64::try_from(len)
        .map_err(|_| arithmetic_overflow(VerificationLocation::Image, "counting rows"))
}

fn row_u32(row: usize) -> Result<u32, VerificationError> {
    u32::try_from(row)
        .map_err(|_| arithmetic_overflow(VerificationLocation::Image, "naming a table row"))
}

const fn table_location(table: CandidateTable, row: u32) -> VerificationLocation {
    VerificationLocation::Table { table, row }
}

fn arithmetic_overflow(location: VerificationLocation, detail: &'static str) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ResourceAccounting,
        location,
        detail: detail.to_string(),
    }
}
