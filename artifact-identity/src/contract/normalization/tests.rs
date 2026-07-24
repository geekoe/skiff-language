use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    websocket_ingress::{canonical_websocket_shape_spec, WebSocketContractBuiltin, WebSocketShape},
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractTypeId, ContractTypeRef, ServiceContract,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION, WEBSOCKET_INGRESS_OPERATION_NAME,
};

use super::{normalize_contract_operation_contract, normalize_contract_type_ref};
use crate::{assign_service_contract_identities, contract_operation_id, service_protocol_identity};

#[test]
fn websocket_builtin_normalization_consumes_only_canonical_public_vocabulary() {
    let spec = canonical_websocket_shape_spec();
    assert_eq!(
        spec.contract_builtins()
            .iter()
            .map(|builtin| (builtin.builtin(), builtin.context_arity()))
            .collect::<Vec<_>>(),
        vec![
            (WebSocketContractBuiltin::Event, 1),
            (WebSocketContractBuiltin::Result, 1),
        ]
    );

    let context = ContractTypeRef::contract(ContractTypeId::new("type:context"));
    for builtin in spec.contract_builtins() {
        let ty = ContractTypeRef::Builtin {
            name: builtin.name().to_string(),
            arguments: vec![context.clone(); builtin.context_arity()],
        };
        assert_eq!(
            normalize_contract_type_ref(ty.clone(), "operation.type").unwrap(),
            ty
        );
    }
}

#[test]
fn websocket_nested_shape_names_are_not_contract_builtins() {
    let spec = canonical_websocket_shape_spec();
    let mut nested_names = BTreeSet::new();
    for (shape_id, shape) in spec.shapes() {
        if spec
            .contract_builtin_named(shape_id.canonical_name())
            .is_none()
        {
            nested_names.insert(shape_id.canonical_name());
        }
        if let WebSocketShape::TaggedUnion { variants, .. } = shape {
            nested_names.extend(variants.iter().map(|variant| variant.canonical_name()));
        }
    }
    assert!(!nested_names.is_empty());

    for name in nested_names {
        let error = normalize_contract_type_ref(
            ContractTypeRef::builtin(name),
            "operation.parameters[0].ty",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(&format!("unknown contract builtin `{name}`")),
            "nested WebSocket shape `{name}` was not rejected: {error}"
        );
    }
}

#[test]
fn websocket_builtin_wrong_arity_and_foreign_name_fail_closed() {
    let spec = canonical_websocket_shape_spec();
    for builtin in spec.contract_builtins() {
        let context_arity = builtin.context_arity();
        assert_eq!(context_arity, 1);
        for actual_arity in [0, context_arity + 1] {
            let error = normalize_contract_type_ref(
                ContractTypeRef::Builtin {
                    name: builtin.name().to_string(),
                    arguments: vec![ContractTypeRef::builtin("null"); actual_arity],
                },
                "operation.type",
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(&format!(
                    "builtin {} expects {} arguments, got {actual_arity}",
                    builtin.name(),
                    context_arity
                )),
                "wrong WebSocket builtin arity was not rejected: {error}"
            );
        }
    }

    let foreign_name = "std.websocket.ForeignBuiltin";
    let error = normalize_contract_type_ref(
        ContractTypeRef::Builtin {
            name: foreign_name.to_string(),
            arguments: vec![ContractTypeRef::builtin("null")],
        },
        "operation.type",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains(&format!("unknown contract builtin `{foreign_name}`")));
}

#[test]
fn websocket_contract_identity_remains_bit_identical() {
    let mut contract = websocket_contract_fixture();
    let operation = &contract.operations.values().next().unwrap().contract;
    assert_eq!(
        normalize_contract_operation_contract(operation.clone(), "operations[websocket]").unwrap(),
        *operation
    );

    let assigned = assign_service_contract_identities(&mut contract).unwrap();
    assert_eq!(assigned, service_protocol_identity(&contract).unwrap());
    assert_eq!(
        assigned.as_str(),
        "skiff-service-protocol-v2:sha256:dcd43002feeeed67b8f98cf6c159f934e03b91f8191ccedb7c2abdc0eb6f0004"
    );
}

fn websocket_contract_fixture() -> ServiceContract {
    let service_id = "example.websocket";
    let contract_version = "1.0.0";
    let operation_id = contract_operation_id(
        service_id,
        contract_version,
        WEBSOCKET_INGRESS_OPERATION_NAME,
    )
    .unwrap();
    let spec = canonical_websocket_shape_spec();
    let event = spec.contract_builtin(WebSocketContractBuiltin::Event);
    let result = spec.contract_builtin(WebSocketContractBuiltin::Result);
    let context = ContractTypeRef::builtin("null");
    let operation = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: WEBSOCKET_INGRESS_OPERATION_NAME.to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "event".to_string(),
                ty: ContractTypeRef::Builtin {
                    name: event.name().to_string(),
                    arguments: vec![context.clone(); event.context_arity()],
                },
                value_plan: linkable_plan(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::Nullable {
                    inner: Box::new(ContractTypeRef::Builtin {
                        name: result.name().to_string(),
                        arguments: vec![context; result.context_arity()],
                    }),
                },
                value_plan: linkable_plan(BoundaryValueOwner::Provider),
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

    ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, operation)]),
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "WebSocket ingress".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    }
}

fn linkable_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
