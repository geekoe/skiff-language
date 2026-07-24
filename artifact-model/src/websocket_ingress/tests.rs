use std::collections::BTreeMap;

use crate::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractOperationId, ContractTypeDescriptor, ContractTypeRef,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSchemaTypeRef, PackageTypeRequirement, ServiceContract, ServiceProtocolIdentity,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};

use super::{websocket_ingress_context, WebSocketIngressContext, WEBSOCKET_INGRESS_OPERATION_NAME};

const PACKAGE_ID: &str = "example.context";

#[test]
fn websocket_context_resolves_package_owned_schema_record() {
    let context_id = PackageSchemaTypeId::new("type:context");
    let context_ref = ContractTypeRef::package_schema(PACKAGE_ID, "Context", context_id.clone());
    let (contract, operation_id) = websocket_contract(context_ref);
    let records = BTreeMap::from([(
        context_id.clone(),
        record(
            "Context",
            context_id.clone(),
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "session".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        ),
    )]);

    assert_eq!(
        websocket_ingress_context(&contract, &operation_id, &records).unwrap(),
        WebSocketIngressContext::PackageSchema(PackageSchemaTypeRef {
            package_id: PACKAGE_ID.to_string(),
            stable_schema_key: "Context".to_string(),
            package_schema_type_id: context_id,
        })
    );
}

#[test]
fn websocket_context_fails_closed_without_required_record() {
    let context_id = PackageSchemaTypeId::new("type:context");
    let (contract, operation_id) = websocket_contract(ContractTypeRef::package_schema(
        PACKAGE_ID, "Context", context_id,
    ));
    let error = websocket_ingress_context(&contract, &operation_id, &BTreeMap::new())
        .expect_err("missing content-addressed record must fail");
    assert!(error.to_string().contains("missing PackageSchemaTypeId"));
}

#[test]
fn websocket_context_fails_closed_on_package_schema_cycle() {
    let context_id = PackageSchemaTypeId::new("type:context");
    let context_ref = ContractTypeRef::package_schema(PACKAGE_ID, "Context", context_id.clone());
    let (contract, operation_id) = websocket_contract(context_ref.clone());
    let records = BTreeMap::from([(
        context_id.clone(),
        record(
            "Context",
            context_id,
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([("self".to_string(), context_ref)]),
            },
        ),
    )]);
    let error = websocket_ingress_context(&contract, &operation_id, &records)
        .expect_err("v1 recursive schema must fail closed");
    assert!(error.to_string().contains("cycle"));
}

fn websocket_contract(context: ContractTypeRef) -> (ServiceContract, ContractOperationId) {
    let operation_id = ContractOperationId::new("operation:websocket");
    let operation = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: WEBSOCKET_INGRESS_OPERATION_NAME.to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "event".to_string(),
                ty: generic("std.websocket.WebSocketIngressEvent", context.clone()),
                value_plan: value_plan(),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::Nullable {
                    inner: Box::new(generic("std.websocket.WebSocketConnectResult", context)),
                },
                value_plan: value_plan(),
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
    let required_type_ids = operation_type_ids(&operation);
    (
        ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: "example.websocket".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(operation_id.clone(), operation)]),
            package_type_requirements: vec![PackageTypeRequirement {
                package_id: PACKAGE_ID.to_string(),
                required_type_ids,
            }],
            diagnostic_text: ContractDiagnosticText {
                service: String::new(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        },
        operation_id,
    )
}

fn operation_type_ids(operation: &BoundaryOperationDescriptor) -> Vec<PackageSchemaTypeId> {
    let ContractTypeRef::Builtin { arguments, .. } = &operation.contract.parameters[0].ty else {
        unreachable!()
    };
    let ContractTypeRef::PackageSchema {
        package_schema_type_id,
        ..
    } = &arguments[0]
    else {
        unreachable!()
    };
    vec![package_schema_type_id.clone()]
}

fn record(
    stable_schema_key: &str,
    package_schema_type_id: PackageSchemaTypeId,
    descriptor: ContractTypeDescriptor,
) -> PackageSchemaTypeRecord {
    PackageSchemaTypeRecord {
        package_id: PACKAGE_ID.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id,
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor,
        },
    }
}

fn generic(name: &str, context: ContractTypeRef) -> ContractTypeRef {
    ContractTypeRef::Builtin {
        name: name.to_string(),
        arguments: vec![context],
    }
}

fn value_plan() -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Call,
    }
}
