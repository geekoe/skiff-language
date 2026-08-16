use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    derive_package_schema_type_id, BoundaryCallbackContract, BoundaryDropPlan,
    BoundaryEffectGuarantee, BoundaryErrorAdmission, BoundaryErrorFallbackIdentity,
    BoundaryErrorPlan, BoundaryErrorPolicy, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryTransfer,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueFact, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, CallableEffectSummary, CallableMayEffects,
    ContractDiagnosticText, ContractOperationId, ContractPublicInstance,
    ContractPublicInstanceInterface, ContractPublicInstanceMethod, ContractRequirement,
    ContractTypeDescriptor, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, InterfaceInstantiationRef, PackageArtifact,
    PackageArtifactRef, PackageBinding, PackageCallableId, PackageSchemaCanonicalDescriptor,
    PackageSchemaTypeRecord, PendingEffectCategory, ServiceBoundaryPlan, ServiceCallRef,
    ServiceCallbackPlan, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceProtocolIdentity, ServiceRequirement, ServiceRequirementKey, ServiceSelectorBinding,
    ValueProvenance, SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
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

pub(super) fn remote_contract(
    service_id: &str,
    stable_key: &str,
) -> (
    Arc<ServiceContract>,
    ServiceContractRef,
    ContractOperationId,
) {
    let (contract, _, operation) = contract(service_id, stable_key, true);
    let mut contract = contract.as_ref().clone();
    let descriptor = contract.operations.get_mut(&operation).unwrap();
    descriptor.contract.return_value = BoundaryReturn {
        ty: ContractTypeRef::builtin("string"),
        value_plan: value_plan(BoundaryValueOwner::Provider),
    };
    let interface = InterfaceInstantiationRef {
        interface_abi_id: super::artifact::interface_identity(),
        canonical_type_args: Vec::new(),
    };
    contract.public_instances.insert(
        "reader".to_string(),
        ContractPublicInstance {
            interfaces: vec![ContractPublicInstanceInterface {
                interface: interface.clone(),
                methods: vec![ContractPublicInstanceMethod {
                    method_abi_id: skiff_artifact_identity::canonical_interface_method_abi_id(
                        &interface, "read",
                    ),
                    contract_operation_id: operation.clone(),
                }],
            }],
        },
    );
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
    package_bindings: Vec<PackageBinding>,
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
        package_bindings,
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

pub(super) fn service_boundary_plan() -> ServiceBoundaryPlan {
    ServiceBoundaryPlan {
        arguments: Vec::new(),
        results: Vec::new(),
        error: BoundaryErrorPlan {
            fallback_contract_type: std_service_internal_error(),
            fallback: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Call,
            },
            policy: BoundaryErrorPolicy::DynamicPublicSchema {
                admission: BoundaryErrorAdmission::PublicNameableSchemaClosed,
                fallback_identity: BoundaryErrorFallbackIdentity::StdServiceInternalError,
            },
            transfer: BoundaryTransfer::Move,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::Fresh,
        },
        stream_item: None,
        callbacks: ServiceCallbackPlan::None,
        effects: CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::ServiceCall],
                inout_path_effects: Vec::new(),
            },
        },
    }
}

pub(super) fn remote_service_boundary_plan() -> ServiceBoundaryPlan {
    ServiceBoundaryPlan {
        arguments: vec![BoundaryValueFact {
            contract_type: ContractTypeRef::builtin("string"),
            value_plan: value_plan(BoundaryValueOwner::Caller),
            transfer: BoundaryTransfer::Copy,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::CallerParameter { index: 0 },
        }],
        results: vec![BoundaryValueFact {
            contract_type: ContractTypeRef::builtin("string"),
            value_plan: value_plan(BoundaryValueOwner::Provider),
            transfer: BoundaryTransfer::Move,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::Fresh,
        }],
        error: BoundaryErrorPlan {
            fallback_contract_type: std_service_internal_error(),
            fallback: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Call,
            },
            policy: BoundaryErrorPolicy::DynamicPublicSchema {
                admission: BoundaryErrorAdmission::PublicNameableSchemaClosed,
                fallback_identity: BoundaryErrorFallbackIdentity::StdServiceInternalError,
            },
            transfer: BoundaryTransfer::Move,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::Fresh,
        },
        stream_item: None,
        callbacks: ServiceCallbackPlan::None,
        effects: CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::ServiceCall],
                inout_path_effects: Vec::new(),
            },
        },
    }
}

pub(super) fn std_service_internal_error() -> ContractTypeRef {
    let record = std_service_internal_error_record();
    ContractTypeRef::package_schema(
        record.package_id,
        record.stable_schema_key,
        record.package_schema_type_id,
    )
}

pub(super) fn std_service_internal_error_record() -> PackageSchemaTypeRecord {
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("message".to_string(), ContractTypeRef::builtin("string")),
                ("traceId".to_string(), ContractTypeRef::builtin("string")),
                ("errorId".to_string(), ContractTypeRef::builtin("string")),
            ]),
        },
    };
    let type_id =
        derive_package_schema_type_id("skiff.run/std", "std.service.InternalError", &descriptor)
            .expect("canonical std.service.InternalError schema derives");
    PackageSchemaTypeRecord {
        package_id: "skiff.run/std".to_string(),
        stable_schema_key: "std.service.InternalError".to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: descriptor,
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
