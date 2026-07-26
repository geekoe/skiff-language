use std::collections::BTreeMap;

use skiff_artifact_identity::service_contract_from_definition;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, ServiceContract,
    ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
    SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION,
};

pub(super) fn contract(stable_key: &str) -> ServiceContract {
    service_contract_from_definition(ServiceContractDefinition {
        schema_version: SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION.to_string(),
        service_id: "example.echo".to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::from([(stable_key.to_string(), operation_contract())]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Echo".to_string(),
            operations: BTreeMap::from([(stable_key.to_string(), "Echo".to_string())]),
            types: BTreeMap::new(),
        },
    })
    .expect("contract fixture")
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
