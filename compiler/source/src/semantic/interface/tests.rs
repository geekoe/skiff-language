use std::collections::BTreeMap;

use crate::{
    semantic::{SemanticPublication, SemanticSource},
    shared::parser::parse_source,
};

use super::*;

fn build_semantics(source: &str) -> InterfaceSemantics {
    let ast = parse_source(source).expect("test source should parse");
    let aliases = BTreeMap::new();
    let publication = SemanticPublication::new(vec![SemanticSource::new(
        "test.skiff",
        "test",
        &ast,
        &aliases,
    )]);
    InterfaceSemantics::build(&publication).expect("interface semantics should build")
}

fn inst(symbol: &str, args: Vec<TypeRefIr>) -> InterfaceInstantiation {
    InterfaceInstantiation {
        symbol: SourceSymbolKey::new("test", symbol),
        args,
    }
}

#[test]
fn marker_interface_is_not_object_safe_for_dynamic_any_interface() {
    let semantics = build_semantics("interface Marker {}\n");
    let interface = inst("Marker", Vec::new());

    assert_eq!(
        semantics.object_safety_diagnostics(&interface).unwrap(),
        vec![InterfaceObjectSafetyDiagnostic::MarkerInterface {
            interface: SourceSymbolKey::new("test", "Marker"),
        }]
    );
    assert!(!semantics.is_object_safe_interface(&interface).unwrap());
}

#[test]
fn object_safety_reports_self_outside_receiver() {
    let semantics = build_semantics(
        r#"
            interface CloneLike {
                function clone() -> Self
            }
            "#,
    );
    let interface = inst("CloneLike", Vec::new());
    let diagnostics = semantics.object_safety_diagnostics(&interface).unwrap();

    assert!(matches!(
        diagnostics.as_slice(),
        [InterfaceObjectSafetyDiagnostic::InvalidSelfUsage { method_name, message }]
            if method_name == "clone"
                && message.contains("can only use Self in the first receiver parameter")
    ));
}

#[test]
fn object_safety_reports_missing_self_receiver() {
    let semantics = build_semantics(
        r#"
            interface Reader {
                function read() -> string
            }
            "#,
    );
    let interface = inst("Reader", Vec::new());

    assert_eq!(
        semantics.object_safety_diagnostics(&interface).unwrap(),
        vec![InterfaceObjectSafetyDiagnostic::MissingSelfReceiver {
            method_name: "read".to_string(),
        }]
    );
}

#[test]
fn interface_method_level_generics_fail_closed_in_existing_parser_diagnostic() {
    let error = parse_source(
        r#"
            interface GenericMethod {
                function get<T>() -> T
            }
            "#,
    )
    .expect_err("interface method type params must be rejected")
    .to_string();

    assert!(
        error.contains("interface method requirements cannot declare type parameters"),
        "unexpected error: {error}"
    );
}

#[test]
fn method_slots_substitute_interface_type_args_without_reparsing_text() {
    let semantics = build_semantics(
        r#"
            interface Reader<T> {
                function read(self: Self, fallback: T) -> Array<T>
            }
            "#,
    );
    let interface = inst("Reader", vec![TypeRefIr::builtin("string")]);

    let slots = semantics.method_slots_for_interface(&interface).unwrap();

    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].slot, 0);
    assert_eq!(slots[0].name, "read");
    assert_eq!(
        slots[0].params,
        vec![
            FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::builtin("Self"),
            },
            FunctionTypeParamIr {
                name: "fallback".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
        ]
    );
    assert_eq!(
        slots[0].return_type,
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        }
    );
    assert_eq!(
        slots[0].method_abi_id,
        canonical_interface_method_abi_id(
            &semantics.canonical_interface_instantiation_ref(&interface),
            "read",
        )
    );
}

#[test]
fn local_conformance_lookup_substitutes_generic_receiver_args() {
    let semantics = build_semantics(
        r#"
            interface Reader<T> {
                function read(self: Self) -> T
            }
            type Box<T> implements Reader<T> {}
            impl Box {
                function read() -> T { return }
            }
            "#,
    );
    let receiver = TypeInstantiationPattern {
        symbol: SourceSymbolKey::new("test", "Box"),
        args: vec![TypeRefIr::builtin("string")],
    };
    let interface = inst("Reader", vec![TypeRefIr::builtin("string")]);

    let conformance = semantics
        .local_conformance_for_receiver_instantiation(&receiver, &interface)
        .expect("generic conformance should instantiate for concrete receiver");

    assert_eq!(conformance.receiver, receiver);
    assert_eq!(conformance.interface, interface);
}
