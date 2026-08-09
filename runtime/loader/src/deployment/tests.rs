use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_identity::{gateway_entry_identity, service_deployment_ref};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractOperationId, ContractRequirement, ContractTypeRef,
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentRevision, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayHttpProtocolSurface, IngressProtocol, IngressSelector,
    PackageArtifact, PackageArtifactRef, PackageBuildId, PackageCallableId,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageRequirementKey,
    PackageRuntimeRequirements, PackageSchemaIndexRef, ServiceCallRef, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity,
    ServiceRequirement, ServiceRequirementKey, ServiceSelectorBinding,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use super::{
    compose_dependency_closure_assembly, compose_deployment_assembly,
    DeploymentReleasePointerResolver,
};
use crate::RuntimeAssemblyContentResolver;

fn package_ref(package_id: &str) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!("build:{package_id}")),
        package_local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
    }
}

fn http_gateway_entry() -> (GatewayEntryKey, DeploymentGatewayEntry) {
    let surface = GatewayEntryProtocolSurface {
        protocol: skiff_artifact_model::GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::RawHttp,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpRequest],
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity = gateway_entry_identity(&surface).unwrap();
    (
        GatewayEntryKey::parse("fixture-http").unwrap(),
        DeploymentGatewayEntry {
            gateway_entry_identity: identity,
            protocol_surface: surface,
            handler: Some(PackageCallableId::new("pkg-callable:example:health")),
            close_handler: None,
            close_adapter_plan: None,
            pre: None,
            guard: None,
            adapter_plan: GatewayAdapterPlan {
                kind: GatewayAdapterKind::RawHttp,
                args: vec![skiff_artifact_model::GatewayAdapterArg {
                    param: "request".to_string(),
                    source: GatewayAdapterSource::HttpRequest,
                }],
            },
        },
    )
}

fn deployment() -> ServiceDeployment {
    let contract = ServiceContractRef {
        service_id: "example.health".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
    };
    let (entry_key, entry) = http_gateway_entry();
    ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref("example.health-provider"),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::from([(entry_key.clone(), entry)]),
        ingress: vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("GET".to_string()),
                path: "/health".to_string(),
            },
            gateway_entry_key: entry_key,
        }],
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Health deployment".to_string(),
            notes: BTreeMap::new(),
        },
    }
}

#[test]
fn compose_succeeds_for_self_contained_deployment() {
    let mut deployment = deployment();
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let assembly = compose_deployment_assembly(&reference, &deployment).unwrap();
    assert_eq!(assembly.roots, vec![reference.clone()]);
    assert_eq!(assembly.resolved_deployments, vec![reference.clone()]);
    assert_eq!(assembly.resolved_contracts, vec![deployment.contract]);
    assert_eq!(
        assembly.activation_templates,
        vec![skiff_artifact_model::ActivationTemplate {
            deployment: reference.clone(),
            implementation_package_build_id: deployment.implementation.package_build_id.clone(),
        }]
    );
    assert_eq!(assembly.gateway_ingress.len(), 1);
    assert_eq!(assembly.gateway_ingress[0].deployment, reference);
    assert_eq!(
        assembly.gateway_ingress[0].gateway_entry_identity,
        deployment
            .gateway_entries
            .values()
            .next()
            .unwrap()
            .gateway_entry_identity
    );
    assert_eq!(assembly.service_binding_templates[0].bindings, Vec::new());
    assert_eq!(assembly.package_link_plan.code_slots.len(), 1);
    skiff_artifact_identity::validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn compose_closes_package_bindings() {
    let mut deployment = deployment();
    deployment.package_bindings = vec![skiff_artifact_model::PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: deployment.implementation.package_build_id.clone(),
            package_requirement_alias: "lib".to_string(),
        },
        package: package_ref("example.lib"),
    }];
    deployment.gateway_entries.clear();
    deployment.ingress.clear();
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let assembly = compose_deployment_assembly(&reference, &deployment).unwrap();
    let builds = assembly
        .package_link_plan
        .code_slots
        .iter()
        .map(|slot| slot.package.package_build_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(builds.len(), 2);
    assert!(builds.contains(&"build:example.health-provider"));
    assert!(builds.contains(&"build:example.lib"));
    assert_eq!(assembly.package_link_plan.package_links.len(), 1);
}

#[test]
fn compose_deployment_assembly_keeps_selectors_as_single_deployment() {
    let mut deployment = deployment();
    deployment.service_selectors = vec![ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: deployment.implementation.package_build_id.clone(),
            service_requirement_slot: 0,
        },
        contract: deployment.contract.clone(),
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let assembly = compose_deployment_assembly(&reference, &deployment).unwrap();
    assert_eq!(assembly.resolved_deployments, vec![reference]);
    skiff_artifact_identity::validate_runtime_assembly_identity(&assembly).unwrap();
}

struct ClosureTestResolver {
    deployments: BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    packages: BTreeMap<PackageArtifactRef, Arc<PackageArtifact>>,
    pointers: BTreeMap<(String, String), ServiceDeploymentRef>,
}

impl RuntimeAssemblyContentResolver for ClosureTestResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployments
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing deployment {reference:?}"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing contract {reference:?}"))
    }

    fn resolve_package_schema_type(
        &self,
        reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        anyhow::bail!("test resolver has no schema records {reference:?}")
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package {reference:?}"))
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &skiff_artifact_model::FileIrRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::FileIrUnit>> {
        anyhow::bail!("test resolver has no File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &skiff_artifact_model::PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("test resolver has no static resources")
    }
}

impl DeploymentReleasePointerResolver for ClosureTestResolver {
    fn resolve_release_pointer(
        &self,
        _profile: &str,
        service_id: &str,
        version: &str,
    ) -> anyhow::Result<Option<ServiceDeploymentRef>> {
        Ok(self
            .pointers
            .get(&(service_id.to_string(), version.to_string()))
            .cloned())
    }
}

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn operation_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("bool"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
        },
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
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

fn bare_contract(service_id: &str) -> (ServiceContract, ContractOperationId) {
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, "op").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: "op".to_string(),
                contract: operation_contract(),
            },
        )]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (contract, operation_id)
}

fn bare_package(
    package_id: &str,
    service_dependency: Option<ServiceRequirement>,
    service_call_refs: Vec<ServiceCallRef>,
) -> PackageArtifact {
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!("build:{package_id}")),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: service_dependency
            .as_ref()
            .map(|requirement| requirement.contract_requirement.clone())
            .into_iter()
            .collect(),
        service_requirements: service_dependency.into_iter().collect(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs,
        bytecode: None,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    package
}

fn package_ref_of(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn bare_deployment(
    contract: ServiceContractRef,
    revision: &str,
    implementation: PackageArtifactRef,
    service_selectors: Vec<ServiceSelectorBinding>,
) -> ServiceDeployment {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation,
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors,
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: revision.to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    deployment
}

/// Consumer -> provider dependency chain under the same profile.
fn dependency_chain_fixture() -> (
    ClosureTestResolver,
    ServiceDeploymentRef,
    ServiceDeploymentRef,
) {
    let (provider_contract, provider_operation_id) = bare_contract("example.health");
    let provider_contract_ref = contract_ref(&provider_contract);
    let (consumer_contract, _) = bare_contract("example.consumer");
    let consumer_contract_ref = contract_ref(&consumer_contract);
    let provider_package = bare_package("example.health-provider", None, Vec::new());
    let provider_package_ref = package_ref_of(&provider_package);
    let provider = bare_deployment(
        provider_contract_ref.clone(),
        "provider-revision",
        provider_package_ref.clone(),
        Vec::new(),
    );
    let provider_ref = service_deployment_ref(&provider);

    let consumer_requirement = ServiceRequirement {
        contract_requirement: ContractRequirement {
            alias: "health".to_string(),
            service_id: provider_contract_ref.service_id.clone(),
            contract_version: provider_contract_ref.contract_version.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        },
        service_binding_slot: 0,
        used_operations: BTreeSet::from([provider_operation_id.clone()]),
    };
    let consumer_call = ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: provider_operation_id,
        expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
    };
    let consumer_package = bare_package(
        "example.consumer",
        Some(consumer_requirement),
        vec![consumer_call],
    );
    let consumer_package_ref = package_ref_of(&consumer_package);
    let consumer = bare_deployment(
        consumer_contract_ref.clone(),
        "consumer-revision",
        consumer_package_ref.clone(),
        vec![ServiceSelectorBinding {
            key: ServiceRequirementKey {
                caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                service_requirement_slot: 0,
            },
            contract: provider_contract_ref.clone(),
        }],
    );
    let consumer_ref = service_deployment_ref(&consumer);

    let resolver = ClosureTestResolver {
        deployments: BTreeMap::from([
            (consumer_ref.clone(), Arc::new(consumer)),
            (provider_ref.clone(), Arc::new(provider)),
        ]),
        contracts: BTreeMap::from([
            (provider_contract_ref.clone(), Arc::new(provider_contract)),
            (consumer_contract_ref.clone(), Arc::new(consumer_contract)),
        ]),
        packages: BTreeMap::from([
            (provider_package_ref, Arc::new(provider_package)),
            (consumer_package_ref, Arc::new(consumer_package)),
        ]),
        pointers: BTreeMap::from([(
            (
                provider_contract_ref.service_id.clone(),
                provider_contract_ref.contract_version.clone(),
            ),
            provider_ref.clone(),
        )]),
    };
    (resolver, consumer_ref, provider_ref)
}

/// Consumer -> provider with the provider selector pointing back at the
/// consumer contract, forming a two-deployment dependency cycle.
fn cycle_fixture() -> (ClosureTestResolver, ServiceDeploymentRef) {
    let (provider_contract, _) = bare_contract("example.health");
    let provider_contract_ref = contract_ref(&provider_contract);
    let (consumer_contract, _) = bare_contract("example.consumer");
    let consumer_contract_ref = contract_ref(&consumer_contract);
    let provider_package = bare_package("example.health-provider", None, Vec::new());
    let provider_package_ref = package_ref_of(&provider_package);
    let mut back = bare_deployment(
        provider_contract_ref.clone(),
        "provider-back-revision",
        provider_package_ref.clone(),
        Vec::new(),
    );
    back.service_selectors = vec![ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: provider_package_ref.package_build_id.clone(),
            service_requirement_slot: 0,
        },
        contract: consumer_contract_ref.clone(),
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut back).unwrap();
    let back_ref = service_deployment_ref(&back);

    let consumer_package = bare_package("example.consumer", None, Vec::new());
    let consumer_package_ref = package_ref_of(&consumer_package);
    let consumer = bare_deployment(
        consumer_contract_ref.clone(),
        "consumer-revision",
        consumer_package_ref.clone(),
        vec![ServiceSelectorBinding {
            key: ServiceRequirementKey {
                caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                service_requirement_slot: 0,
            },
            contract: provider_contract_ref.clone(),
        }],
    );
    let consumer_ref = service_deployment_ref(&consumer);

    let resolver = ClosureTestResolver {
        deployments: BTreeMap::from([
            (consumer_ref.clone(), Arc::new(consumer)),
            (back_ref.clone(), Arc::new(back)),
        ]),
        contracts: BTreeMap::from([
            (provider_contract_ref.clone(), Arc::new(provider_contract)),
            (consumer_contract_ref.clone(), Arc::new(consumer_contract)),
        ]),
        packages: BTreeMap::from([
            (provider_package_ref, Arc::new(provider_package)),
            (consumer_package_ref, Arc::new(consumer_package)),
        ]),
        pointers: BTreeMap::from([
            (
                (
                    provider_contract_ref.service_id.clone(),
                    provider_contract_ref.contract_version.clone(),
                ),
                back_ref.clone(),
            ),
            (
                (
                    consumer_contract_ref.service_id.clone(),
                    consumer_contract_ref.contract_version.clone(),
                ),
                consumer_ref.clone(),
            ),
        ]),
    };
    (resolver, consumer_ref)
}

#[test]
fn compose_dependency_closure_links_provider_into_one_assembly() {
    let (resolver, consumer_ref, provider_ref) = dependency_chain_fixture();
    let assembly = compose_dependency_closure_assembly(&consumer_ref, &resolver, "dev").unwrap();
    assert_eq!(
        assembly.roots,
        vec![consumer_ref.clone(), provider_ref.clone()]
    );
    assert_eq!(
        assembly.resolved_deployments,
        vec![consumer_ref.clone(), provider_ref.clone()]
    );
    assert_eq!(assembly.activation_templates.len(), 2);
    let template = assembly
        .service_binding_templates
        .iter()
        .find(|template| template.activation == consumer_ref)
        .expect("consumer binding template");
    assert_eq!(template.bindings.len(), 1);
    assert_eq!(template.bindings[0].provider, provider_ref);
    skiff_artifact_identity::validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn compose_dependency_closure_fails_without_provider_release_pointer() {
    let (mut resolver, consumer_ref, _) = dependency_chain_fixture();
    resolver.pointers.clear();
    let error = compose_dependency_closure_assembly(&consumer_ref, &resolver, "dev")
        .unwrap_err()
        .to_string();
    assert!(error.contains("no release pointer"), "{error}");
}

#[test]
fn compose_dependency_closure_rejects_cycles() {
    let (resolver, consumer_ref) = cycle_fixture();
    let error = compose_dependency_closure_assembly(&consumer_ref, &resolver, "dev")
        .unwrap_err()
        .to_string();
    assert!(error.contains("dependency cycle"), "{error}");
}

#[test]
fn compose_rejects_exact_ref_mismatch() {
    let deployment = deployment();
    let mut other = deployment.clone();
    other.deployment_revision = DeploymentRevision::new("revision-2");
    let reference = service_deployment_ref(&other);
    let error = compose_deployment_assembly(&reference, &deployment)
        .unwrap_err()
        .to_string();
    assert!(error.contains("deployment"), "{error}");
}
