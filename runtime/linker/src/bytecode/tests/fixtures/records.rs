use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractDiagnosticText, ContractOperationId, ContractRequirement,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, PackageArtifact, PackageArtifactRef,
    PackageCallableId, ServiceCallRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceProtocolIdentity, ServiceRequirement, ServiceRequirementKey, ServiceSelectorBinding,
    SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use super::RootProgram;

pub(super) fn contract(
    service_id: &str,
    stable_key: &str,
    has_parameter: bool,
) -> (
    Arc<ServiceContract>,
    ServiceContractRef,
    ContractOperationId,
) {
    let operation =
        skiff_artifact_identity::contract_operation_id(service_id, "1.0.0", stable_key).unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation.clone(),
                stable_key: stable_key.to_string(),
                contract: operation_contract(has_parameter),
            },
        )]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::from([(operation.clone(), stable_key.to_string())]),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    let reference = skiff_artifact_identity::service_contract_ref(&contract).unwrap();
    (Arc::new(contract), reference, operation)
}

pub(super) fn deployment(
    implementation: PackageArtifactRef,
    contract: ServiceContractRef,
    operation: ContractOperationId,
    callable: PackageCallableId,
    service_selector: Option<ServiceSelectorBinding>,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new("revision:bytecode-link-fixture"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation,
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation,
            package_callable_id: callable,
        }],
        package_bindings: Vec::new(),
        service_selectors: service_selector.into_iter().collect(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "bytecode linker fixture".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_model::ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    };
    (Arc::new(deployment), reference)
}

pub(super) fn add_service_requirement(
    package: &mut PackageArtifact,
    contract: &ServiceContractRef,
    operation: &ContractOperationId,
) {
    let requirement = ContractRequirement {
        alias: "provider".to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    package.contract_requirements = vec![requirement.clone()];
    package.service_requirements = vec![ServiceRequirement {
        contract_requirement: requirement,
        service_binding_slot: 7,
        used_operations: std::collections::BTreeSet::from([operation.clone()]),
    }];
    package.service_call_refs = vec![ServiceCallRef {
        service_requirement_slot: 7,
        contract_operation_id: operation.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    }];
    skiff_artifact_identity::assign_package_artifact_identities(package).unwrap();
}

pub(super) fn service_selector(
    package: &PackageArtifactRef,
    contract: ServiceContractRef,
) -> ServiceSelectorBinding {
    ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: package.package_build_id.clone(),
            service_requirement_slot: 7,
        },
        contract,
    }
}

pub(super) fn package_reference(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

pub(super) fn operation_contract(has_parameter: bool) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: has_parameter
            .then(|| BoundaryParameter {
                name: "carrier".to_string(),
                ty: ContractTypeRef::builtin("string"),
                value_plan: value_plan(BoundaryValueOwner::Caller),
            })
            .into_iter()
            .collect(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: value_plan(BoundaryValueOwner::Provider),
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

fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

pub(super) const fn contract_has_parameter(program: RootProgram) -> bool {
    program.root_has_parameter()
}
