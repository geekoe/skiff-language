use std::collections::BTreeMap;

use crate::context::{
    ProjectedServiceDependencyLockEntry, ProjectedServiceDependencyRemoteBoxProvenance,
};
use crate::error::ProjectionError;
use crate::publication_visible_types::{
    projection_visible_interface_instantiation_ref, publication_type_names_from_file_units,
};
use crate::typed_artifacts::ServiceDependencyConstraint;
use skiff_artifact_model::{
    canonical_interface_method_abi_id, BoxSourceIr, ExecutableBody, ExprIr, FileIrUnit,
    InterfaceInstantiationRef, OperationAbiRef, PublicationOperationAbi,
};

pub struct RemoteBoxProvenanceInput<'a> {
    pub dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
    pub service_dependencies: &'a [ServiceDependencyConstraint],
    pub service_file_units: &'a [FileIrUnit],
}

pub fn attach_remote_box_provenance_to_dependency_lock(
    input: RemoteBoxProvenanceInput<'_>,
) -> Result<Vec<ProjectedServiceDependencyLockEntry>, ProjectionError> {
    dependency_lock_with_remote_box_provenance(
        input.dependency_lock,
        input.service_dependencies,
        input.service_file_units,
    )
}

fn dependency_lock_with_remote_box_provenance(
    dependency_lock: &[ProjectedServiceDependencyLockEntry],
    service_dependencies: &[ServiceDependencyConstraint],
    service_file_units: &[FileIrUnit],
) -> Result<Vec<ProjectedServiceDependencyLockEntry>, ProjectionError> {
    let service_dependencies_by_alias = service_dependencies
        .iter()
        .map(|dependency| (dependency.alias.as_str(), dependency))
        .collect::<BTreeMap<_, _>>();
    let publication_type_names = publication_type_names_from_file_units(
        service_file_units
            .iter()
            .map(|unit| (unit.module_path.as_str(), unit)),
    );
    let mut lock = dependency_lock.to_vec();
    let lock_entry_by_alias = lock
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.alias().to_string(), index))
        .collect::<BTreeMap<_, _>>();

    for (source_module, source) in remote_box_sources(service_file_units) {
        let BoxSourceIr::Remote {
            dependency_ref,
            public_instance_key,
            operations,
            callee_protocol_identity,
        } = source
        else {
            continue;
        };
        // The `publication-local direct refs` lowering pass rewrites the box
        // source's interface identity (and the interface identity embedded in
        // each slot's methodAbiId) into direct address form. The published ABI
        // that producers hash — and that we compare against here — is symbolic,
        // so normalize the consumer-captured interface back to symbolic form
        // before recording provenance. Otherwise producer and consumer derive
        // divergent interfaceAbiId/methodAbiId strings and linking fails.
        let interface = projection_visible_interface_instantiation_ref(
            source_module,
            &operations.interface,
            &publication_type_names,
        );
        let interface_display = interface.interface_abi_id.clone();
        let Some(lock_index) = lock_entry_by_alias.get(dependency_ref.as_str()).copied() else {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "remote interface box {}/{} resolved service dependency {}, but dependencyLock has no entry for it",
                    dependency_ref, public_instance_key, dependency_ref
                ),
            });
        };
        let dependency = service_dependencies_by_alias
            .get(dependency_ref.as_str())
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "remote interface box {}/{} resolved unknown service dependency alias {}",
                    dependency_ref, public_instance_key, dependency_ref
                ),
            })?;
        if callee_protocol_identity != &dependency.service_protocol_identity {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "remote interface box {}/{} expected callee protocol identity {}, got {}",
                    dependency_ref,
                    public_instance_key,
                    callee_protocol_identity,
                    dependency.service_protocol_identity
                ),
            });
        }
        for slot in &operations.slots {
            let operation = remote_box_operation(
                dependency,
                dependency_ref,
                public_instance_key,
                &slot.operation_abi_id,
            )?;
            lock[lock_index].add_remote_box_provenance(
                ProjectedServiceDependencyRemoteBoxProvenance {
                    interface: interface.clone(),
                    interface_display: interface_display.clone(),
                    public_instance: public_instance_key.clone(),
                    method_abi_id: normalized_method_abi_id(&interface, &slot.method_abi_id),
                    operation,
                },
            );
        }
    }

    Ok(lock)
}

fn remote_box_sources(service_file_units: &[FileIrUnit]) -> Vec<(&str, &BoxSourceIr)> {
    service_file_units
        .iter()
        .flat_map(|unit| {
            let module_path = unit.module_path.as_str();
            unit.constants
                .iter()
                .flat_map(|constant| remote_box_sources_in_body(&constant.body))
                .chain(
                    unit.executables
                        .iter()
                        .flat_map(|executable| remote_box_sources_in_body(&executable.body)),
                )
                .map(move |source| (module_path, source))
        })
        .collect()
}

/// Recompute a slot's methodAbiId against the symbol-normalized interface so
/// the consumer's provenance methodAbiId matches the producer's published
/// operation. Falls back to the original identifier when it does not carry the
/// expected `method:<interface>:<method>` shape.
fn normalized_method_abi_id(interface: &InterfaceInstantiationRef, method_abi_id: &str) -> String {
    match method_abi_id.rsplit_once(':') {
        Some((_, method_name)) if !method_name.is_empty() => {
            canonical_interface_method_abi_id(interface, method_name)
        }
        _ => method_abi_id.to_string(),
    }
}

fn remote_box_sources_in_body(body: &ExecutableBody) -> impl Iterator<Item = &BoxSourceIr> {
    body.expressions.iter().filter_map(|expr| match expr {
        ExprIr::InterfaceBox {
            source: source @ BoxSourceIr::Remote { .. },
            ..
        } => Some(source),
        _ => None,
    })
}

fn remote_box_operation(
    dependency: &ServiceDependencyConstraint,
    dependency_ref: &str,
    public_instance_key: &str,
    operation_abi_id: &str,
) -> Result<OperationAbiRef, ProjectionError> {
    let operation_abi = dependency
        .publication_abi
        .operation_abi
        .iter()
        .find(|candidate| candidate.operation.operation_abi_id == operation_abi_id)
        .ok_or_else(|| ProjectionError::ContractValidation {
            message: format!(
                "remote interface box {dependency_ref}/{public_instance_key} references operationAbiId {operation_abi_id}, but dependency publication has no operation ABI"
            ),
        })?;
    ensure_remote_operation_exported(
        dependency,
        dependency_ref,
        public_instance_key,
        operation_abi,
    )?;
    Ok(operation_abi.operation.clone())
}

fn ensure_remote_operation_exported(
    dependency: &ServiceDependencyConstraint,
    dependency_ref: &str,
    public_instance_key: &str,
    operation_abi: &PublicationOperationAbi,
) -> Result<(), ProjectionError> {
    let operation = &operation_abi.operation;
    if operation.public_instance_key.as_deref() != Some(public_instance_key) {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "remote interface box {dependency_ref}/{public_instance_key} references operationAbiId {}, but operation publicInstanceKey is {:?}",
                operation.operation_abi_id, operation.public_instance_key
            ),
        });
    }
    if dependency
        .publication_abi
        .operation_exports
        .iter()
        .any(|candidate| candidate == operation)
    {
        return Ok(());
    }
    Err(ProjectionError::ContractValidation {
        message: format!(
            "remote interface box {dependency_ref}/{public_instance_key} references operationAbiId {}, but dependency publication does not export it",
            operation.operation_abi_id
        ),
    })
}
