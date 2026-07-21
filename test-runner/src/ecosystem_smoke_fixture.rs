//! Canonical production-operation fixture used by the isolated ecosystem smoke.
//!
//! The package-test overlay remains a separate package and deployment. This
//! module adds a code-free production contract for one unary operation and one
//! server-stream operation, then resolves both deployments into one immutable
//! `RuntimeAssembly`.

use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_package_artifact_identities, package_artifact_ref, service_contract_ref,
    service_deployment_ref,
};
use skiff_artifact_model::{
    ActivationPolicy, BoundaryCallableProjection, BoundaryCallbackContract,
    BoundaryCancellationContract, BoundaryConfigRequirement, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryImplementationRequirements, BoundaryReturn, BoundaryStateKind,
    BoundaryStateRequirement, BoundaryStreamContract, BoundaryUnavailableReason,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, CallableMayEffects, CallableProvenanceSummary, ContractOperationId,
    ContractTypeRef, DeploymentDiagnosticText, DeploymentIngressBinding, DeploymentPolicy,
    DeploymentRevision, IngressProtocol, IngressSelector, PackageArtifactRef, PackageBinding,
    PackageCallableId, PackageCallableSignature, PackageLocalAbiSymbol, PackageRequirementKey,
    PackageTypeRef, ResourcePolicy, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentInput, ServiceDeploymentOperationInput,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_contract, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use crate::{
    canonical_fixture::{
        assemble_package_test_fixture, CanonicalFixtureError, CanonicalPackageTestEntrypoint,
        CanonicalTestRecords,
    },
    canonical_package::CanonicalPackageProject,
    test_overlay::PublishedPackageTestOverlay,
};

const SMOKE_SERVICE_ID: &str = "test.skiff/ecosystem-smoke";
const SMOKE_CONTRACT_VERSION: &str = "1.0.0";
const SMOKE_HOST: &str = "ecosystem-smoke.skiff.localhost";

/// Narrow test-only bridge for the compiler's frozen stream-projection gap.
///
/// The source compiler already emits the typed stream executable and proves its
/// semantic facts. At the Phase 05 checkpoint it deliberately reports exactly
/// `UnsupportedStream` for the public boundary projection. The ecosystem smoke
/// needs a real long-lived stream, so this fixture bridge accepts only that one
/// reason, only `Stream<string>`, and recomputes the canonical package identity.
/// Any additional unavailable reason remains a hard error.
pub fn enable_ecosystem_smoke_server_stream(
    project: &mut CanonicalPackageProject,
) -> Result<(), CanonicalFixtureError> {
    let (callable_id, signature) = smoke_stream_callable(project)?;
    let (effects, provenance) = validate_smoke_stream_checkpoint(project, &callable_id)?;
    let requirements = smoke_stream_requirements(project, effects, provenance);
    project.package.artifact.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Available {
            operation_contract: smoke_stream_operation_contract(&signature),
            implementation_requirements: requirements,
        },
    );
    assign_package_artifact_identities(&mut project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok(())
}

fn smoke_stream_callable(
    project: &CanonicalPackageProject,
) -> Result<(PackageCallableId, PackageCallableSignature), CanonicalFixtureError> {
    let symbol = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get("events")
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "smoke package omitted public callable events".to_string(),
            )
        })?;
    let PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    } = symbol
    else {
        return Err(CanonicalFixtureError::InvalidInput(
            "smoke public path events is not callable".to_string(),
        ));
    };
    let callable_id = callable_id.clone();
    let signature = signature.clone();
    let PackageTypeRef::Container { name, arguments } = &signature.return_type else {
        return Err(CanonicalFixtureError::InvalidInput(
            "smoke events must return Stream<string>".to_string(),
        ));
    };
    if name != "Stream"
        || arguments.as_slice()
            != [PackageTypeRef::Container {
                name: "string".to_string(),
                arguments: Vec::new(),
            }]
    {
        return Err(CanonicalFixtureError::InvalidInput(
            "smoke events must return exactly Stream<string>".to_string(),
        ));
    }
    Ok((callable_id, signature))
}

fn validate_smoke_stream_checkpoint(
    project: &CanonicalPackageProject,
    callable_id: &PackageCallableId,
) -> Result<(CallableMayEffects, CallableProvenanceSummary), CanonicalFixtureError> {
    match project
        .package
        .artifact
        .boundary_projections
        .get(callable_id)
    {
        Some(BoundaryCallableProjection::Unavailable { reasons })
            if reasons == &[BoundaryUnavailableReason::UnsupportedStream] => {}
        Some(projection) => {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "smoke stream bridge requires the exact UnsupportedStream checkpoint, found {projection:?}"
            )));
        }
        None => {
            return Err(CanonicalFixtureError::InvalidInput(
                "smoke events has no boundary projection".to_string(),
            ));
        }
    }
    let facts = project
        .package
        .artifact
        .callable_semantic_facts
        .get(callable_id)
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(
                "smoke events has no callable semantic facts".to_string(),
            )
        })?;
    let effects = facts.effects.effects_for_boundary().map_err(|reason| {
        CanonicalFixtureError::InvalidInput(format!(
            "smoke events effects are not analyzed: {reason:?}"
        ))
    })?;
    if effects.writes_caller_reachable
        || effects.returns_caller_alias
        || effects.throws_caller_alias
        || effects.escapes_caller_value
        || effects.requires_same_heap_identity
        || effects.invokes_unknown_target
    {
        return Err(CanonicalFixtureError::InvalidInput(
            "smoke events semantic facts are unsafe for a detached service boundary".to_string(),
        ));
    }
    Ok((*effects, facts.provenance.clone()))
}

fn smoke_stream_requirements(
    project: &CanonicalPackageProject,
    effects: CallableMayEffects,
    provenance: CallableProvenanceSummary,
) -> BoundaryImplementationRequirements {
    let runtime = &project.package.artifact.runtime_requirements;
    let mut config = runtime
        .config
        .iter()
        .map(|requirement| BoundaryConfigRequirement {
            path: requirement.path.clone(),
            value_type: requirement.value_type.clone(),
            required: requirement.required,
        })
        .collect::<Vec<_>>();
    config.sort_by(|left, right| left.path.cmp(&right.path));
    let mut state = runtime
        .resources
        .iter()
        .map(|requirement| BoundaryStateRequirement {
            key: requirement.key.clone(),
            kind: BoundaryStateKind::ExternalResource,
        })
        .collect::<Vec<_>>();
    state.sort_by(|left, right| left.key.cmp(&right.key));
    let mut runtime_capabilities = runtime
        .runtime_capabilities
        .iter()
        .map(|requirement| requirement.capability.clone())
        .collect::<Vec<_>>();
    runtime_capabilities.sort();
    runtime_capabilities.dedup();
    BoundaryImplementationRequirements {
        config,
        state,
        native_capabilities: Vec::new(),
        runtime_capabilities,
        complete_may_effects: effects,
        provenance,
    }
}

fn smoke_stream_operation_contract(
    signature: &PackageCallableSignature,
) -> skiff_artifact_model::BoundaryOperationContract {
    let call_plan = |owner, lifetime| BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    };
    skiff_artifact_model::BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: call_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call),
        },
        errors: BoundaryErrorContract::None,
        stream: BoundaryStreamContract::ServerStream {
            item_type: ContractTypeRef::builtin("string"),
            item_value_plan: call_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream),
        },
        cancellation: if signature.may_suspend {
            BoundaryCancellationContract::Cooperative
        } else {
            BoundaryCancellationContract::NotCancellable
        },
        callbacks: BoundaryCallbackContract::None,
        may_suspend: signature.may_suspend,
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

#[derive(Debug, Clone)]
pub struct EcosystemSmokeEntrypoint {
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub contract: skiff_artifact_model::ServiceContractRef,
    pub operation: skiff_artifact_model::ContractOperationId,
}

#[derive(Debug, Clone)]
pub struct CanonicalEcosystemSmokeFixture {
    pub production: skiff_artifact_model::PackageArtifactRef,
    pub overlay: skiff_artifact_model::PackageArtifactRef,
    pub records: CanonicalTestRecords,
    pub package_test: CanonicalPackageTestEntrypoint,
    pub unary: EcosystemSmokeEntrypoint,
    pub stream: EcosystemSmokeEntrypoint,
}

pub fn assemble_ecosystem_smoke_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
) -> Result<CanonicalEcosystemSmokeFixture, CanonicalFixtureError> {
    let test_fixture = assemble_package_test_fixture(project, overlay)?;
    if test_fixture.entrypoints.len() != 1 {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "ecosystem smoke requires exactly one package-test entrypoint, found {}",
            test_fixture.entrypoints.len()
        )));
    }

    let smoke_contract = compile_smoke_contract(project)?;
    let (unary_selector, stream_selector) = smoke_selectors();
    let production_ref = package_artifact_ref(&project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let package_bindings = smoke_package_bindings(project, &production_ref)?;
    let package_artifacts = project
        .packages()
        .map(|package| package.artifact.clone())
        .collect::<Vec<_>>();
    let deployment = project_smoke_deployment(
        &smoke_contract,
        production_ref,
        package_bindings,
        &unary_selector,
        &stream_selector,
        &package_artifacts,
    )?;
    let deployment_ref = service_deployment_ref(&deployment);

    let mut records = test_fixture.records;
    records.contracts.push(smoke_contract.contract);
    records.deployments.push(deployment);
    let roots = vec![
        test_fixture.entrypoints[0].deployment.clone(),
        deployment_ref.clone(),
    ];
    let all_package_artifacts = records
        .packages
        .iter()
        .map(|package| package.artifact.clone())
        .collect::<Vec<_>>();
    records.assembly = resolve_runtime_assembly(
        &roots,
        &records.deployments,
        &records.contracts,
        &all_package_artifacts,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;

    Ok(CanonicalEcosystemSmokeFixture {
        production: test_fixture.production,
        overlay: test_fixture.overlay,
        package_test: test_fixture.entrypoints.into_iter().next().unwrap(),
        records,
        unary: EcosystemSmokeEntrypoint {
            selector: unary_selector,
            deployment: deployment_ref.clone(),
            contract: smoke_contract.reference.clone(),
            operation: smoke_contract.unary_operation,
        },
        stream: EcosystemSmokeEntrypoint {
            selector: stream_selector,
            deployment: deployment_ref,
            contract: smoke_contract.reference,
            operation: smoke_contract.stream_operation,
        },
    })
}

struct SmokeContract {
    contract: ServiceContract,
    reference: ServiceContractRef,
    unary_operation: ContractOperationId,
    stream_operation: ContractOperationId,
}

fn compile_smoke_contract(
    project: &CanonicalPackageProject,
) -> Result<SmokeContract, CanonicalFixtureError> {
    let unary = production_operation_contract(project, "marker")?;
    if !matches!(unary.stream, BoundaryStreamContract::Unary) {
        return Err(CanonicalFixtureError::InvalidInput(
            "ecosystem smoke marker must be a unary boundary operation".to_string(),
        ));
    }
    let stream = production_operation_contract(project, "events")?;
    if !matches!(stream.stream, BoundaryStreamContract::ServerStream { .. }) {
        return Err(CanonicalFixtureError::InvalidInput(
            "ecosystem smoke events must be a server-stream boundary operation".to_string(),
        ));
    }
    let contract = compile_contract(ServiceContractDefinition {
        service_id: SMOKE_SERVICE_ID.to_string(),
        contract_version: SMOKE_CONTRACT_VERSION.to_string(),
        operations: BTreeMap::from([("unary".to_string(), unary), ("stream".to_string(), stream)]),
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "isolated package/service ecosystem smoke".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let reference = service_contract_ref(&contract)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let operation = |stable_key: &str| {
        contract
            .operations
            .values()
            .find(|candidate| candidate.stable_key == stable_key)
            .map(|candidate| candidate.operation_id.clone())
            .ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "compiled smoke contract omitted {stable_key}"
                ))
            })
    };
    Ok(SmokeContract {
        unary_operation: operation("unary")?,
        stream_operation: operation("stream")?,
        contract,
        reference,
    })
}

fn smoke_selectors() -> (IngressSelector, IngressSelector) {
    (
        IngressSelector {
            protocol: IngressProtocol::Http,
            host: SMOKE_HOST.to_string(),
            method: Some("POST".to_string()),
            path: "/probe".to_string(),
        },
        IngressSelector {
            protocol: IngressProtocol::Http,
            host: SMOKE_HOST.to_string(),
            method: Some("GET".to_string()),
            path: "/stream".to_string(),
        },
    )
}

fn smoke_package_bindings(
    project: &CanonicalPackageProject,
    production: &PackageArtifactRef,
) -> Result<Vec<PackageBinding>, CanonicalFixtureError> {
    project
        .package
        .artifact
        .package_requirements
        .iter()
        .map(|requirement| {
            let package = project
                .artifact(&requirement.package_id, &requirement.exact_version)
                .ok_or_else(|| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "smoke dependency {}@{} is absent",
                        requirement.package_id, requirement.exact_version
                    ))
                })?;
            let package = package_artifact_ref(&package.artifact)
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
            if package.package_local_abi_identity != requirement.expected_local_abi {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "smoke dependency {} ABI changed",
                    requirement.alias
                )));
            }
            Ok(PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: production.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package,
            })
        })
        .collect()
}

fn project_smoke_deployment(
    smoke: &SmokeContract,
    implementation: PackageArtifactRef,
    package_bindings: Vec<PackageBinding>,
    unary_selector: &IngressSelector,
    stream_selector: &IngressSelector,
    packages: &[skiff_artifact_model::PackageArtifact],
) -> Result<ServiceDeployment, CanonicalFixtureError> {
    let revision = implementation
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("package");
    let input = ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: smoke.reference.clone(),
        deployment_revision: DeploymentRevision::new(format!("smoke-{revision}")),
        implementation,
        operation_bindings: vec![
            ServiceDeploymentOperationInput {
                contract_operation_id: smoke.unary_operation.clone(),
                package_public_path: "marker".to_string(),
            },
            ServiceDeploymentOperationInput {
                contract_operation_id: smoke.stream_operation.clone(),
                package_public_path: "events".to_string(),
            },
        ],
        package_bindings,
        service_selectors: Vec::new(),
        ingress: vec![
            DeploymentIngressBinding {
                selector: unary_selector.clone(),
                contract_operation_id: smoke.unary_operation.clone(),
            },
            DeploymentIngressBinding {
                selector: stream_selector.clone(),
                contract_operation_id: smoke.stream_operation.clone(),
            },
        ],
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: 30_000,
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            activation: ActivationPolicy {
                max_concurrency: 4,
                idle_timeout_ms: None,
            },
            principal: "test:ecosystem-smoke".to_string(),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "isolated ecosystem smoke".to_string(),
            notes: BTreeMap::from([
                ("configOwner".to_string(), "smoke deployment".to_string()),
                ("stateOwner".to_string(), "smoke deployment".to_string()),
                ("doubleOwner".to_string(), "smoke request".to_string()),
            ]),
        },
    };
    project_service_deployment(input, &smoke.contract, packages)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn production_operation_contract(
    project: &CanonicalPackageProject,
    public_path: &str,
) -> Result<skiff_artifact_model::BoundaryOperationContract, CanonicalFixtureError> {
    let symbol = project
        .package
        .artifact
        .package_local_abi
        .public_symbols
        .get(public_path)
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "smoke package omitted public callable {public_path}"
            ))
        })?;
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "smoke public path {public_path} is not callable"
        )));
    };
    let projection = project
        .package
        .artifact
        .boundary_projections
        .get(callable_id)
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "smoke callable {public_path} has no boundary projection"
            ))
        })?;
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "smoke callable {public_path} is unavailable at the service boundary: {projection:?}"
        )));
    };
    Ok(operation_contract.clone())
}
