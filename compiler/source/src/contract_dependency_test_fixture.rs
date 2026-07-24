use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, contract_type_id,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractRequirement, ContractSchemaType, ContractTypeDescriptor,
    ContractTypeNameability, ContractTypeRef, ContractTypeShape, ServiceContract,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};
use skiff_compiler_input::ResolvedContractDependency;

pub(crate) fn resolved_contract_fixture(
    alias: &str,
    service_id: &str,
    operation_key: &str,
    public_type_key: &str,
    closure_type_key: &str,
) -> ResolvedContractDependency {
    let contract = contract_fixture(
        service_id,
        "1.0.0",
        operation_key,
        public_type_key,
        closure_type_key,
    );
    ResolvedContractDependency::validated(requirement(alias, &contract), contract).unwrap()
}

pub(crate) fn requirement(alias: &str, contract: &ServiceContract) -> ContractRequirement {
    ContractRequirement {
        alias: alias.to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

pub(crate) fn contract_fixture(
    service_id: &str,
    version: &str,
    operation_key: &str,
    public_type_key: &str,
    closure_type_key: &str,
) -> ServiceContract {
    let public_type_id = contract_type_id(service_id, version, public_type_key).unwrap();
    let closure_type_id = contract_type_id(service_id, version, closure_type_key).unwrap();
    let operation_id = contract_operation_id(service_id, version, operation_key).unwrap();
    let operation = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: operation_key.to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "input".to_string(),
                ty: ContractTypeRef::contract(public_type_id.clone()),
                value_plan: linkable(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::contract(closure_type_id.clone()),
                value_plan: linkable(BoundaryValueOwner::Provider),
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
        },
    };
    let boundary_schema = BTreeMap::from([
        (
            public_type_id.clone(),
            ContractSchemaType {
                contract_type_id: public_type_id,
                stable_key: public_type_key.to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::PublicNameable,
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "value".to_string(),
                            ContractTypeRef::builtin("string"),
                        )]),
                    },
                },
            },
        ),
        (
            closure_type_id.clone(),
            ContractSchemaType {
                contract_type_id: closure_type_id,
                stable_key: closure_type_key.to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::ClosureOnly,
                    type_params: Vec::new(),
                    descriptor: ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "value".to_string(),
                            ContractTypeRef::builtin("string"),
                        )]),
                    },
                },
            },
        ),
    ]);
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, operation)]),
        boundary_schema,
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
