use crate::{
    callable_facts::ProjectionCallableFactsIndex, error::ProjectionError,
    typed_artifacts::public_function_operation_abi_id,
};
use skiff_artifact_model::{CanonicalPublicCallableSignature, FunctionTypeParamIr};
use skiff_compiler_projection_input::{EntryParamSpec, EntryTypeSpec, ProjectionView};

use super::entrypoints::{EntryOperationCallable, EntryPointArtifacts};

pub(super) fn canonicalize_service_entry_operation_abis(
    artifacts: &mut EntryPointArtifacts,
    input: ProjectionView<'_>,
) -> Result<(), ProjectionError> {
    let callable_facts_index =
        ProjectionCallableFactsIndex::new(input.file_ir_units(), input.source().callable_effects());
    for spec in &artifacts.service_operations {
        let source_symbol = spec.callable.display_symbol();
        let callable_facts = callable_facts_index.for_symbol(
            &spec.implementation_module,
            &source_symbol,
            &format!("entry operation {}", spec.operation),
        )?;
        if callable_facts.may_suspend() != spec.may_suspend {
            return Err(ProjectionError::ImplementationConformance {
                message: format!(
                    "entry operation {} projected maySuspend {} does not match callable owner maySuspend {}",
                    spec.operation,
                    spec.may_suspend,
                    callable_facts.may_suspend()
                ),
            });
        }
        let public_signature = match &spec.callable {
            EntryOperationCallable::ImplMethod { .. } => callable_facts.receiver_public_signature(),
            EntryOperationCallable::Function { .. } => callable_facts.public_signature(),
        };
        let operation_abi_id =
            entry_operation_abi_id_from_public_signature(&spec.operation, &public_signature);
        let mut matching_operations = artifacts
            .runtime_operations
            .iter_mut()
            .filter(|operation| operation.operation == spec.operation);
        let runtime_operation = matching_operations.next().ok_or_else(|| {
            ProjectionError::ImplementationConformance {
                message: format!(
                    "entry operation {} is missing from the runtime operation manifest",
                    spec.operation
                ),
            }
        })?;
        if matching_operations.next().is_some() {
            return Err(ProjectionError::ImplementationConformance {
                message: format!(
                    "entry operation {} has multiple runtime operation manifest entries",
                    spec.operation
                ),
            });
        }
        runtime_operation.operation_abi_id = operation_abi_id.clone();
        for route in artifacts
            .http_routes
            .iter_mut()
            .filter(|route| route.operation == spec.operation)
        {
            route.operation_abi_id = Some(operation_abi_id.clone());
        }
    }
    Ok(())
}

pub(super) fn entry_operation_abi_id(
    public_path: &str,
    params: &[EntryParamSpec],
    return_type: &EntryTypeSpec,
    may_suspend: bool,
) -> String {
    let public_signature = entry_operation_public_signature(params, return_type, may_suspend);
    entry_operation_abi_id_from_public_signature(public_path, &public_signature)
}

pub(super) fn entry_operation_abi_id_from_public_signature(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> String {
    public_function_operation_abi_id(public_path, public_signature, &[], &Default::default())
}

fn entry_operation_public_signature(
    params: &[EntryParamSpec],
    return_type: &EntryTypeSpec,
    may_suspend: bool,
) -> CanonicalPublicCallableSignature {
    CanonicalPublicCallableSignature {
        params: params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: param.ty.ir.clone(),
            })
            .collect(),
        return_type: return_type.ir.clone(),
        may_suspend,
    }
}
