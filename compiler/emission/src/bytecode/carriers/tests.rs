use skiff_artifact_model::{LiteralIr, TypeRefIr};

use super::{
    model::SemanticRole,
    policy::{catch_default_literal, literal_carrier_type, semantic_accepts_carrier},
};

#[test]
fn literal_carriers_are_physical_vm_kinds() {
    let cases = [
        (LiteralIr::Null, "null"),
        (LiteralIr::Bool { value: true }, "bool"),
        (
            LiteralIr::Number {
                value: serde_json::Number::from(1_u64),
            },
            "number",
        ),
        (
            LiteralIr::String {
                value: "x".to_string(),
            },
            "string",
        ),
    ];
    for (literal, expected) in cases {
        assert_eq!(literal_carrier_type(&literal), TypeRefIr::builtin(expected));
    }
}

#[test]
fn catch_defaults_publish_their_actual_scalar_producers() {
    let cases = [
        (
            TypeRefIr::builtin("integer"),
            LiteralIr::Number {
                value: serde_json::Number::from(0_u64),
            },
            TypeRefIr::builtin("number"),
        ),
        (
            TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::builtin("integer")),
            },
            LiteralIr::Null,
            TypeRefIr::builtin("null"),
        ),
        (
            TypeRefIr::builtin("string"),
            LiteralIr::String {
                value: String::new(),
            },
            TypeRefIr::builtin("string"),
        ),
    ];
    for (semantic, expected_value, expected_carrier) in cases {
        let (value, carrier) = catch_default_literal(&semantic)
            .unwrap_or_else(|| panic!("{semantic:?} has an exact default producer"));
        assert_eq!(value, expected_value);
        assert_eq!(carrier, expected_carrier);
    }
}

#[test]
fn semantic_mapping_is_producer_driven_and_fail_closed() {
    let string_literal = TypeRefIr::Literal {
        value: LiteralIr::String {
            value: "tag".to_string(),
        },
    };
    assert!(semantic_accepts_carrier(
        &string_literal,
        &TypeRefIr::builtin("string"),
        SemanticRole::Position,
    ));
    assert!(semantic_accepts_carrier(
        &TypeRefIr::builtin("integer"),
        &TypeRefIr::builtin("number"),
        SemanticRole::Position,
    ));
    assert!(!semantic_accepts_carrier(
        &TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::builtin("number")),
        },
        &TypeRefIr::builtin("number"),
        SemanticRole::Position,
    ));
    assert!(semantic_accepts_carrier(
        &TypeRefIr::builtin("integer"),
        &TypeRefIr::builtin("integer"),
        SemanticRole::Expression,
    ));
    assert!(!semantic_accepts_carrier(
        &TypeRefIr::builtin("integer"),
        &TypeRefIr::builtin("string"),
        SemanticRole::Position,
    ));
}
