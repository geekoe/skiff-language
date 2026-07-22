use super::*;
use crate::{
    BoundaryCallbackOperation, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryParameter, BoundaryReturn, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractSchemaType, ContractTypeNameability, ContractTypeShape,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};

#[test]
fn canonical_spec_exposes_only_event_and_result_as_contract_builtins() {
    let spec = canonical_websocket_shape_spec();
    assert!(std::ptr::eq(spec, canonical_websocket_shape_spec()));
    assert_eq!(
        spec.contract_builtins()
            .iter()
            .map(|builtin| (builtin.builtin(), builtin.name(), builtin.context_arity()))
            .collect::<Vec<_>>(),
        vec![
            (
                WebSocketContractBuiltin::Event,
                WEBSOCKET_INGRESS_EVENT_TYPE,
                1,
            ),
            (
                WebSocketContractBuiltin::Result,
                WEBSOCKET_CONNECT_RESULT_TYPE,
                1,
            ),
        ]
    );
    assert_eq!(
        spec.contract_builtin(WebSocketContractBuiltin::Event)
            .shape(),
        WebSocketShapeId::Event
    );
    assert_eq!(
        spec.contract_builtin(WebSocketContractBuiltin::Result)
            .shape(),
        WebSocketShapeId::Result
    );
    for nested in [
        WebSocketShapeId::ConnectRequest,
        WebSocketShapeId::ReceiveEvent,
        WebSocketShapeId::Connection,
        WebSocketShapeId::Message,
        WebSocketShapeId::ConnectionPolicy,
        WebSocketShapeId::HttpHeader,
        WebSocketShapeId::HttpQueryParam,
    ] {
        assert!(spec
            .contract_builtin_named(nested.canonical_name())
            .is_none());
    }
    assert_eq!(spec.shapes().len(), 9);
}

#[test]
fn websocket_admission_consumes_context_arity_from_builtin_spec() {
    let operation_id = ContractOperationId::new("operation:websocket");
    let one_argument_contract = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::builtin("null"),
        BTreeMap::new(),
    );
    let mut arity_drift = build_canonical_websocket_shape_spec();
    for builtin in &mut arity_drift.contract_builtins {
        builtin.context_arity = 2;
    }

    assert_error_contains(
        websocket_ingress_context_with_shape_spec(
            &one_argument_contract,
            &operation_id,
            &arity_drift,
        ),
        "event must be",
    );

    let mut two_argument_contract = one_argument_contract;
    let operation = &mut two_argument_contract
        .operations
        .get_mut(&operation_id)
        .expect("WebSocket operation is present")
        .contract;
    let ContractTypeRef::Builtin { arguments, .. } = &mut operation.parameters[0].ty else {
        panic!("event must be a builtin")
    };
    arguments.push(ContractTypeRef::builtin("null"));
    let ContractTypeRef::Nullable { inner } = &mut operation.return_value.ty else {
        panic!("result must be nullable")
    };
    let ContractTypeRef::Builtin { arguments, .. } = inner.as_mut() else {
        panic!("result must wrap a builtin")
    };
    arguments.push(ContractTypeRef::builtin("null"));

    assert_eq!(
        websocket_ingress_context_with_shape_spec(
            &two_argument_contract,
            &operation_id,
            &arity_drift,
        )
        .unwrap(),
        WebSocketIngressContext::Null
    );
}

#[test]
fn canonical_event_spec_covers_connect_and_receive_nested_shapes_exactly() {
    use WebSocketShapeId as Id;
    use WebSocketShapeType as Ty;

    let spec = canonical_websocket_shape_spec();
    assert_tagged_union(
        spec,
        Id::Event,
        "tag",
        &[
            (
                "connect",
                "std.websocket.WebSocketIngressConnectEvent",
                &["tag", "connectRequest"],
            ),
            (
                "receive",
                "std.websocket.WebSocketIngressReceiveEvent",
                &["tag", "receiveEvent"],
            ),
        ],
    );
    assert_eq!(
        record_fields(spec, Id::ConnectRequest),
        &[
            field("connectionId", Ty::String),
            field("url", Ty::String),
            field("query", array(Ty::Shape(Id::HttpQueryParam))),
            field("headers", array(Ty::Shape(Id::HttpHeader))),
            field("cookies", array(Ty::Shape(Id::HttpHeader))),
            field("version", nullable(Ty::String)),
        ]
    );
    assert_eq!(
        record_fields(spec, Id::ReceiveEvent),
        &[
            field("connection", Ty::Shape(Id::Connection)),
            field("message", Ty::Shape(Id::Message)),
        ]
    );
    assert_eq!(
        record_fields(spec, Id::Connection),
        &[
            field("id", Ty::String),
            field("businessIdentity", nullable(Ty::String)),
            field("context", Ty::Context),
        ]
    );
    assert_tagged_union(
        spec,
        Id::Message,
        "tag",
        &[
            (
                "text",
                "std.websocket.TextConnectionMessage",
                &["tag", "text"],
            ),
            (
                "binary",
                "std.websocket.BinaryConnectionMessage",
                &["tag", "base64"],
            ),
        ],
    );
    let event_variants = tagged_variants(spec, Id::Event);
    assert_eq!(
        event_variants[0].fields()[1].ty(),
        &Ty::Shape(Id::ConnectRequest)
    );
    assert_eq!(
        event_variants[1].fields()[1].ty(),
        &Ty::Shape(Id::ReceiveEvent)
    );
    let message_variants = tagged_variants(spec, Id::Message);
    assert_eq!(message_variants[0].fields()[1].ty(), &Ty::String);
    assert_eq!(message_variants[1].fields()[1].ty(), &Ty::String);
    assert_eq!(
        record_fields(spec, Id::HttpHeader),
        &[field("name", Ty::String), field("value", Ty::String)]
    );
    assert_eq!(
        record_fields(spec, Id::HttpQueryParam),
        &[field("name", Ty::String), field("value", Ty::String)]
    );
}

#[test]
fn canonical_result_spec_covers_accept_reject_and_policy_exactly() {
    use WebSocketShapeId as Id;
    use WebSocketShapeType as Ty;

    let spec = canonical_websocket_shape_spec();
    assert_tagged_union(
        spec,
        Id::Result,
        "tag",
        &[
            (
                "accept",
                "std.websocket.WebSocketConnectAccept",
                &["tag", "context", "businessIdentity", "connectionPolicy"],
            ),
            (
                "reject",
                "std.websocket.WebSocketConnectReject",
                &["tag", "code", "reason"],
            ),
        ],
    );
    assert_eq!(
        record_fields(spec, Id::ConnectionPolicy),
        &[
            field("maxConnections", Ty::Integer),
            field(
                "overflow",
                Ty::StringLiteralUnion(vec!["close-oldest", "reject-new"]),
            ),
            field("closeCode", nullable(Ty::Integer)),
            field("closeReason", nullable(Ty::String)),
        ]
    );

    let WebSocketShape::TaggedUnion { variants, .. } = spec.shape(Id::Result) else {
        panic!("Result must be a tagged union")
    };
    assert_eq!(variants[0].fields()[1].ty(), &Ty::Context);
    assert_eq!(variants[0].fields()[2].ty(), &nullable(Ty::String));
    assert_eq!(
        variants[0].fields()[3].ty(),
        &nullable(Ty::Shape(Id::ConnectionPolicy))
    );
    assert_eq!(variants[1].fields()[1].ty(), &Ty::Integer);
    assert_eq!(variants[1].fields()[2].ty(), &Ty::String);
}

#[test]
fn websocket_context_accepts_null_and_exact_persistable_nominal_graphs() {
    let operation_id = ContractOperationId::new("operation:websocket");
    let null_contract = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::builtin("null"),
        BTreeMap::new(),
    );
    assert_eq!(
        websocket_ingress_context(&null_contract, &operation_id).unwrap(),
        WebSocketIngressContext::Null
    );

    let context_id = ContractTypeId::new("type:context");
    let role_id = ContractTypeId::new("type:role");
    let nominal_contract = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(context_id.clone()),
        BTreeMap::from([
            (
                context_id.clone(),
                schema_type(
                    context_id.clone(),
                    "Context",
                    ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([
                            (
                                "role".to_string(),
                                ContractTypeRef::contract(role_id.clone()),
                            ),
                            (
                                "expiresAfter".to_string(),
                                ContractTypeRef::builtin("Duration"),
                            ),
                        ]),
                    },
                ),
            ),
            (
                role_id.clone(),
                schema_type(
                    role_id,
                    "Role",
                    ContractTypeDescriptor::Enumeration {
                        variants: vec!["member".to_string(), "admin".to_string()],
                    },
                ),
            ),
        ]),
    );
    assert_eq!(
        websocket_ingress_context(&nominal_contract, &operation_id).unwrap(),
        WebSocketIngressContext::Contract(context_id)
    );
}

#[test]
fn websocket_context_requires_identical_null_or_same_contract_nominal_refs() {
    let operation_id = ContractOperationId::new("operation:websocket");
    let missing_id = ContractTypeId::new("type:foreign");
    let foreign = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(missing_id),
        BTreeMap::new(),
    );
    assert_error_contains(
        websocket_ingress_context(&foreign, &operation_id),
        "owned by the same ServiceContract",
    );

    let scalar = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::builtin("string"),
        BTreeMap::new(),
    );
    assert_error_contains(
        websocket_ingress_context(&scalar, &operation_id),
        "null or a contract-owned nominal type",
    );

    let context_id = ContractTypeId::new("type:context");
    let mut mismatch = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(context_id.clone()),
        BTreeMap::from([(
            context_id.clone(),
            schema_type(
                context_id,
                "Context",
                ContractTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            ),
        )]),
    );
    mismatch
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract
        .return_value
        .ty = nullable_generic(
        WEBSOCKET_CONNECT_RESULT_TYPE,
        ContractTypeRef::builtin("null"),
    );
    assert_error_contains(
        websocket_ingress_context(&mismatch, &operation_id),
        "event and result Context must be identical",
    );
}

#[test]
fn websocket_context_rejects_callback_missing_cycle_and_alias_graphs() {
    let operation_id = ContractOperationId::new("operation:websocket");
    let context_id = ContractTypeId::new("type:context");
    let callback_id = ContractTypeId::new("type:callback");
    let callback_graph = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(context_id.clone()),
        BTreeMap::from([
            (
                context_id.clone(),
                schema_type(
                    context_id.clone(),
                    "Context",
                    ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "listener".to_string(),
                            ContractTypeRef::contract(callback_id.clone()),
                        )]),
                    },
                ),
            ),
            (
                callback_id.clone(),
                schema_type(
                    callback_id,
                    "Listener",
                    ContractTypeDescriptor::CallbackInterface {
                        operations: BTreeMap::from([(
                            "notify".to_string(),
                            BoundaryCallbackOperation {
                                parameters: vec![ContractTypeRef::builtin("string")],
                                return_type: ContractTypeRef::builtin("void"),
                                may_suspend: false,
                            },
                        )]),
                    },
                ),
            ),
        ]),
    );
    assert_error_contains(
        websocket_ingress_context(&callback_graph, &operation_id),
        "CallbackInterface",
    );

    let missing_id = ContractTypeId::new("type:missing");
    let missing_graph = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(context_id.clone()),
        BTreeMap::from([(
            context_id.clone(),
            schema_type(
                context_id.clone(),
                "Context",
                ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "missing".to_string(),
                        ContractTypeRef::contract(missing_id),
                    )]),
                },
            ),
        )]),
    );
    assert_error_contains(
        websocket_ingress_context(&missing_graph, &operation_id),
        "missing or foreign ContractTypeId",
    );

    let child_id = ContractTypeId::new("type:child");
    let cycle = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(context_id.clone()),
        BTreeMap::from([
            (
                context_id.clone(),
                schema_type(
                    context_id.clone(),
                    "Context",
                    ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "child".to_string(),
                            ContractTypeRef::contract(child_id.clone()),
                        )]),
                    },
                ),
            ),
            (
                child_id.clone(),
                schema_type(
                    child_id,
                    "Child",
                    ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "parent".to_string(),
                            ContractTypeRef::contract(context_id.clone()),
                        )]),
                    },
                ),
            ),
        ]),
    );
    assert_error_contains(
        websocket_ingress_context(&cycle, &operation_id),
        "contract schema cycle",
    );

    let alias = websocket_contract(
        operation_id.clone(),
        ContractTypeRef::contract(context_id.clone()),
        BTreeMap::from([(
            context_id.clone(),
            schema_type(
                context_id,
                "Context",
                ContractTypeDescriptor::Alias {
                    target: ContractTypeRef::builtin("string"),
                },
            ),
        )]),
    );
    assert_error_contains(
        websocket_ingress_context(&alias, &operation_id),
        "not an exact persistable nominal type",
    );
}

fn record_fields(
    spec: &CanonicalWebSocketShapeSpec,
    shape: WebSocketShapeId,
) -> &[WebSocketShapeField] {
    let WebSocketShape::Record { fields } = spec.shape(shape) else {
        panic!("{} must be a record", shape.canonical_name())
    };
    fields
}

fn assert_tagged_union(
    spec: &CanonicalWebSocketShapeSpec,
    shape: WebSocketShapeId,
    expected_discriminator: &str,
    expected: &[(&str, &str, &[&str])],
) {
    let WebSocketShape::TaggedUnion {
        discriminator_field,
        variants,
    } = spec.shape(shape)
    else {
        panic!("{} must be a tagged union", shape.canonical_name())
    };
    assert_eq!(*discriminator_field, expected_discriminator);
    assert_eq!(variants.len(), expected.len());
    for (variant, (tag, canonical_name, fields)) in variants.iter().zip(expected) {
        assert_eq!(variant.tag(discriminator_field), Some(*tag));
        assert_eq!(variant.canonical_name(), *canonical_name);
        assert_eq!(
            variant
                .fields()
                .iter()
                .map(WebSocketShapeField::name)
                .collect::<Vec<_>>(),
            *fields
        );
    }
}

fn tagged_variants(
    spec: &CanonicalWebSocketShapeSpec,
    shape: WebSocketShapeId,
) -> &[WebSocketTaggedVariant] {
    let WebSocketShape::TaggedUnion { variants, .. } = spec.shape(shape) else {
        panic!("{} must be a tagged union", shape.canonical_name())
    };
    variants
}

fn websocket_contract(
    operation_id: ContractOperationId,
    context: ContractTypeRef,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
) -> ServiceContract {
    ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: "example.websocket".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id,
                stable_key: WEBSOCKET_INGRESS_OPERATION_NAME.to_string(),
                contract: websocket_operation(context),
            },
        )]),
        boundary_schema,
        diagnostic_text: ContractDiagnosticText {
            service: String::new(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    }
}

fn websocket_operation(context: ContractTypeRef) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "event".to_string(),
            ty: generic(WEBSOCKET_INGRESS_EVENT_TYPE, context.clone()),
            value_plan: linkable_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: nullable_generic(WEBSOCKET_CONNECT_RESULT_TYPE, context),
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
    }
}

fn generic(name: &str, context: ContractTypeRef) -> ContractTypeRef {
    ContractTypeRef::Builtin {
        name: name.to_string(),
        arguments: vec![context],
    }
}

fn nullable_generic(name: &str, context: ContractTypeRef) -> ContractTypeRef {
    ContractTypeRef::Nullable {
        inner: Box::new(generic(name, context)),
    }
}

fn schema_type(
    contract_type_id: ContractTypeId,
    stable_key: &str,
    descriptor: ContractTypeDescriptor,
) -> ContractSchemaType {
    ContractSchemaType {
        contract_type_id,
        stable_key: stable_key.to_string(),
        shape: ContractTypeShape {
            nameability: ContractTypeNameability::PublicNameable,
            descriptor,
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

fn assert_error_contains(
    result: Result<WebSocketIngressContext, WebSocketIngressContractError>,
    expected: &str,
) {
    let error = result.expect_err("WebSocket Context must fail closed");
    assert!(
        error.to_string().contains(expected),
        "expected `{expected}` in `{error}`"
    );
}
