use super::*;

fn type_param(name: &str) -> TypeRefIr {
    TypeRefIr::TypeParam {
        name: name.to_string(),
    }
}

fn native(name: &str, args: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: name.to_string(),
        args,
    }
}

fn param(name: &str, ty: TypeRefIr) -> FunctionTypeParamIr {
    FunctionTypeParamIr {
        name: name.to_string(),
        ty,
    }
}

fn method(
    type_params: Vec<&str>,
    params: Vec<FunctionTypeParamIr>,
    return_type: TypeRefIr,
    implicit_self: Option<TypeRefIr>,
) -> InterfaceMethodSignature {
    InterfaceMethodSignature {
        name: "call".to_string(),
        type_params: type_params.into_iter().map(str::to_string).collect(),
        params,
        return_type,
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self,
    }
}

#[test]
fn instantiates_implicit_self_type_params() {
    let instantiated = instantiate_interface_method_signatures(
        vec![method(
            vec![],
            vec![],
            type_param("T"),
            Some(type_param("T")),
        )],
        &["T".to_string()],
        &[native("String", Vec::new())],
    )
    .unwrap();

    assert_eq!(
        instantiated[0].implicit_self,
        Some(native("String", Vec::new()))
    );
}

#[test]
fn instantiates_nested_params_and_return_type() {
    let mut fields = BTreeMap::new();
    fields.insert("value".to_string(), type_param("T"));
    let instantiated = instantiate_interface_method_signatures(
        vec![method(
            vec![],
            vec![param(
                "items",
                native(
                    "Array",
                    vec![TypeRefIr::Nullable {
                        inner: Box::new(type_param("T")),
                    }],
                ),
            )],
            TypeRefIr::Record { fields },
            None,
        )],
        &["T".to_string()],
        &[native("Number", Vec::new())],
    )
    .unwrap();

    assert_eq!(
        instantiated[0].params[0].ty,
        native(
            "Array",
            vec![TypeRefIr::Nullable {
                inner: Box::new(native("Number", Vec::new())),
            }],
        )
    );
    let TypeRefIr::Record { fields } = &instantiated[0].return_type else {
        panic!("return type should stay a record");
    };
    assert_eq!(fields.get("value"), Some(&native("Number", Vec::new())));
}

#[test]
fn method_type_params_shadow_interface_substitutions() {
    let instantiated = instantiate_interface_method_signatures(
        vec![method(
            vec!["T"],
            vec![param("value", type_param("T"))],
            type_param("T"),
            Some(type_param("T")),
        )],
        &["T".to_string()],
        &[native("String", Vec::new())],
    )
    .unwrap();

    assert_eq!(instantiated[0].params[0].ty, type_param("T"));
    assert_eq!(instantiated[0].return_type, type_param("T"));
    assert_eq!(instantiated[0].implicit_self, Some(type_param("T")));
}

#[test]
fn mismatched_type_arg_count_returns_error() {
    let error = instantiate_interface_method_signatures(
        vec![method(vec![], vec![], type_param("T"), None)],
        &["T".to_string(), "U".to_string()],
        &[native("String", Vec::new())],
    )
    .unwrap_err();

    assert_eq!(
        error,
        InterfaceMethodInstantiationError {
            expected_type_args: 2,
            actual_type_args: 1,
        }
    );
}

#[test]
fn normalizes_applied_nominal_base_and_arguments_to_exact_package_owners() {
    let mut symbols = PackageTypeSymbolIndex::default();
    symbols.insert_type("pkg.types", 0, "Box", "types.Box");
    symbols.insert_type("pkg.types", 1, "Payload", "types.Payload");
    let applied = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![TypeRefIr::LocalType { type_index: 1 }],
    };

    let normalized = normalize_package_interface_type_ref(
        "example.com/pkg",
        &symbols,
        "pkg.types",
        &applied,
        "interface method",
    )
    .unwrap();

    assert_eq!(
        normalized,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "example.com/pkg".to_string(),
                    },
                    symbol_path: "types.Box".to_string(),
                    abi_expectation: None,
                },
            },
            arguments: vec![TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "example.com/pkg".to_string(),
                    },
                    symbol_path: "types.Payload".to_string(),
                    abi_expectation: None,
                },
            }],
        }
    );
}
