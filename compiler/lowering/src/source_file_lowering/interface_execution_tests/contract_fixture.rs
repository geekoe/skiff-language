use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, contract_type_id,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractDiagnosticText, ContractRequirement,
    ContractSchemaType, ContractTypeDescriptor, ContractTypeId, ContractTypeNameability,
    ContractTypeRef, ContractTypeShape, ServiceContract, ServiceProtocolIdentity,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};
use skiff_compiler_input::ResolvedContractDependency;

pub(super) fn contract_dependency() -> (ResolvedContractDependency, ContractTypeId) {
    let service_id = "example.payments";
    let version = "1.0.0";
    let user_type_id = contract_type_id(service_id, version, "User").unwrap();
    let ping_operation_id = contract_operation_id(service_id, version, "ping").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            ping_operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: ping_operation_id,
                stable_key: "ping".to_string(),
                contract: unary_string_operation(),
            },
        )]),
        boundary_schema: BTreeMap::from([(
            user_type_id.clone(),
            ContractSchemaType {
                contract_type_id: user_type_id.clone(),
                stable_key: "User".to_string(),
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
        )]),
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    let requirement = ContractRequirement {
        alias: "payments".to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    (
        ResolvedContractDependency::validated(requirement, contract).unwrap(),
        user_type_id,
    )
}

fn unary_string_operation() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("string"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
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
