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
