use skiff_artifact_model::{
    InterfaceInstantiationRef, LiteralIr, ResourceDropPlan, ValueDropPlan, ValueTransferPlan,
};

use super::*;

#[test]
fn pinned_scalars_and_snapshot_roots_produce_complete_drop_plans() {
    let facts = SourceValueTransferFacts::new();
    for name in ["null", "bool", "number", "integer", "Date"] {
        let actual = plan(&facts, &builtin(name)).expect("audited scalar has a lifecycle");
        assert_eq!(
            actual,
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            }
        );
        assert_no_recursive_shape(&actual);
    }
    for name in ["string", "bytes", "Json", "JsonObject"] {
        let actual = plan(&facts, &builtin(name)).expect("audited root has a lifecycle");
        assert_eq!(actual, snapshot_release());
        assert_no_recursive_shape(&actual);
    }
}

#[test]
fn never_and_void_have_exact_trivial_snapshot_plans() {
    let facts = SourceValueTransferFacts::new();
    for name in ["never", "void"] {
        let actual = plan(&facts, &builtin(name))
            .unwrap_or_else(|error| panic!("{name} must have an exact plan: {error:?}"));
        assert_eq!(
            actual,
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            },
            "{name} is uninhabited/value-less and must use the sidecar-free trivial plan"
        );
        assert_no_recursive_shape(&actual);
    }
    // The first CatchResult argument carries the try type; both uninhabited
    // and value-less try results now plan exactly instead of Missing.
    let never_result = generic_builtin("CatchResult", vec![builtin("never"), builtin("number")]);
    assert_eq!(plan(&facts, &never_result), Ok(snapshot_release()));
    let void_result = generic_builtin("CatchResult", vec![builtin("void"), builtin("number")]);
    assert_eq!(plan(&facts, &void_result), Ok(snapshot_release()));
}

#[test]
fn unsupported_builtins_still_fail_closed_after_the_never_slice() {
    let facts = SourceValueTransferFacts::new();
    let error = plan(&facts, &builtin("notABuiltin"))
        .expect_err("unknown builtins must keep failing closed");
    assert!(matches!(
        root_error(&error),
        SourceValueTransferError::UnknownBuiltin { .. }
    ));
    let error = plan(
        &facts,
        &TypeRefIr::Function {
            params: Vec::new(),
            return_type: Box::new(builtin("number")),
        },
    )
    .expect_err("callbacks must keep failing closed");
    assert!(matches!(
        root_error(&error),
        SourceValueTransferError::CallbackTypeUnsupported
    ));
}

#[test]
fn array_map_and_stream_are_derived_from_the_pinned_registry() {
    let facts = SourceValueTransferFacts::new();
    let values = generic_builtin("Array", vec![builtin("string")]);
    let lookup = generic_builtin("Map", vec![builtin("string"), builtin("Json")]);
    assert_eq!(plan(&facts, &values), Ok(snapshot_release()));
    assert_eq!(plan(&facts, &lookup), Ok(snapshot_release()));

    let stream = generic_builtin("Stream", vec![builtin("number")]);
    let actual = plan(&facts, &stream).expect("Stream<number> is an audited resource");
    assert_eq!(
        actual,
        ValueTransferPlan::AffineResource {
            drop: ResourceDropPlan::ResourceTableRelease,
        }
    );
    assert_no_recursive_shape(&actual);
}

#[test]
fn literals_and_ordinary_structures_use_root_only_release() {
    let facts = SourceValueTransferFacts::new();
    assert_eq!(
        plan(
            &facts,
            &TypeRefIr::Literal {
                value: LiteralIr::Number { value: 7.into() },
            },
        ),
        Ok(ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::Trivial,
        })
    );
    assert_eq!(
        plan(
            &facts,
            &TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "ready".to_string(),
                },
            },
        ),
        Ok(snapshot_release())
    );

    for ty in [
        record([
            ("name", builtin("string")),
            ("labels", generic_builtin("Array", vec![builtin("string")])),
        ]),
        TypeRefIr::Union {
            items: vec![builtin("integer"), builtin("null")],
        },
        TypeRefIr::Nullable {
            inner: Box::new(builtin("number")),
        },
    ] {
        let actual = plan(&facts, &ty).expect("ordinary aggregate is snapshot-shareable");
        assert_eq!(actual, snapshot_release());
        assert_no_recursive_shape(&actual);
    }
}

#[test]
fn exact_any_interface_remains_a_snapshot_value_not_a_callback_plan() {
    let facts = SourceValueTransferFacts::new();
    let any_interface = TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: "iface:exact:v1".to_string(),
            canonical_type_args: vec![builtin("string")],
        },
    };
    assert_eq!(plan(&facts, &any_interface), Ok(snapshot_release()));
}
