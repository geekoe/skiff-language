use super::*;

#[test]
fn package_receiver_type_arguments_precede_explicit_method_arguments() {
    let arguments =
        positional_type_arguments([TypeRefIr::builtin("string"), TypeRefIr::builtin("number")]);
    assert_eq!(arguments["T0"], TypeRefIr::builtin("string"));
    assert_eq!(arguments["T1"], TypeRefIr::builtin("number"));
}

#[test]
fn package_receiver_classification_keeps_static_dependency_addresses_out() {
    let static_dependency =
        Expr::DependencySourceAddress(skiff_syntax::ast::DependencySourceAddress {
            dependency_ref: "subjectImpl".to_string(),
            public_path: "internal.makeBox".to_string(),
        });
    assert!(!is_receiver_call_object(&static_dependency, &|_| false));

    let local = Expr::Identifier("box".to_string());
    assert!(is_receiver_call_object(&local, &|name| name == "box"));
    assert!(!is_receiver_call_object(&local, &|_| false));
}
