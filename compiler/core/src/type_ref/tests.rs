use super::*;

fn param(name: &str, ty: TypeRefIr) -> FunctionTypeParamIr {
    FunctionTypeParamIr {
        name: name.to_string(),
        ty,
    }
}

fn type_param(name: &str) -> TypeRefIr {
    TypeRefIr::TypeParam {
        name: name.to_string(),
    }
}

fn native(name: &str) -> TypeRefIr {
    TypeRefIr::builtin(name)
}

fn any_interface(args: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::AnyInterface {
        interface: skiff_artifact_model::InterfaceInstantiationRef {
            interface_abi_id: "iface".to_string(),
            canonical_type_args: args,
        },
    }
}

fn applied_local(type_index: u32, arguments: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::AppliedNominal {
        base: skiff_artifact_model::NominalTypeRefBaseIr::LocalType { type_index },
        arguments,
    }
}

#[test]
fn substitutes_root_type_param() {
    let substitutions = BTreeMap::from([("T".to_string(), native("string"))]);

    assert_eq!(
        substitute_type_params_in_type_ref(type_param("T"), &substitutions),
        native("string")
    );
}

#[test]
fn substitutes_nested_type_params_in_all_structural_variants() {
    let ty = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::Record {
            fields: BTreeMap::from([
                (
                    "fn".to_string(),
                    TypeRefIr::Function {
                        params: vec![param(
                            "input",
                            TypeRefIr::Nullable {
                                inner: Box::new(type_param("T")),
                            },
                        )],
                        return_type: Box::new(type_param("U")),
                    },
                ),
                (
                    "union".to_string(),
                    TypeRefIr::Union {
                        items: vec![type_param("V"), native("null")],
                    },
                ),
            ]),
        }],
    };
    let substitutions = BTreeMap::from([
        ("T".to_string(), native("string")),
        ("U".to_string(), native("number")),
        ("V".to_string(), native("bool")),
    ]);

    let actual = substitute_type_params_in_type_ref(ty, &substitutions);

    assert_eq!(
        actual,
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::Record {
                fields: BTreeMap::from([
                    (
                        "fn".to_string(),
                        TypeRefIr::Function {
                            params: vec![param(
                                "input",
                                TypeRefIr::Nullable {
                                    inner: Box::new(native("string")),
                                }
                            )],
                            return_type: Box::new(native("number")),
                        }
                    ),
                    (
                        "union".to_string(),
                        TypeRefIr::Union {
                            items: vec![native("bool"), native("null")],
                        }
                    ),
                ]),
            }],
        }
    );
}

#[test]
fn substitution_value_is_not_substituted_again() {
    let substitutions = BTreeMap::from([
        ("T".to_string(), type_param("U")),
        ("U".to_string(), native("string")),
    ]);

    assert_eq!(
        substitute_type_params_in_type_ref(type_param("T"), &substitutions),
        type_param("U")
    );
}

#[test]
fn applied_nominal_arguments_are_walked_in_order_and_substituted() {
    let ty = applied_local(
        4,
        vec![
            type_param("T"),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![applied_local(2, vec![type_param("U")])],
            },
        ],
    );
    let substitutions = BTreeMap::from([
        ("T".to_string(), native("string")),
        ("U".to_string(), native("number")),
    ]);

    let substituted = substitute_type_params_in_type_ref_ref(&ty, &substitutions);
    assert_eq!(
        substituted,
        applied_local(
            4,
            vec![
                native("string"),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![applied_local(2, vec![native("number")])],
                },
            ],
        )
    );

    let children = type_ref_children(&ty);
    assert_eq!(children.len(), 2);
    assert_eq!(
        children[0].segment,
        TypeRefVisitPathSegment::AppliedNominalArgument { index: 0 }
    );
    assert_eq!(
        children[1].segment,
        TypeRefVisitPathSegment::AppliedNominalArgument { index: 1 }
    );
    let mut visited = Vec::new();
    walk_type_ref_with_path(&ty, &mut |visit| {
        if let TypeRefIr::TypeParam { name } = visit.ty {
            visited.push((name.clone(), visit.path.segments().to_vec()));
        }
    });
    assert_eq!(visited[0].0, "T");
    assert_eq!(
        visited[0].1,
        vec![TypeRefVisitPathSegment::AppliedNominalArgument { index: 0 }]
    );
    assert_eq!(visited[1].0, "U");
    assert_eq!(
        visited[1].1.last(),
        Some(&TypeRefVisitPathSegment::AppliedNominalArgument { index: 0 })
    );
}

#[test]
fn walk_and_any_visit_function_params_and_return_type() {
    let ty = TypeRefIr::Function {
        params: vec![param("input", type_param("P"))],
        return_type: Box::new(type_param("R")),
    };
    let mut visited = Vec::new();

    walk_type_ref(&ty, &mut |ty| {
        if let TypeRefIr::TypeParam { name } = ty {
            visited.push(name.clone());
        }
    });

    assert_eq!(visited, vec!["P".to_string(), "R".to_string()]);
    assert!(any_type_ref(&ty, &mut |ty| matches!(
        ty,
        TypeRefIr::TypeParam { name } if name == "R"
    )));
}

#[test]
fn map_type_ref_is_bottom_up_and_does_not_recurse_into_returned_value() {
    let ty = TypeRefIr::Builtin {
        name: "Box".to_string(),
        args: vec![type_param("T")],
    };
    let mut visited = Vec::new();

    let actual = map_type_ref(ty, &mut |ty| {
        match &ty {
            TypeRefIr::TypeParam { name } => visited.push(format!("param:{name}")),
            TypeRefIr::Builtin { name, .. } => visited.push(format!("native:{name}")),
            TypeRefIr::LocalType { .. } => visited.push("local".to_string()),
            TypeRefIr::PublicationType { .. } => visited.push("publication".to_string()),
            TypeRefIr::ServiceSymbol { .. } => visited.push("service".to_string()),
            TypeRefIr::PackageSymbol { .. } => visited.push("package".to_string()),
            TypeRefIr::PackageSchema { .. } => visited.push("packageSchema".to_string()),
            TypeRefIr::AppliedNominal { .. } => visited.push("appliedNominal".to_string()),
            TypeRefIr::DbObjectSymbol { .. } => visited.push("db".to_string()),
            TypeRefIr::Record { .. } => visited.push("record".to_string()),
            TypeRefIr::Union { .. } => visited.push("union".to_string()),
            TypeRefIr::Nullable { .. } => visited.push("nullable".to_string()),
            TypeRefIr::Literal { .. } => visited.push("literal".to_string()),
            TypeRefIr::AnyInterface { .. } => visited.push("anyInterface".to_string()),
            TypeRefIr::Function { .. } => visited.push("function".to_string()),
        }
        match ty {
            TypeRefIr::TypeParam { name } if name == "T" => TypeRefIr::Builtin {
                name: "Wrapper".to_string(),
                args: vec![type_param("SHOULD_NOT_VISIT")],
            },
            other => other,
        }
    });

    assert_eq!(visited, vec!["param:T", "native:Box"]);
    assert_eq!(
        actual,
        TypeRefIr::Builtin {
            name: "Box".to_string(),
            args: vec![TypeRefIr::Builtin {
                name: "Wrapper".to_string(),
                args: vec![type_param("SHOULD_NOT_VISIT")],
            }],
        }
    );
}

#[test]
fn any_interface_helpers_recurse_into_function_params_and_return_type() {
    let ty = TypeRefIr::Function {
        params: vec![param("input", native("string"))],
        return_type: Box::new(TypeRefIr::Record {
            fields: BTreeMap::from([("item".to_string(), any_interface(vec![type_param("T")]))]),
        }),
    };
    let mut visited = Vec::new();

    walk_type_ref(&ty, &mut |ty| {
        if let TypeRefIr::TypeParam { name } = ty {
            visited.push(name.clone());
        }
    });

    assert_eq!(visited, vec!["T".to_string()]);
    assert!(contains_any_interface(&ty));
    assert!(contains_boundary_unsafe_type(&ty));
    assert!(any_type_ref(&ty, &mut |ty| matches!(
        ty,
        TypeRefIr::TypeParam { name } if name == "T"
    )));
}

#[test]
fn substitution_reaches_any_interface_type_args() {
    let substitutions = BTreeMap::from([("T".to_string(), native("string"))]);

    assert_eq!(
        substitute_type_params_in_type_ref(any_interface(vec![type_param("T")]), &substitutions),
        any_interface(vec![native("string")])
    );
}

#[test]
fn walk_type_ref_with_path_reports_record_field_function_param_and_return() {
    let ty = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "handler".to_string(),
            TypeRefIr::Function {
                params: vec![param("input", type_param("P"))],
                return_type: Box::new(type_param("R")),
            },
        )]),
    };
    let mut paths = Vec::new();

    walk_type_ref_with_path(&ty, &mut |visit| {
        if let TypeRefIr::TypeParam { name } = visit.ty {
            paths.push((name.clone(), visit.path));
        }
    });

    assert_eq!(
        paths,
        vec![
            (
                "P".to_string(),
                TypeRefVisitPath::empty()
                    .child(TypeRefVisitPathSegment::RecordField {
                        name: "handler".to_string(),
                    })
                    .child(TypeRefVisitPathSegment::FunctionParam {
                        name: "input".to_string(),
                        index: 0,
                    }),
            ),
            (
                "R".to_string(),
                TypeRefVisitPath::empty()
                    .child(TypeRefVisitPathSegment::RecordField {
                        name: "handler".to_string(),
                    })
                    .child(TypeRefVisitPathSegment::FunctionReturn),
            ),
        ]
    );
    assert_eq!(
        paths[0].1.segments(),
        &[
            TypeRefVisitPathSegment::RecordField {
                name: "handler".to_string(),
            },
            TypeRefVisitPathSegment::FunctionParam {
                name: "input".to_string(),
                index: 0,
            },
        ]
    );
}

fn service_symbol(module: &str, symbol: &str) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: skiff_artifact_model::ServiceSymbolRef {
            module_path: module.to_string(),
            symbol: symbol.to_string(),
        },
    }
}

fn package_symbol(package: &str, path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: skiff_artifact_model::PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::PackageId {
                package_id: package.to_string(),
            },
            symbol_path: path.to_string(),
            abi_expectation: None,
        },
    }
}

fn package_schema_ir(package: &str, key: &str) -> TypeRefIr {
    TypeRefIr::PackageSchema {
        package_id: package.to_string(),
        stable_schema_key: key.to_string(),
        package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new("type:test"),
    }
}

fn package_schema_ref(package: &str, key: &str) -> PackageTypeRef {
    PackageTypeRef::PackageSchema {
        package_id: package.to_string(),
        stable_schema_key: key.to_string(),
        package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new("type:test"),
    }
}

fn contract_schema_ref(package: &str, key: &str) -> ContractTypeRef {
    ContractTypeRef::package_schema(
        package,
        key,
        skiff_artifact_model::PackageSchemaTypeId::new("type:test"),
    )
}

#[test]
fn debug_text_renders_all_variants() {
    assert_eq!(debug_text(&native("string")), "string");
    assert_eq!(
        debug_text(&TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![native("string")],
        }),
        "Array<string>"
    );
    assert_eq!(
        debug_text(&TypeRefIr::Nullable {
            inner: Box::new(native("string")),
        }),
        "string?"
    );
    assert_eq!(
        debug_text(&TypeRefIr::Union {
            items: vec![native("string"), native("number")],
        }),
        "string | number"
    );
    assert_eq!(
        debug_text(&TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::String {
                value: "a\nb".to_string(),
            },
        }),
        "\"a\\nb\""
    );
    assert_eq!(
        debug_text(&TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::Null,
        }),
        "null"
    );
    assert_eq!(
        debug_text(&TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::Number {
                value: serde_json::Number::from(1),
            },
        }),
        "<literal>"
    );
    assert_eq!(debug_text(&TypeRefIr::LocalType { type_index: 3 }), "#3");
    assert_eq!(
        debug_text(&TypeRefIr::PublicationType {
            module_path: "m".to_string(),
            type_index: 2,
        }),
        "m#2"
    );
    assert_eq!(debug_text(&service_symbol("mod", "Sym")), "mod.Sym");
    assert_eq!(
        debug_text(&TypeRefIr::DbObjectSymbol {
            symbol: skiff_artifact_model::ServiceSymbolRef {
                module_path: "mod".to_string(),
                symbol: "Sym".to_string(),
            },
        }),
        "mod.Sym"
    );
    assert_eq!(debug_text(&package_symbol("pkg", "pkg.Sym")), "pkg.Sym");
    assert_eq!(debug_text(&package_schema_ir("pkg", "key")), "pkg::key");
    assert_eq!(
        debug_text(&TypeRefIr::Record {
            fields: BTreeMap::from([("a".to_string(), native("string"))]),
        }),
        "{}"
    );
    assert_eq!(debug_text(&type_param("T")), "T");
    assert_eq!(
        debug_text(&TypeRefIr::Function {
            params: vec![param("input", native("string"))],
            return_type: Box::new(native("void")),
        }),
        "fn"
    );
}

#[test]
fn debug_text_formats_every_applied_nominal_base() {
    let argument = native("string");
    let cases = [
        (
            skiff_artifact_model::NominalTypeRefBaseIr::LocalType { type_index: 3 },
            "#3<string>",
        ),
        (
            skiff_artifact_model::NominalTypeRefBaseIr::PublicationType {
                module_path: "m".to_string(),
                type_index: 2,
            },
            "m#2<string>",
        ),
        (
            skiff_artifact_model::NominalTypeRefBaseIr::ServiceSymbol {
                symbol: skiff_artifact_model::ServiceSymbolRef {
                    module_path: "mod".to_string(),
                    symbol: "Sym".to_string(),
                },
            },
            "mod.Sym<string>",
        ),
        (
            skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol {
                symbol: skiff_artifact_model::PackageSymbolRef {
                    package: skiff_artifact_model::PackageRefIr::PackageId {
                        package_id: "pkg".to_string(),
                    },
                    symbol_path: "pkg.Sym".to_string(),
                    abi_expectation: None,
                },
            },
            "pkg.Sym<string>",
        ),
        (
            skiff_artifact_model::NominalTypeRefBaseIr::PackageSchema {
                package_id: "pkg".to_string(),
                stable_schema_key: "key".to_string(),
                package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new("type:test"),
            },
            "pkg::key<string>",
        ),
    ];
    for (base, expected) in cases {
        assert_eq!(
            debug_text(&TypeRefIr::AppliedNominal {
                base,
                arguments: vec![argument.clone()],
            }),
            expected
        );
    }
}

#[test]
fn debug_text_renders_any_interface_from_identity_or_raw_text() {
    let parsed = TypeRefIr::AnyInterface {
        interface: skiff_artifact_model::InterfaceInstantiationRef {
            interface_abi_id: serde_json::to_string(&native("Iface")).unwrap(),
            canonical_type_args: Vec::new(),
        },
    };
    assert_eq!(debug_text(&parsed), "any Iface");

    let with_args = TypeRefIr::AnyInterface {
        interface: skiff_artifact_model::InterfaceInstantiationRef {
            interface_abi_id: serde_json::to_string(&native("Iface")).unwrap(),
            canonical_type_args: vec![native("string"), native("number")],
        },
    };
    assert_eq!(debug_text(&with_args), "any Iface<string, number>");

    let raw = TypeRefIr::AnyInterface {
        interface: skiff_artifact_model::InterfaceInstantiationRef {
            interface_abi_id: "raw-identity".to_string(),
            canonical_type_args: Vec::new(),
        },
    };
    assert_eq!(debug_text(&raw), "any raw-identity");
}

#[test]
fn record_field_type_resolves_fields_unions_and_native_shapes() {
    let record = TypeRefIr::Record {
        fields: BTreeMap::from([
            ("a".to_string(), native("string")),
            ("b".to_string(), native("number")),
        ]),
    };
    assert_eq!(record_field_type(&record, "a"), Some(native("string")));
    assert_eq!(record_field_type(&record, "b"), Some(native("number")));
    assert_eq!(record_field_type(&record, "missing"), None);

    let union = TypeRefIr::Union {
        items: vec![
            TypeRefIr::Record {
                fields: BTreeMap::from([("a".to_string(), native("string"))]),
            },
            TypeRefIr::Record {
                fields: BTreeMap::from([("a".to_string(), native("number"))]),
            },
        ],
    };
    assert_eq!(
        record_field_type(&union, "a"),
        Some(TypeRefIr::Union {
            items: vec![native("number"), native("string")],
        })
    );
    assert_eq!(record_field_type(&union, "missing"), None);

    let catch_result = TypeRefIr::Builtin {
        name: "CatchResult".to_string(),
        args: vec![native("string"), native("number")],
    };
    assert_eq!(
        record_field_type(&catch_result, "tag"),
        Some(TypeRefIr::Union {
            items: vec![
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "err".to_string(),
                    },
                },
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "ok".to_string(),
                    },
                },
            ],
        })
    );
    assert_eq!(record_field_type(&catch_result, "other"), None);

    let upsert = TypeRefIr::Builtin {
        name: "DbUpsertResult".to_string(),
        args: vec![native("string")],
    };
    assert_eq!(record_field_type(&upsert, "inserted"), Some(native("bool")));
    assert_eq!(record_field_type(&upsert, "value"), Some(native("string")));
    assert_eq!(record_field_type(&upsert, "other"), None);

    let exception = TypeRefIr::Builtin {
        name: "Exception".to_string(),
        args: vec![native("string")],
    };
    assert_eq!(
        record_field_type(&exception, "error"),
        Some(native("string"))
    );
    assert_eq!(record_field_type(&exception, "other"), None);

    assert_eq!(record_field_type(&native("string"), "a"), None);
}

#[test]
fn normalize_union_flattens_folds_null_sorts_and_dedups() {
    let nested = TypeRefIr::Union {
        items: vec![
            TypeRefIr::Union {
                items: vec![native("string"), native("number")],
            },
            native("number"),
        ],
    };
    assert_eq!(
        normalize_union(nested),
        TypeRefIr::Union {
            items: vec![native("number"), native("string")],
        }
    );

    assert_eq!(
        normalize_union(TypeRefIr::Union {
            items: vec![
                native("string"),
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::Null,
                }
            ],
        }),
        TypeRefIr::Nullable {
            inner: Box::new(native("string")),
        }
    );

    assert_eq!(
        normalize_union(TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::Union {
                items: vec![native("string"), native("number")],
            }),
        }),
        TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::Union {
                items: vec![native("number"), native("string")],
            }),
        }
    );

    assert_eq!(
        normalize_union(TypeRefIr::Union { items: Vec::new() }),
        TypeRefIr::Union { items: Vec::new() }
    );

    assert_eq!(
        normalize_union(TypeRefIr::Union {
            items: vec![
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::Null,
                },
                native("null"),
            ],
        }),
        native("null")
    );

    assert_eq!(
        normalize_union(TypeRefIr::Union {
            items: vec![native("string"), native("string")],
        }),
        native("string")
    );

    assert_eq!(
        normalize_union(TypeRefIr::Record {
            fields: BTreeMap::from([(
                "field".to_string(),
                TypeRefIr::Union {
                    items: vec![
                        native("string"),
                        TypeRefIr::Literal {
                            value: skiff_artifact_model::LiteralIr::Null,
                        }
                    ],
                },
            )]),
        }),
        TypeRefIr::Record {
            fields: BTreeMap::from([(
                "field".to_string(),
                TypeRefIr::Nullable {
                    inner: Box::new(native("string")),
                },
            )]),
        }
    );
}

#[test]
fn single_item_returns_container_item_or_map_key() {
    for name in [
        "Array",
        "Stream",
        "std.collection.Array",
        "std.stream.Stream",
    ] {
        assert_eq!(
            single_item(&TypeRefIr::Builtin {
                name: name.to_string(),
                args: vec![native("string")],
            }),
            Some(&native("string"))
        );
    }
    for name in ["Map", "std.collection.Map"] {
        assert_eq!(
            single_item(&TypeRefIr::Builtin {
                name: name.to_string(),
                args: vec![native("string"), native("number")],
            }),
            Some(&native("string"))
        );
    }
    assert_eq!(
        single_item(&TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![native("string"), native("number")],
        }),
        None
    );
    assert_eq!(
        single_item(&TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![native("string")],
        }),
        None
    );
    assert_eq!(
        single_item(&TypeRefIr::Record {
            fields: BTreeMap::new(),
        }),
        None
    );
}

#[test]
fn map_entry_returns_key_and_value_for_map_shape() {
    for name in ["Map", "std.collection.Map"] {
        assert_eq!(
            map_entry(&TypeRefIr::Builtin {
                name: name.to_string(),
                args: vec![native("string"), native("number")],
            }),
            Some((&native("string"), &native("number")))
        );
    }
    assert_eq!(
        map_entry(&TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![native("string")],
        }),
        None
    );
    assert_eq!(map_entry(&native("string")), None);
}

#[test]
fn exception_payload_requires_exactly_one_argument() {
    assert_eq!(
        exception_payload(&TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: vec![native("string")],
        }),
        Some(&native("string"))
    );
    assert_eq!(
        exception_payload(&TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: Vec::new(),
        }),
        None
    );
    assert_eq!(
        exception_payload(&TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: vec![native("string"), native("number")],
        }),
        None
    );
    assert_eq!(exception_payload(&native("string")), None);
}

#[test]
fn catch_result_branches_returns_union_record_and_catch_result_branches() {
    let union = TypeRefIr::Union {
        items: vec![native("string"), native("number")],
    };
    assert_eq!(
        catch_result_branches(&union),
        Some(vec![native("string"), native("number")])
    );

    let record = TypeRefIr::Record {
        fields: BTreeMap::from([("a".to_string(), native("string"))]),
    };
    assert_eq!(catch_result_branches(&record), Some(vec![record.clone()]));

    let catch_result = TypeRefIr::Builtin {
        name: "CatchResult".to_string(),
        args: vec![native("string"), native("number")],
    };
    assert_eq!(
        catch_result_branches(&catch_result),
        Some(vec![
            TypeRefIr::Record {
                fields: BTreeMap::from([
                    (
                        "tag".to_string(),
                        TypeRefIr::Literal {
                            value: skiff_artifact_model::LiteralIr::String {
                                value: "ok".to_string(),
                            },
                        },
                    ),
                    ("value".to_string(), native("string")),
                ]),
            },
            TypeRefIr::Record {
                fields: BTreeMap::from([
                    (
                        "tag".to_string(),
                        TypeRefIr::Literal {
                            value: skiff_artifact_model::LiteralIr::String {
                                value: "err".to_string(),
                            },
                        },
                    ),
                    (
                        "exception".to_string(),
                        TypeRefIr::Builtin {
                            name: "Exception".to_string(),
                            args: vec![native("number")],
                        },
                    ),
                ]),
            },
        ])
    );
    assert_eq!(catch_result_branches(&native("string")), None);
}

#[test]
fn is_null_type_matches_builtin_and_null_literal() {
    assert!(is_null_type(&native("null")));
    assert!(is_null_type(&TypeRefIr::Literal {
        value: skiff_artifact_model::LiteralIr::Null,
    }));
    assert!(!is_null_type(&native("string")));
    assert!(!is_null_type(&TypeRefIr::Literal {
        value: skiff_artifact_model::LiteralIr::String {
            value: "null".to_string(),
        },
    }));
}

#[test]
fn contains_type_param_recurse_across_all_structural_variants() {
    let cases = vec![
        type_param("T"),
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![type_param("T")],
        },
        TypeRefIr::Union {
            items: vec![type_param("T")],
        },
        TypeRefIr::AppliedNominal {
            base: skiff_artifact_model::NominalTypeRefBaseIr::LocalType { type_index: 0 },
            arguments: vec![type_param("T")],
        },
        TypeRefIr::Nullable {
            inner: Box::new(type_param("T")),
        },
        TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: "iface".to_string(),
                canonical_type_args: vec![type_param("T")],
            },
        },
        TypeRefIr::Record {
            fields: BTreeMap::from([("a".to_string(), type_param("T"))]),
        },
        TypeRefIr::Function {
            params: vec![param("input", type_param("T"))],
            return_type: Box::new(native("void")),
        },
    ];
    for ty in cases {
        assert!(contains_type_param(&ty), "expected type param in {ty:?}");
    }

    assert!(!contains_type_param(&native("string")));
    assert!(!contains_type_param(&TypeRefIr::Record {
        fields: BTreeMap::from([("a".to_string(), native("string"))]),
    }));
    assert!(!contains_type_param(&package_symbol("pkg", "pkg.Sym")));
}

#[test]
fn builtin_shape_of_name_maps_all_shapes_and_std_full_names() {
    let expected = [
        ("Array", BuiltinShape::Array),
        ("Stream", BuiltinShape::Stream),
        ("Map", BuiltinShape::Map),
        ("Exception", BuiltinShape::Exception),
        ("CatchResult", BuiltinShape::CatchResult),
        ("DbUpsertResult", BuiltinShape::DbUpsertResult),
        ("Json", BuiltinShape::Json),
        ("JsonObject", BuiltinShape::JsonObject),
        ("null", BuiltinShape::Null),
        ("void", BuiltinShape::Void),
        ("never", BuiltinShape::Never),
        ("unknown", BuiltinShape::Unknown),
        ("string", BuiltinShape::String),
        ("integer", BuiltinShape::Integer),
        ("number", BuiltinShape::Number),
        ("bool", BuiltinShape::Bool),
    ];
    for (name, shape) in expected {
        assert_eq!(BuiltinShape::of_name(name), Some(shape));
    }
    assert_eq!(
        BuiltinShape::of_name("std.collection.Array"),
        Some(BuiltinShape::Array)
    );
    assert_eq!(
        BuiltinShape::of_name("std.stream.Stream"),
        Some(BuiltinShape::Stream)
    );
    assert_eq!(
        BuiltinShape::of_name("std.collection.Map"),
        Some(BuiltinShape::Map)
    );
    assert_eq!(BuiltinShape::of_name("String"), None);
    assert_eq!(BuiltinShape::of_name("not-a-builtin"), None);
}

#[test]
fn package_type_ref_to_ir_folds_schema_and_keeps_local_verbatim() {
    let local_with_schema = PackageTypeRef::Local {
        local_type: package_schema_ir("pkg", "key"),
    };
    assert_eq!(
        package_type_ref_to_ir(&local_with_schema),
        package_schema_ir("pkg", "key")
    );
    assert_eq!(
        package_type_ref_to_ir(&PackageTypeRef::Local {
            local_type: native("string"),
        }),
        native("string")
    );

    let schema = package_schema_ref("pkg", "key");
    assert_eq!(
        package_type_ref_to_ir(&schema),
        package_symbol("pkg", "key")
    );

    let container = PackageTypeRef::Container {
        name: "Array".to_string(),
        arguments: vec![PackageTypeRef::Container {
            name: "string".to_string(),
            arguments: Vec::new(),
        }],
    };
    assert_eq!(
        package_type_ref_to_ir(&container),
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![native("string")],
        }
    );

    let nullable = PackageTypeRef::Nullable {
        inner: Box::new(PackageTypeRef::Container {
            name: "string".to_string(),
            arguments: Vec::new(),
        }),
    };
    assert_eq!(
        package_type_ref_to_ir(&nullable),
        TypeRefIr::Nullable {
            inner: Box::new(native("string")),
        }
    );
}

#[test]
fn package_type_ref_to_ir_uses_serde_json_interface_identity() {
    let interface = PackageTypeRef::Container {
        name: "Iface".to_string(),
        arguments: Vec::new(),
    };
    let any_interface = PackageTypeRef::AnyInterface {
        interface: Box::new(interface),
        arguments: vec![PackageTypeRef::Container {
            name: "string".to_string(),
            arguments: Vec::new(),
        }],
    };
    let ir = package_type_ref_to_ir(&any_interface);
    let TypeRefIr::AnyInterface { interface, .. } = ir else {
        panic!("expected AnyInterface");
    };
    assert_eq!(
        interface.interface_abi_id,
        serde_json::to_string(&native("Iface")).unwrap()
    );
    assert_eq!(interface.canonical_type_args, vec![native("string")]);
}

#[test]
fn package_type_ref_to_ir_exact_preserves_schema_and_uses_canonical_identity() {
    let schema = package_schema_ref("pkg", "key");
    assert_eq!(
        package_type_ref_to_ir_exact(&schema),
        package_schema_ir("pkg", "key")
    );

    let local_with_schema = PackageTypeRef::Local {
        local_type: package_schema_ir("pkg", "key"),
    };
    assert_eq!(
        package_type_ref_to_ir_exact(&local_with_schema),
        package_schema_ir("pkg", "key")
    );

    let interface = PackageTypeRef::Container {
        name: "Iface".to_string(),
        arguments: vec![PackageTypeRef::Container {
            name: "string".to_string(),
            arguments: Vec::new(),
        }],
    };
    let any_interface = PackageTypeRef::AnyInterface {
        interface: Box::new(interface),
        arguments: Vec::new(),
    };
    let exact = package_type_ref_to_ir_exact(&any_interface);
    let folded = package_type_ref_to_ir(&any_interface);
    let TypeRefIr::AnyInterface { interface, .. } = &exact else {
        panic!("expected AnyInterface");
    };
    let expected_key = String::from_utf8(
        skiff_canonical_json::canonical_json_bytes(&TypeRefIr::Builtin {
            name: "Iface".to_string(),
            args: vec![native("string")],
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(interface.interface_abi_id, expected_key);
    assert_ne!(exact, folded);
}

#[test]
fn contract_type_ref_to_ir_projects_all_variants() {
    let builtin = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    assert_eq!(
        contract_type_ref_to_ir(&builtin),
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![native("string")],
        }
    );

    assert_eq!(
        contract_type_ref_to_ir(&contract_schema_ref("pkg", "key")),
        package_symbol("pkg", "key")
    );

    let any_interface = ContractTypeRef::AnyInterface {
        interface: Box::new(ContractTypeRef::builtin("Iface")),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    let TypeRefIr::AnyInterface { interface, .. } = contract_type_ref_to_ir(&any_interface) else {
        panic!("expected AnyInterface");
    };
    assert_eq!(
        interface.interface_abi_id,
        serde_json::to_string(&native("Iface")).unwrap()
    );
    assert_eq!(interface.canonical_type_args, vec![native("string")]);

    assert_eq!(
        contract_type_ref_to_ir(&ContractTypeRef::TypeParam {
            name: "T".to_string(),
        }),
        type_param("T")
    );
    assert_eq!(
        contract_type_ref_to_ir(&ContractTypeRef::Record {
            fields: BTreeMap::from([("a".to_string(), ContractTypeRef::builtin("string"))]),
        }),
        TypeRefIr::Record {
            fields: BTreeMap::from([("a".to_string(), native("string"))]),
        }
    );
    assert_eq!(
        contract_type_ref_to_ir(&ContractTypeRef::structural_union(vec![
            ContractTypeRef::builtin("string"),
            ContractTypeRef::builtin("number"),
        ])),
        TypeRefIr::Union {
            items: vec![native("string"), native("number")],
        }
    );
    assert_eq!(
        contract_type_ref_to_ir(&ContractTypeRef::Nullable {
            inner: Box::new(ContractTypeRef::builtin("string")),
        }),
        TypeRefIr::Nullable {
            inner: Box::new(native("string")),
        }
    );
    assert_eq!(
        contract_type_ref_to_ir(&ContractTypeRef::string_literal("x")),
        TypeRefIr::Literal {
            value: skiff_artifact_model::LiteralIr::String {
                value: "x".to_string(),
            },
        }
    );
}
