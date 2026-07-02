use std::collections::BTreeMap;

use crate::context::{
    ProjectedServiceDependencyLockEntry, ProjectedServiceDependencyRemoteBoxProvenance,
};
use crate::error::ProjectionError;
use crate::typed_artifacts::ServiceDependencyConstraint;
use skiff_artifact_model::{
    BoxSourceIr, ExecutableBody, ExprIr, FileIrUnit, OperationAbiRef, PublicationOperationAbi,
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
    let mut lock = dependency_lock.to_vec();
    let lock_entry_by_alias = lock
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.alias().to_string(), index))
        .collect::<BTreeMap<_, _>>();

    for source in remote_box_sources(service_file_units) {
        let BoxSourceIr::Remote {
            dependency_ref,
            public_instance_key,
            operations,
            callee_protocol_identity,
        } = source
        else {
            continue;
        };
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
                    interface: operations.interface.clone(),
                    interface_display: operations.interface.interface_abi_id.clone(),
                    public_instance: public_instance_key.clone(),
                    method_abi_id: slot.method_abi_id.clone(),
                    operation,
                },
            );
        }
    }

    Ok(lock)
}

fn remote_box_sources(service_file_units: &[FileIrUnit]) -> Vec<&BoxSourceIr> {
    service_file_units
        .iter()
        .flat_map(|unit| {
            unit.constants
                .iter()
                .flat_map(|constant| remote_box_sources_in_body(&constant.body))
                .chain(
                    unit.executables
                        .iter()
                        .flat_map(|executable| remote_box_sources_in_body(&executable.body)),
                )
        })
        .collect()
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
