use std::collections::BTreeSet;

use crate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, LinkedArtifactPoolOrigin,
    LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts, LinkedInterfaceTable,
    LinkedInterfaceTableKind, SpecializationKey,
};

mod callbacks;
mod data;
mod functions;
mod plans;
mod types;
mod unique;

use plans::{validate_callable_signature, validate_native_signature};

pub(super) fn validate_parts(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_dense_tables(parts)?;
    unique::validate_unique_keys(parts)?;
    validate_all_references(parts)
}

fn validate_dense_tables(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_dense(CandidateTable::Packages, &parts.packages, |row| {
        row.index().get()
    })?;
    validate_dense(CandidateTable::Functions, &parts.functions, |row| {
        row.index().get()
    })?;
    validate_dense(
        CandidateTable::ServiceOperations,
        &parts.service_operations,
        |row| row.index().get(),
    )?;
    validate_dense(CandidateTable::ActorCreates, &parts.actor_creates, |row| {
        row.index().get()
    })?;
    validate_dense(CandidateTable::ActorMethods, &parts.actor_methods, |row| {
        row.index().get()
    })?;
    validate_dense(
        CandidateTable::InterfaceTables,
        &parts.interface_tables,
        |row| row.index().get(),
    )?;
    validate_dense(
        CandidateTable::SyntheticCallbacks,
        &parts.synthetic_callbacks,
        |row| row.index().get(),
    )?;
    validate_dense(
        CandidateTable::CallbackCaptureLayouts,
        &parts.callback_capture_layouts,
        |row| row.index().get(),
    )?;
    validate_dense(
        CandidateTable::HostEffectAdapters,
        &parts.host_effect_adapters,
        |row| row.index().get(),
    )?;
    validate_dense(CandidateTable::Intrinsics, &parts.intrinsics, |row| {
        row.index().get()
    })?;
    validate_dense(CandidateTable::Types, &parts.types, |row| row.index().get())?;
    validate_dense(CandidateTable::Shapes, &parts.shapes, |row| {
        row.index().get()
    })?;
    validate_dense(CandidateTable::Constants, &parts.constants, |row| {
        row.index().get()
    })?;
    validate_dense(
        CandidateTable::FrozenConstantNodes,
        &parts.frozen_constant_nodes,
        |row| row.index().get(),
    )?;
    validate_dense(CandidateTable::ResumeSites, &parts.resume_sites, |row| {
        row.index().get()
    })?;
    validate_dense(
        CandidateTable::WritablePaths,
        &parts.writable_paths,
        |row| row.index().get(),
    )
}

fn validate_dense<T>(
    table: CandidateTable,
    rows: &[T],
    index: impl Fn(&T) -> u32,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut seen = BTreeSet::new();
    for (position, row) in rows.iter().enumerate() {
        let actual = index(row);
        if !seen.insert(actual) {
            return Err(LinkedBytecodeCandidateError::DuplicateIndex {
                table,
                index: actual,
            });
        }
        let expected = position_u32(table, position, rows.len())?;
        if actual != expected {
            return Err(LinkedBytecodeCandidateError::NonDenseIndex {
                table,
                position,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

fn validate_all_references(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let package_ids = parts
        .packages
        .iter()
        .map(|package| package.package_build_id().clone())
        .collect::<BTreeSet<_>>();

    validate_function_references(parts, &package_ids)?;
    validate_entry_references(parts, &package_ids)?;
    validate_dispatch_target_references(parts, &package_ids)?;
    data::validate_data_references(parts, &package_ids)
}

fn validate_function_references(
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    for function in &parts.functions {
        let location = CandidateLocation::Function {
            function: function.index(),
        };
        validate_specialization(function.key(), location, parts, package_ids)?;
        functions::validate_function(function, parts)?;
    }
    Ok(())
}

fn validate_entry_references(
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    for (position, entry) in parts.operation_entries.iter().enumerate() {
        let location = table_location(CandidateTable::OperationEntries, position, parts)?;
        check_index(
            location,
            CandidateReferenceKind::Function,
            entry.function().get(),
            parts.functions.len(),
        )?;
        validate_callable_signature(entry.signature(), location, parts)?;
    }
    for (position, entry) in parts.gateway_entries.iter().enumerate() {
        let location = table_location(CandidateTable::GatewayEntries, position, parts)?;
        for callable in entry.callables() {
            check_index(
                location,
                CandidateReferenceKind::Function,
                callable.function().get(),
                parts.functions.len(),
            )?;
            if let Some(function) = parts.functions.get(callable.function().get() as usize) {
                if function.key().template_function_key() != callable.package_callable_id() {
                    return Err(
                        LinkedBytecodeCandidateError::GatewayCallableFunctionMismatch {
                            gateway_entry_key: entry.gateway_entry_key().clone(),
                            role: callable.role(),
                            function: callable.function(),
                        },
                    );
                }
            }
            validate_callable_signature(callable.signature(), location, parts)?;
        }
    }
    for (position, target) in parts.exact_local_targets.iter().enumerate() {
        let location = table_location(CandidateTable::ExactLocalTargets, position, parts)?;
        check_index(
            location,
            CandidateReferenceKind::Function,
            target.function().get(),
            parts.functions.len(),
        )?;
        validate_specialization(target.key(), location, parts, package_ids)?;
        if let Some(function) = parts.functions.get(target.function().get() as usize) {
            if function.key() != target.key() {
                return Err(
                    LinkedBytecodeCandidateError::ExactLocalTargetFunctionMismatch {
                        row: position_u32(
                            CandidateTable::ExactLocalTargets,
                            position,
                            parts.exact_local_targets.len(),
                        )?,
                        function: target.function(),
                    },
                );
            }
        }
    }
    Ok(())
}

fn validate_dispatch_target_references(
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    for target in &parts.service_operations {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::ServiceOperations,
            row: target.index().get(),
        };
        check_package(
            location,
            &target.service_requirement_key().caller_package_build_id,
            package_ids,
        )?;
        validate_callable_signature(target.signature(), location, parts)?;
    }
    for target in &parts.actor_creates {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::ActorCreates,
            row: target.index().get(),
        };
        check_package(location, target.owner_package_build_id(), package_ids)?;
        check_index(
            location,
            CandidateReferenceKind::Function,
            target.function().get(),
            parts.functions.len(),
        )?;
        if let Some(function) = parts.functions.get(target.function().get() as usize) {
            if function.key().package_build_id() != target.owner_package_build_id() {
                return Err(
                    LinkedBytecodeCandidateError::ActorCreateTargetFunctionOwnerMismatch {
                        actor_create: target.index(),
                        function: target.function(),
                    },
                );
            }
        }
        validate_callable_signature(target.signature(), location, parts)?;
    }
    for target in &parts.actor_methods {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::ActorMethods,
            row: target.index().get(),
        };
        check_package(location, target.owner_package_build_id(), package_ids)?;
        check_index(
            location,
            CandidateReferenceKind::Function,
            target.function().get(),
            parts.functions.len(),
        )?;
        if let Some(function) = parts.functions.get(target.function().get() as usize) {
            if function.key().package_build_id() != target.owner_package_build_id() {
                return Err(
                    LinkedBytecodeCandidateError::ActorTargetFunctionOwnerMismatch {
                        actor_method: target.index(),
                        function: target.function(),
                    },
                );
            }
        }
        validate_callable_signature(target.signature(), location, parts)?;
    }
    for table in &parts.interface_tables {
        validate_interface_table(table, parts, package_ids)?;
    }
    for target in &parts.synthetic_callbacks {
        callbacks::validate_synthetic_callback(target, parts)?;
    }
    for layout in &parts.callback_capture_layouts {
        callbacks::validate_capture_layout(layout, parts, package_ids)?;
    }
    for target in &parts.host_effect_adapters {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::HostEffectAdapters,
            row: target.index().get(),
        };
        validate_native_signature(target.signature(), location, parts)?;
    }
    for target in &parts.intrinsics {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::Intrinsics,
            row: target.index().get(),
        };
        validate_native_signature(target.signature(), location, parts)?;
    }
    Ok(())
}

fn validate_specialization(
    key: &SpecializationKey,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    check_package(location, key.package_build_id(), package_ids)?;
    for ty in key.concrete_type_arguments() {
        check_index(
            location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        )?;
    }
    if let Some(receiver) = key.concrete_receiver() {
        check_index(
            location,
            CandidateReferenceKind::Type,
            receiver.get(),
            parts.types.len(),
        )?;
    }
    Ok(())
}

fn validate_origin<I>(
    origin: &LinkedArtifactPoolOrigin<I>,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    check_package(location, origin.package_build_id(), package_ids)?;
    if let Some(specialization) = origin.specialization() {
        validate_specialization(specialization, location, parts, package_ids)?;
        if !parts
            .functions
            .iter()
            .any(|function| function.key() == specialization)
        {
            return Err(LinkedBytecodeCandidateError::MissingOriginSpecialization {
                location,
                key: specialization.clone(),
            });
        }
    }
    Ok(())
}

fn validate_interface_table(
    table: &LinkedInterfaceTable,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::InterfaceTables,
        row: table.index().get(),
    };
    for ty in table.interface().concrete_type_arguments() {
        check_index(
            location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        )?;
    }
    match table.kind() {
        LinkedInterfaceTableKind::Requirement(requirement)
        | LinkedInterfaceTableKind::Callback(requirement) => {
            for method in requirement.methods() {
                validate_callable_signature(method.signature(), location, parts)?;
            }
        }
        LinkedInterfaceTableKind::Local(local) => {
            check_index(
                location,
                CandidateReferenceKind::Type,
                local.concrete_type().get(),
                parts.types.len(),
            )?;
            for method in local.methods() {
                check_index(
                    location,
                    CandidateReferenceKind::Function,
                    method.function().get(),
                    parts.functions.len(),
                )?;
                validate_callable_signature(method.signature(), location, parts)?;
            }
        }
        LinkedInterfaceTableKind::Remote(remote) => {
            check_package(
                location,
                &remote.service_requirement_key().caller_package_build_id,
                package_ids,
            )?;
            for method in remote.methods() {
                validate_callable_signature(method.signature(), location, parts)?;
            }
        }
    }
    Ok(())
}

fn check_package(
    location: CandidateLocation,
    package_build_id: &skiff_artifact_model::PackageBuildId,
    package_ids: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    if !package_ids.contains(package_build_id) {
        return Err(LinkedBytecodeCandidateError::MissingPackageProvenance {
            location,
            package_build_id: package_build_id.clone(),
        });
    }
    Ok(())
}

fn check_index(
    location: CandidateLocation,
    reference: CandidateReferenceKind,
    index: u32,
    len: usize,
) -> Result<(), LinkedBytecodeCandidateError> {
    if index as usize >= len {
        return Err(LinkedBytecodeCandidateError::ReferenceOutOfBounds {
            location,
            reference,
            index,
            len,
        });
    }
    Ok(())
}

fn check_boundary(
    location: CandidateLocation,
    index: u32,
    instruction_len: usize,
) -> Result<(), LinkedBytecodeCandidateError> {
    if index as usize > instruction_len {
        return Err(LinkedBytecodeCandidateError::ReferenceOutOfBounds {
            location,
            reference: CandidateReferenceKind::InstructionBoundary,
            index,
            len: instruction_len.saturating_add(1),
        });
    }
    Ok(())
}

fn table_location(
    table: CandidateTable,
    position: usize,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<CandidateLocation, LinkedBytecodeCandidateError> {
    Ok(CandidateLocation::TableRow {
        table,
        row: position_u32(table, position, table_len(table, parts))?,
    })
}

fn table_len(table: CandidateTable, parts: &LinkedBytecodeCandidateParts) -> usize {
    match table {
        CandidateTable::Packages => parts.packages.len(),
        CandidateTable::Functions => parts.functions.len(),
        CandidateTable::OperationEntries => parts.operation_entries.len(),
        CandidateTable::GatewayEntries => parts.gateway_entries.len(),
        CandidateTable::ExactLocalTargets => parts.exact_local_targets.len(),
        CandidateTable::ServiceOperations => parts.service_operations.len(),
        CandidateTable::ActorCreates => parts.actor_creates.len(),
        CandidateTable::ActorMethods => parts.actor_methods.len(),
        CandidateTable::InterfaceTables => parts.interface_tables.len(),
        CandidateTable::SyntheticCallbacks => parts.synthetic_callbacks.len(),
        CandidateTable::CallbackCaptureLayouts => parts.callback_capture_layouts.len(),
        CandidateTable::HostEffectAdapters => parts.host_effect_adapters.len(),
        CandidateTable::Intrinsics => parts.intrinsics.len(),
        CandidateTable::Types => parts.types.len(),
        CandidateTable::Shapes => parts.shapes.len(),
        CandidateTable::Constants => parts.constants.len(),
        CandidateTable::ConstantRoots => parts.constant_roots.len(),
        CandidateTable::FrozenConstantNodes => parts.frozen_constant_nodes.len(),
        CandidateTable::ResumeSites => parts.resume_sites.len(),
        CandidateTable::WritablePaths => parts.writable_paths.len(),
    }
}

fn position_u32(
    table: CandidateTable,
    position: usize,
    len: usize,
) -> Result<u32, LinkedBytecodeCandidateError> {
    u32::try_from(position).map_err(|_| LinkedBytecodeCandidateError::TableTooLarge { table, len })
}
