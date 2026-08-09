use skiff_artifact_model::{DeploymentGatewayEntry, PackageActorImplementation, PackageCallableId};
use skiff_runtime_linked_bytecode::{
    CandidateTable, FunctionIndex, LinkedBytecodeCandidate, LinkedGatewayCallableRole,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::{VerificationError, VerificationLocation};

use super::{semantic_violation, table_location, TargetCoverage};

pub(super) fn prove_entry_and_target_tables(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    coverage: &TargetCoverage,
) -> Result<(), VerificationError> {
    prove_operation_entries(hydrated, candidate)?;
    prove_gateway_entries(hydrated, candidate)?;
    prove_service_operations(hydrated, candidate)?;
    prove_actor_targets(hydrated, candidate)?;
    prove_referenced_table_coverage(candidate, coverage)
}

fn prove_operation_entries(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let bindings = &hydrated.deployment().operation_bindings;
    if candidate.operation_entries().len() != bindings.len() {
        return Err(semantic_violation(
            VerificationLocation::Image,
            "operation entry coverage differs from the exact deployment",
        ));
    }
    let implementation = implementation_package(hydrated)?;
    for binding in bindings {
        let entry = candidate
            .operation_entries()
            .iter()
            .find(|entry| entry.contract_operation_id() == &binding.contract_operation_id)
            .ok_or_else(|| {
                semantic_violation(
                    VerificationLocation::Image,
                    format!(
                        "deployment operation {} has no linked entry",
                        binding.contract_operation_id
                    ),
                )
            })?;
        prove_callable_function(
            candidate,
            implementation,
            entry.function(),
            &binding.package_callable_id,
            VerificationLocation::Image,
        )?;
    }
    Ok(())
}

fn prove_gateway_entries(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let deployment_entries = &hydrated.deployment().gateway_entries;
    if candidate.gateway_entries().len() != deployment_entries.len() {
        return Err(semantic_violation(
            VerificationLocation::Image,
            "gateway entry coverage differs from the exact deployment",
        ));
    }
    let implementation = implementation_package(hydrated)?;
    for (row, linked) in candidate.gateway_entries().iter().enumerate() {
        let row = u32::try_from(row).map_err(|_| {
            semantic_violation(VerificationLocation::Image, "gateway row does not fit u32")
        })?;
        let location = table_location(CandidateTable::GatewayEntries, row);
        let source = deployment_entries
            .get(linked.gateway_entry_key())
            .ok_or_else(|| semantic_violation(location, "gateway key is not deployed"))?;
        let exact = linked.gateway_entry_identity() == &source.gateway_entry_identity
            && linked.protocol_surface() == &source.protocol_surface
            && linked.adapter_plan() == &source.adapter_plan
            && linked.close_adapter_plan() == source.close_adapter_plan.as_ref();
        if !exact {
            return Err(semantic_violation(
                location,
                "linked gateway identity, protocol surface, or adapter plan differs from the exact deployment",
            ));
        }
        prove_gateway_roles(candidate, implementation, linked, source, location)?;
    }
    Ok(())
}

fn prove_gateway_roles(
    candidate: &LinkedBytecodeCandidate,
    implementation: &HydratedBytecodePackage,
    linked: &skiff_runtime_linked_bytecode::LinkedGatewayEntry,
    source: &DeploymentGatewayEntry,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected = [
        (LinkedGatewayCallableRole::Handler, source.handler.as_ref()),
        (LinkedGatewayCallableRole::Pre, source.pre.as_ref()),
        (LinkedGatewayCallableRole::Guard, source.guard.as_ref()),
        (
            LinkedGatewayCallableRole::CloseHandler,
            source.close_handler.as_ref(),
        ),
    ];
    let expected_count = expected
        .iter()
        .filter(|(_, callable)| callable.is_some())
        .count();
    if linked.callables().len() != expected_count {
        return Err(semantic_violation(
            location,
            "linked gateway callable-role coverage differs from the exact deployment",
        ));
    }
    for (role, expected_callable) in expected {
        let linked_callable = linked.callable(role);
        match (linked_callable, expected_callable) {
            (None, None) => {}
            (Some(linked_callable), Some(expected_callable)) => {
                if linked_callable.package_callable_id() != expected_callable {
                    return Err(semantic_violation(
                        location,
                        "linked gateway callable id differs from the exact deployment role",
                    ));
                }
                prove_callable_function(
                    candidate,
                    implementation,
                    linked_callable.function(),
                    expected_callable,
                    location,
                )?;
            }
            _ => {
                return Err(semantic_violation(
                    location,
                    "linked gateway callable role presence differs from the exact deployment",
                ));
            }
        }
    }
    Ok(())
}

fn prove_service_operations(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let expected_count = hydrated
        .service_dependencies()
        .values()
        .try_fold(0_usize, |count, dependency| {
            count.checked_add(dependency.used_operations().len())
        })
        .ok_or_else(|| {
            semantic_violation(
                VerificationLocation::Image,
                "service operation count overflowed usize",
            )
        })?;
    if candidate.service_operations().len() != expected_count {
        return Err(semantic_violation(
            VerificationLocation::Image,
            "service-operation target coverage differs from the hydrated dependency manifests",
        ));
    }
    for target in candidate.service_operations() {
        let location = table_location(CandidateTable::ServiceOperations, target.index().get());
        let dependency = hydrated
            .service_dependencies()
            .get(target.service_requirement_key())
            .ok_or_else(|| semantic_violation(location, "service dependency is not hydrated"))?;
        let contract = hydrated
            .contract_store()
            .get(dependency.contract())
            .ok_or_else(|| semantic_violation(location, "service contract is not hydrated"))?;
        let exact = dependency
            .used_operations()
            .contains(target.contract_operation_id())
            && contract
                .operations
                .contains_key(target.contract_operation_id())
            && target.expected_protocol_identity() == &contract.service_protocol_identity;
        if !exact {
            return Err(semantic_violation(
                location,
                "linked service target differs from its exact dependency contract facts",
            ));
        }
    }
    Ok(())
}

fn prove_actor_targets(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let expected_creates = hydrated
        .packages()
        .values()
        .flat_map(|package| &package.artifact().actor_implementations)
        .filter(|actor| actor.create.is_some())
        .count();
    let expected_methods = hydrated
        .packages()
        .values()
        .flat_map(|package| &package.artifact().actor_implementations)
        .try_fold(0_usize, |count, actor| {
            count.checked_add(actor.methods.len())
        })
        .ok_or_else(|| {
            semantic_violation(
                VerificationLocation::Image,
                "actor method count overflowed usize",
            )
        })?;
    if candidate.actor_creates().len() != expected_creates
        || candidate.actor_methods().len() != expected_methods
    {
        return Err(semantic_violation(
            VerificationLocation::Image,
            "actor create/method target coverage differs from hydrated package manifests",
        ));
    }
    for package in hydrated.packages().values() {
        for actor in &package.artifact().actor_implementations {
            prove_actor_row(package, actor, candidate)?;
        }
    }
    Ok(())
}

fn prove_actor_row(
    package: &HydratedBytecodePackage,
    actor: &PackageActorImplementation,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let actor_abi = package
        .artifact()
        .implementation_links
        .types
        .values()
        .find(|export| {
            export.file.module_path == actor.actor.module_path
                && export.symbol == actor.actor.symbol
        })
        .and_then(|export| export.actor.as_ref())
        .ok_or_else(|| {
            semantic_violation(
                VerificationLocation::Image,
                format!(
                    "actor {} has no package ABI authority",
                    actor.actor.symbol_path()
                ),
            )
        })?;
    for (method, callable) in &actor.methods {
        let target = candidate.actor_methods().iter().find(|target| {
            target.owner_package_build_id() == &package.reference().package_build_id
                && target.actor() == &actor.actor
                && target.actor_implementation_identity() == &actor.actor_implementation_identity
                && target.method_identity() == method
        });
        let Some(target) = target else {
            return Err(semantic_violation(
                VerificationLocation::Image,
                format!("actor method {method:?} has no exact linked target"),
            ));
        };
        if target.actor_abi_identity() != &actor_abi.actor_abi_identity {
            return Err(semantic_violation(
                table_location(CandidateTable::ActorMethods, target.index().get()),
                "linked actor ABI identity differs from its package authority",
            ));
        }
        prove_callable_function(
            candidate,
            package,
            target.function(),
            callable,
            table_location(CandidateTable::ActorMethods, target.index().get()),
        )?;
    }
    prove_actor_create(package, actor, actor_abi, candidate)
}

fn prove_actor_create(
    package: &HydratedBytecodePackage,
    actor: &PackageActorImplementation,
    actor_abi: &skiff_artifact_model::PackageActorAbi,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let linked = candidate.actor_creates().iter().find(|target| {
        target.owner_package_build_id() == &package.reference().package_build_id
            && target.actor() == &actor.actor
            && target.actor_implementation_identity() == &actor.actor_implementation_identity
    });
    match (&actor.create, linked) {
        (None, None) => Ok(()),
        (Some(source), Some(linked)) => {
            let location = table_location(CandidateTable::ActorCreates, linked.index().get());
            if linked.create_identity() != &source.method_identity
                || linked.actor_abi_identity() != &actor_abi.actor_abi_identity
            {
                return Err(semantic_violation(
                    location,
                    "linked actor create identity differs from its package authority",
                ));
            }
            prove_callable_function(
                candidate,
                package,
                linked.function(),
                &source.package_callable_id,
                location,
            )
        }
        _ => Err(semantic_violation(
            VerificationLocation::Image,
            "actor create target presence differs from its package manifest",
        )),
    }
}

fn prove_callable_function(
    candidate: &LinkedBytecodeCandidate,
    package: &HydratedBytecodePackage,
    function: FunctionIndex,
    callable: &PackageCallableId,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let function = candidate
        .functions()
        .get(function.get() as usize)
        .ok_or_else(|| semantic_violation(location, "entry function is out of bounds"))?;
    let artifact_function_key = package.function_key_for_callable(callable);
    let canonical = artifact_function_key.and_then(|function_key| {
        package.canonical_implementation_callable_for_function_key(function_key)
    });
    let exact = function.key().package_build_id() == &package.reference().package_build_id
        && artifact_function_key == Some(function.key().artifact_function_key().as_str())
        && function.key().template_function_key() == callable
        && canonical.and_then(|canonical| {
            package.function_key_for_canonical_implementation_callable(canonical)
        }) == artifact_function_key;
    if !exact {
        return Err(semantic_violation(
            location,
            "entry function does not match its exact package callable manifest",
        ));
    }
    Ok(())
}

fn implementation_package(
    hydrated: &HydratedDeploymentBytecode,
) -> Result<&HydratedBytecodePackage, VerificationError> {
    let implementation = &hydrated.deployment().implementation;
    hydrated
        .packages()
        .get(&implementation.package_build_id)
        .filter(|package| package.reference() == implementation)
        .ok_or_else(|| {
            semantic_violation(
                VerificationLocation::Image,
                "exact implementation package is absent from hydration",
            )
        })
}

fn prove_referenced_table_coverage(
    candidate: &LinkedBytecodeCandidate,
    coverage: &TargetCoverage,
) -> Result<(), VerificationError> {
    let tables = [
        (
            CandidateTable::InterfaceTables,
            candidate.interface_tables().len(),
            coverage.interface_tables.len(),
        ),
        (
            CandidateTable::SyntheticCallbacks,
            candidate.synthetic_callbacks().len(),
            coverage.synthetic_callbacks.len(),
        ),
        (
            CandidateTable::CallbackCaptureLayouts,
            candidate.callback_capture_layouts().len(),
            coverage.callback_capture_layouts.len(),
        ),
        (
            CandidateTable::HostEffectAdapters,
            candidate.host_effect_adapters().len(),
            coverage.host_effect_adapters.len(),
        ),
        (
            CandidateTable::Intrinsics,
            candidate.intrinsics().len(),
            coverage.intrinsics.len(),
        ),
    ];
    for (table, rows, referenced) in tables {
        if rows != referenced {
            return Err(semantic_violation(
                VerificationLocation::Image,
                format!(
                    "{} table has {rows} rows but exact typed operands cover {referenced}",
                    table.name()
                ),
            ));
        }
    }
    Ok(())
}
