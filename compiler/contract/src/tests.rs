use std::collections::BTreeMap;

use skiff_artifact_identity::package_schema_type_id;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeDescriptor, ContractTypeRef, PackageSchemaCanonicalDescriptor,
    PackageTypeRequirement,
};

use crate::{
    compile_service_contract_definition, ContractDefinitionError, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};

fn operation(ty: ContractTypeRef) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty,
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
    }
}

#[test]
fn package_owned_type_identity_is_independent_of_service_and_version() {
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    };
    let id = package_schema_type_id("example.pkg", "User", &descriptor).unwrap();
    let reference = ContractTypeRef::package_schema("example.pkg", "User", id.clone());
    let compile = |service: &str, version: &str| {
        compile_service_contract_definition(ServiceContractDefinition {
            service_id: service.to_string(),
            contract_version: version.to_string(),
            operations: BTreeMap::from([("get".to_string(), operation(reference.clone()))]),
            package_type_requirements: vec![PackageTypeRequirement {
                package_id: "example.pkg".to_string(),
                required_type_ids: vec![id.clone()],
            }],
            diagnostic_text: ServiceContractDefinitionDiagnosticText {
                service: service.to_string(),
                operations: BTreeMap::from([("get".to_string(), "get".to_string())]),
                types: BTreeMap::from([(id.clone(), "User".to_string())]),
            },
        })
        .unwrap()
    };
    let first = compile("service.one", "1.0.0");
    let second = compile("service.two", "9.0.0");
    assert_eq!(
        first.package_type_requirements[0].required_type_ids,
        second.package_type_requirements[0].required_type_ids
    );
}

#[test]
fn standalone_zero_operation_definition_is_rejected_even_with_type_requirements() {
    let definition = ServiceContractDefinition {
        service_id: "example.empty".to_string(),
        contract_version: "1.0.0".to_string(),
        operations: BTreeMap::new(),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: "example.types".to_string(),
            required_type_ids: vec![skiff_artifact_model::PackageSchemaTypeId::new(
                "type:unreachable",
            )],
        }],
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "example.empty".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };

    assert!(matches!(
        compile_service_contract_definition(definition),
        Err(ContractDefinitionError::EmptyOperations)
    ));
}
