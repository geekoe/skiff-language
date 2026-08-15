use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CandidateTable, LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts,
    LinkedInterfaceTable, LinkedInterfaceTableKind,
};

use super::position_u32;

pub(super) fn validate_unique_keys(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_package_order(parts)?;

    let mut function_keys = BTreeSet::new();
    let mut previous_function_key: Option<&crate::SpecializationKey> = None;
    for function in &parts.functions {
        if !function_keys.insert(function.key().clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateFunctionKey {
                key: function.key().clone(),
            });
        }
        if let Some(previous) = previous_function_key {
            if previous > function.key() {
                return Err(LinkedBytecodeCandidateError::NonCanonicalFunctionOrder {
                    previous: Box::new(previous.clone()),
                    current: Box::new(function.key().clone()),
                });
            }
        }
        previous_function_key = Some(function.key());
    }

    let mut exact_local_keys = BTreeSet::new();
    let mut previous_exact_local: Option<&crate::SpecializationKey> = None;
    for target in &parts.exact_local_targets {
        if !exact_local_keys.insert(target.key().clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateExactLocalTarget {
                key: target.key().clone(),
            });
        }
        if let Some(previous) = previous_exact_local {
            if previous > target.key() {
                return Err(
                    LinkedBytecodeCandidateError::NonCanonicalExactLocalTargetOrder {
                        previous: Box::new(previous.clone()),
                        current: Box::new(target.key().clone()),
                    },
                );
            }
        }
        previous_exact_local = Some(target.key());
    }

    validate_operation_entry_order(parts)?;
    validate_gateway_entry_order(parts)?;
    validate_constant_root_order(parts)?;

    let mut service_operations = BTreeSet::new();
    for target in &parts.service_operations {
        let key = (
            target.service_requirement_key().clone(),
            target.contract_operation_id().clone(),
        );
        if !service_operations.insert(key.clone()) {
            return Err(LinkedBytecodeCandidateError::DuplicateServiceOperation {
                service_requirement_key: key.0,
                contract_operation_id: key.1,
            });
        }
    }

    validate_actor_create_order(parts)?;
    validate_actor_method_order(parts)?;

    for (index, table) in parts.interface_tables.iter().enumerate() {
        if let Some(first) = parts.interface_tables[..index]
            .iter()
            .position(|candidate| same_interface_target(candidate, table))
        {
            return Err(LinkedBytecodeCandidateError::DuplicateInterfaceTable {
                first_index: position_u32(
                    CandidateTable::InterfaceTables,
                    first,
                    parts.interface_tables.len(),
                )?,
                duplicate_index: position_u32(
                    CandidateTable::InterfaceTables,
                    index,
                    parts.interface_tables.len(),
                )?,
            });
        }
    }

    let mut resume_sites = BTreeMap::new();
    for resume in &parts.resume_sites {
        let key = (resume.function(), resume.site());
        if let Some(first_index) = resume_sites.insert(key, resume.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateResumeSite {
                first_index,
                duplicate_index: resume.index().get(),
                function: resume.function(),
                site: resume.site(),
            });
        }
    }

    validate_artifact_origins(parts)
}

fn validate_package_order(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut previous = None;
    for package in &parts.packages {
        let current = package.package_build_id();
        if let Some(previous) = previous {
            match current.cmp(previous) {
                Ordering::Equal => {
                    return Err(LinkedBytecodeCandidateError::DuplicatePackage {
                        package_build_id: current.clone(),
                    });
                }
                Ordering::Less => {
                    return Err(LinkedBytecodeCandidateError::NonCanonicalPackageOrder {
                        previous: previous.clone(),
                        current: current.clone(),
                    });
                }
                Ordering::Greater => {}
            }
        }
        previous = Some(current);
    }
    Ok(())
}

fn validate_actor_method_order(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    for adjacent in parts.actor_methods.windows(2) {
        let previous = &adjacent[0];
        let current = &adjacent[1];
        match compare_actor_targets(previous, current) {
            Ordering::Equal => {
                return Err(LinkedBytecodeCandidateError::DuplicateActorMethod {
                    first_index: previous.index().get(),
                    duplicate_index: current.index().get(),
                });
            }
            Ordering::Greater => {
                return Err(LinkedBytecodeCandidateError::NonCanonicalActorMethodOrder {
                    previous_index: previous.index().get(),
                    current_index: current.index().get(),
                });
            }
            Ordering::Less => {}
        }
    }
    Ok(())
}

fn validate_actor_create_order(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    for adjacent in parts.actor_creates.windows(2) {
        let previous = &adjacent[0];
        let current = &adjacent[1];
        match compare_actor_creates(previous, current) {
            Ordering::Equal => {
                return Err(LinkedBytecodeCandidateError::DuplicateActorCreate {
                    first_index: previous.index().get(),
                    duplicate_index: current.index().get(),
                });
            }
            Ordering::Greater => {
                return Err(LinkedBytecodeCandidateError::NonCanonicalActorCreateOrder {
                    previous_index: previous.index().get(),
                    current_index: current.index().get(),
                });
            }
            Ordering::Less => {}
        }
    }
    Ok(())
}

fn compare_actor_creates(
    left: &crate::LinkedActorCreateTarget,
    right: &crate::LinkedActorCreateTarget,
) -> Ordering {
    left.owner_package_build_id()
        .cmp(right.owner_package_build_id())
        .then_with(|| left.actor().module_path.cmp(&right.actor().module_path))
        .then_with(|| left.actor().symbol.cmp(&right.actor().symbol))
}

fn compare_actor_targets(
    left: &crate::LinkedActorMethodTarget,
    right: &crate::LinkedActorMethodTarget,
) -> Ordering {
    left.owner_package_build_id()
        .cmp(right.owner_package_build_id())
        .then_with(|| left.actor().module_path.cmp(&right.actor().module_path))
        .then_with(|| left.actor().symbol.cmp(&right.actor().symbol))
        .then_with(|| left.method_identity().cmp(right.method_identity()))
}

fn same_interface_target(left: &LinkedInterfaceTable, right: &LinkedInterfaceTable) -> bool {
    if left.interface().artifact() != right.interface().artifact()
        || left.interface().concrete_type_arguments() != right.interface().concrete_type_arguments()
    {
        return false;
    }
    match (left.kind(), right.kind()) {
        (
            LinkedInterfaceTableKind::Requirement(left),
            LinkedInterfaceTableKind::Requirement(right),
        ) => left == right,
        (LinkedInterfaceTableKind::Local(left), LinkedInterfaceTableKind::Local(right)) => {
            left.concrete_type() == right.concrete_type()
        }
        (LinkedInterfaceTableKind::Remote(left), LinkedInterfaceTableKind::Remote(right)) => {
            left.service_requirement_key() == right.service_requirement_key()
                && left.public_instance_key() == right.public_instance_key()
        }
        (LinkedInterfaceTableKind::Callback(left), LinkedInterfaceTableKind::Callback(right)) => {
            left == right
        }
        _ => false,
    }
}

fn validate_artifact_origins(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut type_origins = BTreeMap::new();
    for row in &parts.types {
        let key = (
            row.origin().package_build_id().clone(),
            row.origin().artifact_index().get(),
            row.origin().specialization().cloned(),
        );
        if let Some(first_index) = type_origins.insert(key, row.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
                table: CandidateTable::Types,
                first_index,
                duplicate_index: row.index().get(),
            });
        }
    }

    let mut shape_origins = BTreeMap::new();
    for row in &parts.shapes {
        let key = (
            row.origin().package_build_id().clone(),
            row.origin().artifact_index().get(),
            row.origin().specialization().cloned(),
        );
        if let Some(first_index) = shape_origins.insert(key, row.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
                table: CandidateTable::Shapes,
                first_index,
                duplicate_index: row.index().get(),
            });
        }
    }

    let mut constant_origins = BTreeMap::new();
    for row in &parts.constants {
        let key = (
            row.origin().package_build_id().clone(),
            row.origin().artifact_index().get(),
            row.origin().specialization().cloned(),
        );
        if let Some(first_index) = constant_origins.insert(key, row.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
                table: CandidateTable::Constants,
                first_index,
                duplicate_index: row.index().get(),
            });
        }
    }

    let mut node_origins = BTreeMap::new();
    for row in &parts.frozen_constant_nodes {
        let key = (
            row.origin().package_build_id().clone(),
            row.origin().artifact_index().get(),
            row.origin().specialization().cloned(),
        );
        if let Some(first_index) = node_origins.insert(key, row.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
                table: CandidateTable::FrozenConstantNodes,
                first_index,
                duplicate_index: row.index().get(),
            });
        }
    }

    let mut path_origins = BTreeMap::new();
    for row in &parts.writable_paths {
        let key = (
            row.origin().package_build_id().clone(),
            row.origin().artifact_index().get(),
            row.origin().specialization().cloned(),
        );
        if let Some(first_index) = path_origins.insert(key, row.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
                table: CandidateTable::WritablePaths,
                first_index,
                duplicate_index: row.index().get(),
            });
        }
    }

    let mut capture_origins = BTreeMap::new();
    for row in &parts.callback_capture_layouts {
        let key = (
            row.origin().package_build_id().clone(),
            row.origin().artifact_index().get(),
            row.origin().specialization().cloned(),
        );
        if let Some(first_index) = capture_origins.insert(key, row.index().get()) {
            return Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
                table: CandidateTable::CallbackCaptureLayouts,
                first_index,
                duplicate_index: row.index().get(),
            });
        }
    }
    Ok(())
}

fn validate_operation_entry_order(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut previous = None;
    for entry in &parts.operation_entries {
        let current = entry.contract_operation_id();
        if let Some(previous) = previous {
            match current.cmp(previous) {
                Ordering::Equal => {
                    return Err(LinkedBytecodeCandidateError::DuplicateOperationEntry {
                        contract_operation_id: current.clone(),
                    });
                }
                Ordering::Less => {
                    return Err(
                        LinkedBytecodeCandidateError::NonCanonicalOperationEntryOrder {
                            previous: previous.clone(),
                            current: current.clone(),
                        },
                    );
                }
                Ordering::Greater => {}
            }
        }
        previous = Some(current);
    }
    Ok(())
}

fn validate_gateway_entry_order(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut previous = None;
    for entry in &parts.gateway_entries {
        let current = entry.gateway_entry_key();
        if let Some(previous) = previous {
            match current.cmp(previous) {
                Ordering::Equal => {
                    return Err(LinkedBytecodeCandidateError::DuplicateGatewayEntry {
                        gateway_entry_key: current.clone(),
                    });
                }
                Ordering::Less => {
                    return Err(
                        LinkedBytecodeCandidateError::NonCanonicalGatewayEntryOrder {
                            previous: previous.clone(),
                            current: current.clone(),
                        },
                    );
                }
                Ordering::Greater => {}
            }
        }
        previous = Some(current);
    }
    Ok(())
}

fn validate_constant_root_order(
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let mut previous: Option<(&skiff_artifact_model::PackageBuildId, &str)> = None;
    for root in &parts.constant_roots {
        let current = (root.owner_package_build_id(), root.symbol_path().as_str());
        if let Some(previous) = previous {
            match current.cmp(&previous) {
                Ordering::Equal => {
                    return Err(LinkedBytecodeCandidateError::DuplicateConstantRoot {
                        owner_package_build_id: current.0.clone(),
                        symbol_path: current.1.to_string(),
                    });
                }
                Ordering::Less => {
                    return Err(
                        LinkedBytecodeCandidateError::NonCanonicalConstantRootOrder {
                            previous_owner: previous.0.clone(),
                            previous_symbol_path: previous.1.to_string(),
                            current_owner: current.0.clone(),
                            current_symbol_path: current.1.to_string(),
                        },
                    );
                }
                Ordering::Greater => {}
            }
        }
        previous = Some(current);
    }
    Ok(())
}
