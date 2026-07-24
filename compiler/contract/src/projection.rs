use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, PackageArtifact, PackageCallableId,
    PackageLocalAbiSymbol, ServiceContract,
};

use crate::{
    compile_service_contract_definition, ContractDefinitionError, Result,
    ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};

/// Complete, machine-readable projection of one service package's public API.
///
/// `contract` contains every boundary-available public callable. `unavailable`
/// retains every package-only public callable and its canonical reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceApiProjection {
    pub contract: ServiceContract,
    pub available: BTreeMap<String, PackageCallableId>,
    pub unavailable: BTreeMap<String, Vec<BoundaryUnavailableReason>>,
}

/// Projects the code-free ServiceContract from the already compiled package.
///
/// Public paths come exclusively from `api.yml`'s PackageLocalAbi projection;
/// operation bodies come exclusively from the same callables' canonical
/// boundary projections. No independently authored operation list is accepted.
pub fn project_service_api(
    service_id: impl Into<String>,
    package: &PackageArtifact,
) -> Result<ServiceApiProjection> {
    let service_id = service_id.into();
    let public_callables = public_callable_paths(package)?;
    let mut available = BTreeMap::new();
    let mut unavailable = BTreeMap::new();
    let mut operations = BTreeMap::new();
    let mut operation_text = BTreeMap::new();

    for (callable_id, projection) in &package.boundary_projections {
        let public_path = public_callables.get(callable_id).ok_or_else(|| {
            ContractDefinitionError::MissingPublicCallable {
                callable_id: callable_id.to_string(),
            }
        })?;
        match projection {
            BoundaryCallableProjection::Available {
                operation_contract, ..
            } => {
                operations.insert(public_path.clone(), operation_contract.clone());
                operation_text.insert(public_path.clone(), public_path.clone());
                available.insert(public_path.clone(), callable_id.clone());
            }
            BoundaryCallableProjection::Unavailable { reasons } => {
                unavailable.insert(public_path.clone(), reasons.clone());
            }
        }
    }

    for callable_id in public_callables.keys() {
        if !package.boundary_projections.contains_key(callable_id) {
            return Err(ContractDefinitionError::MissingBoundaryProjection {
                callable_id: callable_id.to_string(),
            });
        }
    }

    let contract = compile_service_contract_definition(ServiceContractDefinition {
        service_id: service_id.clone(),
        contract_version: package.package_version.clone(),
        operations,
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: service_id,
            operations: operation_text,
            types: BTreeMap::new(),
        },
    })?;
    Ok(ServiceApiProjection {
        contract,
        available,
        unavailable,
    })
}

fn public_callable_paths(package: &PackageArtifact) -> Result<BTreeMap<PackageCallableId, String>> {
    let mut paths = BTreeMap::new();
    for (public_path, symbol) in &package.package_local_abi.public_symbols {
        let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
            continue;
        };
        if let Some(first) = paths.insert(callable_id.clone(), public_path.clone()) {
            return Err(ContractDefinitionError::DuplicatePublicCallable {
                callable_id: callable_id.to_string(),
                first,
                second: public_path.clone(),
            });
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
        BoundaryErrorContract, BoundaryImplementationRequirements, BoundaryOperationContract,
        BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
        BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, CallableMayEffects,
        CallableProvenanceSummary, PackageArtifact, PackageBuildId, PackageCallableId,
        PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi,
        PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRuntimeRequirements, PackageTypeRef,
        TypeRefIr, ValueProvenance,
    };

    use super::*;

    #[test]
    fn available_and_unavailable_public_functions_project_exactly() {
        let package = package_fixture("1.0.0");
        let projected = project_service_api("example.registry", &package).unwrap();

        assert_eq!(projected.available.keys().collect::<Vec<_>>(), vec!["read"]);
        assert_eq!(
            projected.unavailable,
            BTreeMap::from([(
                "mutate".to_string(),
                vec![BoundaryUnavailableReason::WritesCallerReachable],
            )])
        );
        assert_eq!(projected.contract.operations.len(), 1);
        assert_eq!(
            projected
                .contract
                .operations
                .values()
                .next()
                .unwrap()
                .stable_key,
            "read"
        );
        assert!(projected.contract.boundary_schema.is_empty());
    }

    #[test]
    fn identity_ignores_human_version_and_build_but_tracks_api() {
        let first = project_service_api("example.registry", &package_fixture("1.0.0")).unwrap();
        let mut rebuilt = package_fixture("9.7.3");
        rebuilt.package_build_id = PackageBuildId::new("different-build");
        let rebuilt = project_service_api("example.registry", &rebuilt).unwrap();
        assert_eq!(
            first.contract.service_protocol_identity,
            rebuilt.contract.service_protocol_identity
        );
        assert_eq!(
            first.contract.operations.keys().collect::<Vec<_>>(),
            rebuilt.contract.operations.keys().collect::<Vec<_>>()
        );

        let mut changed = package_fixture("1.0.0");
        let read = callable("read");
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = changed.boundary_projections.get_mut(&read).unwrap()
        else {
            unreachable!()
        };
        operation_contract.may_suspend = true;
        let changed = project_service_api("example.registry", &changed).unwrap();
        assert_ne!(
            first.contract.service_protocol_identity,
            changed.contract.service_protocol_identity
        );
    }

    #[test]
    fn missing_duplicate_and_unclosed_inputs_fail_closed() {
        let mut missing = package_fixture("1.0.0");
        missing.boundary_projections.remove(&callable("read"));
        assert!(matches!(
            project_service_api("example.registry", &missing),
            Err(ContractDefinitionError::MissingBoundaryProjection { .. })
        ));

        let mut duplicate = package_fixture("1.0.0");
        let symbol = duplicate
            .package_local_abi
            .public_symbols
            .get("read")
            .unwrap()
            .clone();
        duplicate
            .package_local_abi
            .public_symbols
            .insert("readAlias".to_string(), symbol);
        assert!(matches!(
            project_service_api("example.registry", &duplicate),
            Err(ContractDefinitionError::DuplicatePublicCallable { .. })
        ));

        let mut unclosed = package_fixture("1.0.0");
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = unclosed
            .boundary_projections
            .get_mut(&callable("read"))
            .unwrap()
        else {
            unreachable!()
        };
        operation_contract.return_value.ty = skiff_artifact_model::ContractTypeRef::contract(
            skiff_artifact_model::ContractTypeId::new("missing"),
        );
        assert!(project_service_api("example.registry", &unclosed).is_err());
    }

    fn package_fixture(version: &str) -> PackageArtifact {
        let read = callable("read");
        let mutate = callable("mutate");
        let signature = PackageCallableSignature {
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::native("string"),
            },
            throw_types: Vec::new(),
            may_suspend: false,
        };
        PackageArtifact {
            schema_version: skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.registry.impl".to_string(),
            package_version: version.to_string(),
            package_build_id: PackageBuildId::new("build"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("abi"),
                public_symbols: BTreeMap::from([
                    (
                        "mutate".to_string(),
                        PackageLocalAbiSymbol::Callable {
                            callable_id: mutate.clone(),
                            signature: signature.clone(),
                        },
                    ),
                    (
                        "read".to_string(),
                        PackageLocalAbiSymbol::Callable {
                            callable_id: read.clone(),
                            signature,
                        },
                    ),
                ]),
            },
            implementation_links: PackageImplementationLinks {
                types: BTreeMap::new(),
                constants: BTreeMap::new(),
                functions: BTreeMap::new(),
                impl_methods: BTreeMap::new(),
                operation_targets: BTreeMap::new(),
            },
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::from([
                (
                    mutate,
                    BoundaryCallableProjection::Unavailable {
                        reasons: vec![BoundaryUnavailableReason::WritesCallerReachable],
                    },
                ),
                (
                    read,
                    BoundaryCallableProjection::Available {
                        operation_contract: operation(),
                        implementation_requirements: implementation_requirements(),
                    },
                ),
            ]),
            service_call_refs: Vec::new(),
        }
    }

    fn callable(name: &str) -> PackageCallableId {
        PackageCallableId::new(format!("callable:{name}"))
    }

    fn operation() -> BoundaryOperationContract {
        BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: skiff_artifact_model::ContractTypeRef::builtin("string"),
                value_plan: value_plan(BoundaryValueOwner::Provider),
            },
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        }
    }

    fn implementation_requirements() -> BoundaryImplementationRequirements {
        BoundaryImplementationRequirements {
            config: Vec::new(),
            state: Vec::new(),
            native_capabilities: Vec::new(),
            runtime_capabilities: Vec::new(),
            complete_may_effects: CallableMayEffects {
                writes_caller_reachable: false,
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            },
            provenance: CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::Fresh],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
        }
    }

    fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime: BoundaryValueLifetime::Call,
        }
    }
}
