use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_eval::{
    error::{RuntimeError, UserException},
    exceptions::{
        annotate_runtime_type_plan, request_exception_for_catch, request_exception_for_rethrow,
    },
    runtime_ops::{runtime_carrier_for_plan, runtime_representation_wrap_for_plan},
};
use skiff_runtime_linked_program::{
    FileAddr, LinkOverlay, LinkedNominalTypeRefBase, LinkedTypeDescriptor, LinkedTypeRef,
    RuntimeTypeContext, TypeAddr, TypeDeclIr, UnitAddr,
};
use skiff_runtime_linked_type_plan::ProgramTypeView;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueCarrier},
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InstantiatedTypeArgumentIdentity,
        LiteralIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
        NamedUnionOwnerIdentity, NominalTypeIdentity, RequestException,
    },
    type_plan::{RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan},
};

fn local_nominal(unit: UnitAddr, type_index: usize, arguments: &[&str]) -> NominalTypeIdentity {
    NominalTypeIdentity::LocalExecution(LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit,
            file: FileAddr::loaded_file(0),
            type_index,
        },
        type_arguments: arguments
            .iter()
            .map(|argument| {
                InstantiatedTypeArgumentIdentity::new((*argument).to_string())
                    .expect("type argument identity")
            })
            .collect(),
    })
}

fn identified_plan(node: RuntimeTypeNode, identity: CatchIdentity) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "identified fixture".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan {
            catch_identity: Some(identity),
            ..RuntimeTypeIdentityPlan::default()
        },
        node,
    }
}

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::new("string", None, RuntimeTypeNode::String)
}

fn representation_plan(identity: NominalTypeIdentity, payload: RuntimeTypePlan) -> RuntimeTypePlan {
    identified_plan(
        RuntimeTypeNode::Representation {
            type_name: "Representation".to_string(),
            payload: Box::new(payload),
        },
        CatchIdentity::Nominal(identity),
    )
}

fn union_branch_plan(
    union: NominalTypeIdentity,
    branch: NamedUnionBranchIdentity,
    node: RuntimeTypeNode,
) -> RuntimeTypePlan {
    identified_plan(
        node,
        CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(match union {
                NominalTypeIdentity::LocalExecution(identity) => identity,
                _ => panic!("fixture union must be local execution"),
            }),
            branch,
        },
    )
}

#[test]
fn wrap_preserves_raw_value_and_replaces_only_the_outer_identity() {
    let inner = local_nominal(UnitAddr::Service, 1, &[]);
    let outer = local_nominal(UnitAddr::Service, 2, &[]);
    let mut heap = RequestHeap::default();

    let raw = RuntimeValue::from("payload");
    let inner_carrier = runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(raw.clone()),
        &representation_plan(inner.clone(), string_plan()),
        "inner wrap",
        &mut heap,
    )
    .expect("primitive-backed representation");
    let outer_carrier = runtime_representation_wrap_for_plan(
        inner_carrier.clone(),
        &representation_plan(
            outer.clone(),
            representation_plan(inner.clone(), string_plan()),
        ),
        "outer wrap",
        &mut heap,
    )
    .expect("nested representation");

    assert_eq!(inner_carrier.value(), &raw);
    assert_eq!(outer_carrier.value(), &raw);
    assert_eq!(
        inner_carrier.catch_identity(),
        Some(&CatchIdentity::Nominal(inner))
    );
    assert_eq!(
        outer_carrier.catch_identity(),
        Some(&CatchIdentity::Nominal(outer))
    );
    assert_ne!(
        inner_carrier.catch_identity(),
        outer_carrier.catch_identity()
    );
}

#[test]
fn wrap_target_identity_keeps_external_owner_and_ordered_generic_arguments() {
    let target_addr = TypeAddr {
        unit: UnitAddr::Package(4),
        file: FileAddr::loaded_file(3),
        type_index: 7,
    };
    let mut types = RuntimeTypeContext::default();
    types.descriptors.insert(
        target_addr.clone(),
        TypeDeclIr {
            name: "ExternalPair".to_string(),
            descriptor: LinkedTypeDescriptor::Representation {
                representation: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            },
            type_params: vec!["Left".to_string(), "Right".to_string()],
            implements: Vec::new(),
            source_span: None,
        },
    );
    let service_files = Vec::new();
    let packages = Vec::new();
    let package_files = Vec::new();
    let overlay = LinkOverlay::default();
    let program = ProgramTypeView::new(&service_files, &packages, &package_files, &overlay, &types);
    let arguments = vec![
        LinkedTypeRef::Native {
            name: "number".to_string(),
            args: Vec::new(),
        },
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        },
    ];
    let target = LinkedTypeRef::AppliedNominal {
        base: LinkedNominalTypeRefBase::Address {
            addr: target_addr.clone(),
        },
        arguments: arguments.clone(),
    };
    let mut plan = RuntimeTypePlan::new(
        "representation",
        None,
        RuntimeTypeNode::Representation {
            type_name: "ExternalPair".to_string(),
            payload: Box::new(string_plan()),
        },
    );

    annotate_runtime_type_plan(&mut plan, &target, program).expect("exact target annotation");

    let expected = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: target_addr.clone(),
            type_arguments: arguments
                .iter()
                .map(|argument| {
                    InstantiatedTypeArgumentIdentity::new(
                        serde_json::to_string(argument).expect("canonical linked type argument"),
                    )
                    .expect("type argument identity")
                })
                .collect(),
        },
    ));
    assert_eq!(plan.catch_identity(), Some(&expected));

    let reversed = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: target_addr,
            type_arguments: arguments
                .iter()
                .rev()
                .map(|argument| {
                    InstantiatedTypeArgumentIdentity::new(
                        serde_json::to_string(argument).expect("canonical linked type argument"),
                    )
                    .expect("type argument identity")
                })
                .collect(),
        },
    ));
    assert_ne!(plan.catch_identity(), Some(&reversed));
}

#[test]
fn wrap_fails_closed_for_wrong_plan_missing_identity_and_payload_conflict() {
    let representation = local_nominal(UnitAddr::Service, 1, &[]);
    let other = local_nominal(UnitAddr::Service, 2, &[]);
    let mut heap = RequestHeap::default();

    assert!(runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(RuntimeValue::from("payload")),
        &string_plan(),
        "wrong target",
        &mut heap,
    )
    .is_err());
    assert!(runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(RuntimeValue::from("payload")),
        &RuntimeTypePlan::new(
            "unidentified representation",
            None,
            RuntimeTypeNode::Representation {
                type_name: "Missing".to_string(),
                payload: Box::new(string_plan()),
            },
        ),
        "missing target identity",
        &mut heap,
    )
    .is_err());
    assert!(runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(RuntimeValue::from("payload")),
        &representation_plan(
            representation.clone(),
            RuntimeTypePlan::new(
                "unidentified nested representation",
                None,
                RuntimeTypeNode::Representation {
                    type_name: "MissingPayloadIdentity".to_string(),
                    payload: Box::new(string_plan()),
                },
            ),
        ),
        "missing nested payload identity",
        &mut heap,
    )
    .is_err());
    assert!(runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(RuntimeValue::from("payload")),
        &representation_plan(
            other.clone(),
            representation_plan(representation.clone(), string_plan()),
        ),
        "missing payload identity",
        &mut heap,
    )
    .is_err());
    assert!(runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::identified(
            RuntimeValue::from("payload"),
            CatchIdentity::Nominal(other.clone()),
        ),
        &representation_plan(representation.clone(), string_plan()),
        "conflicting payload identity",
        &mut heap,
    )
    .is_err());
    assert!(runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(RuntimeValue::Number(7.0)),
        &representation_plan(representation, string_plan()),
        "wrong payload value",
        &mut heap,
    )
    .is_err());
}

#[test]
fn named_union_context_promotes_only_the_exact_concrete_nominal() {
    let string_repr = local_nominal(UnitAddr::Service, 1, &["string"]);
    let number_repr = local_nominal(UnitAddr::Service, 1, &["number"]);
    let first_union = local_nominal(UnitAddr::Service, 2, &["string"]);
    let second_union = local_nominal(UnitAddr::Service, 3, &["string"]);
    let concrete_branch =
        |identity: NominalTypeIdentity| NamedUnionBranchIdentity::ConcreteNominal { identity };
    let target_union = |owner: NominalTypeIdentity| {
        RuntimeTypePlan::new(
            "target union",
            None,
            RuntimeTypeNode::Union(vec![union_branch_plan(
                owner,
                concrete_branch(string_repr.clone()),
                RuntimeTypeNode::Representation {
                    type_name: "GenericRepresentation".to_string(),
                    payload: Box::new(string_plan()),
                },
            )]),
        )
    };
    let actual = RuntimeValueCarrier::identified(
        RuntimeValue::from("payload"),
        CatchIdentity::Nominal(string_repr.clone()),
    );
    let mut heap = RequestHeap::default();

    let first = runtime_carrier_for_plan(
        actual.clone(),
        &target_union(first_union.clone()),
        "first union",
        &mut heap,
    )
    .expect("exact concrete branch");
    let second = runtime_carrier_for_plan(
        actual.clone(),
        &target_union(second_union.clone()),
        "second union",
        &mut heap,
    )
    .expect("same concrete nominal in a different target union");

    assert_eq!(
        first.catch_identity(),
        Some(&CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(match first_union {
                NominalTypeIdentity::LocalExecution(identity) => identity,
                _ => unreachable!(),
            }),
            branch: concrete_branch(string_repr.clone()),
        })
    );
    assert_eq!(
        second.catch_identity(),
        Some(&CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(match second_union {
                NominalTypeIdentity::LocalExecution(identity) => identity,
                _ => unreachable!(),
            }),
            branch: concrete_branch(string_repr.clone()),
        })
    );
    assert_ne!(first.catch_identity(), second.catch_identity());

    assert!(runtime_carrier_for_plan(
        first.clone(),
        &target_union(local_nominal(UnitAddr::Service, 2, &["number"])),
        "union argument mismatch",
        &mut heap,
    )
    .is_err());

    let wrong_argument = RuntimeValueCarrier::identified(
        RuntimeValue::from("payload"),
        CatchIdentity::Nominal(number_repr),
    );
    assert!(runtime_carrier_for_plan(
        wrong_argument,
        &target_union(local_nominal(UnitAddr::Service, 2, &["number"])),
        "generic mismatch",
        &mut heap,
    )
    .is_err());

    let wrong_nominal = RuntimeValueCarrier::identified(
        RuntimeValue::from("payload"),
        CatchIdentity::Nominal(local_nominal(UnitAddr::Service, 9, &["string"])),
    );
    assert!(runtime_carrier_for_plan(
        wrong_nominal,
        &target_union(local_nominal(UnitAddr::Service, 2, &["string"])),
        "same shape wrong nominal",
        &mut heap,
    )
    .is_err());

    for branch in [
        NamedUnionBranchIdentity::SyntheticDiscriminator {
            discriminator_field: "kind".to_string(),
            discriminator_value: "representation".to_string(),
        },
        NamedUnionBranchIdentity::Literal {
            value: LiteralIdentity::String("payload".to_string()),
        },
    ] {
        let plan = RuntimeTypePlan::new(
            "non-concrete union",
            None,
            RuntimeTypeNode::Union(vec![union_branch_plan(
                local_nominal(UnitAddr::Service, 4, &[]),
                branch,
                RuntimeTypeNode::String,
            )]),
        );
        assert!(
            runtime_carrier_for_plan(actual.clone(), &plan, "non-concrete branch", &mut heap,)
                .is_err()
        );
    }
}

#[test]
fn ordinary_representation_materialization_remains_unchanged() {
    let identity = local_nominal(UnitAddr::Service, 1, &[]);
    let mut heap = RequestHeap::default();
    let carrier = runtime_carrier_for_plan(
        RuntimeValue::from("payload"),
        &representation_plan(identity.clone(), string_plan()),
        "ordinary representation",
        &mut heap,
    )
    .expect("ordinary materialization still assigns its target identity");

    assert_eq!(carrier.value(), &RuntimeValue::from("payload"));
    assert_eq!(
        carrier.catch_identity(),
        Some(&CatchIdentity::Nominal(identity))
    );
}

#[test]
fn wrapped_throw_catch_and_rethrow_keep_the_actual_identity_and_exception_state() {
    let exact = local_nominal(UnitAddr::Service, 1, &["string"]);
    let other = local_nominal(UnitAddr::Service, 2, &["string"]);
    let source = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    };
    let stack = vec![ExceptionStackFrame::Local {
        site: source.clone(),
    }];
    let correlation = ErrorCorrelation {
        trace_id: "trace-wrap".to_string(),
        error_id: "trace-wrap:local-error:1".to_string(),
    };
    let mut heap = RequestHeap::default();
    let payload = runtime_representation_wrap_for_plan(
        RuntimeValueCarrier::unidentified(RuntimeValue::from("denied")),
        &representation_plan(exact.clone(), string_plan()),
        "throw payload",
        &mut heap,
    )
    .expect("wrapped throw payload");
    let exception =
        RequestException::local(payload, source.clone(), stack.clone(), correlation.clone())
            .expect("request-local exception");
    let error = RuntimeError::UserException(UserException::new(exception.clone()));

    let caught = request_exception_for_catch(
        &error,
        &[CatchIdentity::Nominal(exact.clone())],
        source.clone(),
        Vec::new(),
        ErrorCorrelation {
            trace_id: "unused".to_string(),
            error_id: "unused".to_string(),
        },
        &mut heap,
    )
    .expect("catch lookup")
    .expect("exact catch");
    assert!(request_exception_for_catch(
        &error,
        &[CatchIdentity::Nominal(other)],
        source.clone(),
        Vec::new(),
        ErrorCorrelation {
            trace_id: "unused".to_string(),
            error_id: "unused".to_string(),
        },
        &mut heap,
    )
    .expect("catch lookup")
    .is_none());
    assert_eq!(
        caught.local_catch_identity(),
        Some(&CatchIdentity::Nominal(exact))
    );
    assert_eq!(caught.source(), &source);
    assert_eq!(caught.stack(), stack);
    assert_eq!(caught.correlation(), &correlation);

    let handle = heap
        .alloc_exception(caught.clone())
        .expect("exception heap node");
    let exception_value = RuntimeValueCarrier::unidentified(RuntimeValue::Heap(handle));
    let rethrown =
        request_exception_for_rethrow(&exception_value, &heap).expect("request-local rethrow");
    assert_eq!(rethrown, caught);
    assert!(matches!(
        heap.get(handle).expect("same exception node"),
        HeapNode::Exception(stored) if stored == &rethrown
    ));
}
