use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    assign_package_artifact_identities, assign_service_contract_identities,
    assign_service_deployment_identity, contract_operation_id, package_schema_index_identity,
    service_deployment_ref,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractDiagnosticText, ContractRequirement, DeploymentArtifactIdentity,
    DeploymentDiagnosticText, DeploymentIngressBinding, DeploymentOperationBinding,
    DeploymentRevision, GatewayEntryKey, IngressProtocol, IngressSelector, PackageArtifact,
    PackageArtifactRef, PackageBinding, PackageBuildId, PackageCallableId,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageRequirement,
    PackageRequirementKey, PackageRuntimeRequirements, PackageSchemaIndexRef, ServiceCallRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceProtocolIdentity, ServiceRequirement, ServiceRequirementKey, ServiceSelectorBinding,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

pub fn contract(service_id: &str) -> ServiceContract {
    contract_with_stable_key(service_id, "call")
}

pub fn contract_with_stable_key(service_id: &str, stable_key: &str) -> ServiceContract {
    let version = "1.0.0";
    let operation_id = contract_operation_id(service_id, version, stable_key).unwrap();
    let descriptor = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: stable_key.to_string(),
        contract: BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: skiff_artifact_model::ContractTypeRef::builtin("bool"),
                value_plan: BoundaryValuePlan::Linkable {
                    carrier: BoundaryValueCarrier::DetachedValueGraph,
                    encoding: BoundaryValueEncoding::CanonicalValue,
                    owner: BoundaryValueOwner::Provider,
                    lifetime: BoundaryValueLifetime::Call,
                },
            },
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
        },
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, descriptor)]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}

pub fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

pub fn operation(contract: &ServiceContract) -> skiff_artifact_model::ContractOperationId {
    contract.operations.keys().next().unwrap().clone()
}

pub fn package(
    package_id: &str,
    package_dependencies: &[(&str, &PackageArtifact)],
    service_dependencies: &[(&str, &ServiceContract, u32)],
) -> PackageArtifact {
    let package_requirements = package_dependencies
        .iter()
        .map(|(alias, dependency)| PackageRequirement {
            alias: (*alias).to_string(),
            package_id: dependency.package_id.clone(),
            exact_version: dependency.package_version.clone(),
            expected_local_abi: dependency.package_local_abi.local_abi_identity.clone(),
            expected_package_build: None,
        })
        .collect();
    let contract_requirements = service_dependencies
        .iter()
        .map(|(alias, contract, _)| ContractRequirement {
            alias: (*alias).to_string(),
            service_id: contract.service_id.clone(),
            contract_version: contract.contract_version.clone(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        })
        .collect::<Vec<_>>();
    let service_requirements = service_dependencies
        .iter()
        .zip(&contract_requirements)
        .map(|((_, contract, slot), requirement)| ServiceRequirement {
            contract_requirement: requirement.clone(),
            service_binding_slot: *slot,
            used_operations: BTreeSet::from([operation(contract)]),
        })
        .collect::<Vec<_>>();
    let service_call_refs = service_dependencies
        .iter()
        .map(|(_, contract, slot)| ServiceCallRef {
            service_requirement_slot: *slot,
            contract_operation_id: operation(contract),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        })
        .collect();

    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements,
        contract_requirements,
        service_requirements,
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_roots: Vec::new(),
        service_call_refs,
    };
    assign_package_artifact_identities(&mut package).unwrap();
    package
}

pub fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

pub fn package_binding(
    caller: &PackageArtifact,
    alias: &str,
    provider: &PackageArtifact,
) -> PackageBinding {
    PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: caller.package_build_id.clone(),
            package_requirement_alias: alias.to_string(),
        },
        package: package_ref(provider),
    }
}

pub fn service_selector(
    caller: &PackageArtifact,
    slot: u32,
    contract: &ServiceContract,
) -> ServiceSelectorBinding {
    ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: caller.package_build_id.clone(),
            service_requirement_slot: slot,
        },
        contract: contract_ref(contract),
    }
}

pub fn deployment(
    contract: &ServiceContract,
    implementation: &PackageArtifact,
    revision: &str,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
) -> ServiceDeployment {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref(contract),
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref(implementation),
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation(contract),
            package_callable_id: PackageCallableId::new("callable.fixture"),
        }],
        package_bindings,
        service_selectors,
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: crate::fixtures::deployment_policy_fixture(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: contract.service_id.clone(),
            notes: BTreeMap::new(),
        },
    };
    assign_service_deployment_identity(&mut deployment).unwrap();
    deployment
}

pub fn deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    service_deployment_ref(deployment)
}

pub fn add_http_ingress(
    deployment: &mut ServiceDeployment,
    _contract: &ServiceContract,
    host: &str,
    path: &str,
) {
    let key = GatewayEntryKey::parse("fixture-http").unwrap();
    deployment.gateway_entries.insert(
        key.clone(),
        crate::fixtures::gateway_entry_fixture(PackageCallableId::new("callable.fixture")),
    );
    deployment.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            host: host.to_string(),
            method: Some("POST".to_string()),
            path: path.to_string(),
        },
        gateway_entry_key: key,
    });
    assign_service_deployment_identity(deployment).unwrap();
}
