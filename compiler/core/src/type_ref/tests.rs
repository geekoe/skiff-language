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
