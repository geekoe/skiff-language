use super::*;
use skiff_artifact_model::{
    CallableEffectSummary, PackageCallableParameter, PackageSchemaTypeId, ParamModeIr, TypeDeclIr,
    TypeDeclarationIr, TypeLinkTargetIr,
};

fn fixture() -> (Vec<FileIrUnit>, BTreeMap<(String, String), ContractTypeRef>) {
    let mut unit = FileIrUnit::empty("api", "source-hash");
    unit.type_table
        .extend(
            ["PublicError", "LocalHandle", "PrivateDetail"].map(|name| TypeDeclIr {
                name: name.into(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            }),
        );
    unit.declarations.types.insert(
        "PublicError".into(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "PublicError".into(),
            source_span: None,
        },
    );
    unit.declarations.types.insert(
        "LocalHandle".into(),
        TypeDeclarationIr {
            type_index: 1,
            symbol: "LocalHandle".into(),
            source_span: None,
        },
    );
    unit.declarations.types.insert(
        "PrivateDetail".into(),
        TypeDeclarationIr {
            type_index: 2,
            symbol: "PrivateDetail".into(),
            source_span: None,
        },
    );
    unit.link_targets
        .types
        .insert("PublicError".into(), TypeLinkTargetIr { type_index: 0 });
    unit.link_targets
        .types
        .insert("LocalHandle".into(), TypeLinkTargetIr { type_index: 1 });
    let exact = ContractTypeRef::package_schema(
        "example.pkg",
        "errors.PublicError",
        PackageSchemaTypeId::new("schema:public-error"),
    );
    (
        vec![unit],
        BTreeMap::from([(("api".into(), "PublicError".into()), exact)]),
    )
}

#[test]
fn public_nominals_are_exact_through_parameters_and_return() {
    let (units, refs) = fixture();
    let nested = PackageTypeRef::Local {
        local_type: TypeRefIr::Builtin {
            name: "Array".into(),
            args: vec![TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::LocalType { type_index: 0 }),
            }],
        },
    };
    let mut signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "values".into(),
            ty: nested,
            mode: ParamModeIr::Value,
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::PublicationType {
                module_path: "api".into(),
                type_index: 0,
            },
        },
        may_suspend: false,
    };

    normalize_public_signature("api", &mut signature, &units, &refs, &[]).unwrap();

    let exact = PackageTypeRef::PackageSchema {
        package_id: "example.pkg".into(),
        stable_schema_key: "errors.PublicError".into(),
        package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
    };
    assert_eq!(signature.return_type, exact);
    assert_eq!(
        signature.parameters[0].ty,
        PackageTypeRef::Container {
            name: "Array".into(),
            arguments: vec![PackageTypeRef::Nullable {
                inner: Box::new(exact),
            }],
        }
    );
}

#[test]
fn private_or_unresolved_local_nominal_is_rejected() {
    let (units, refs) = fixture();
    let private = PackageTypeRef::Local {
        local_type: TypeRefIr::LocalType { type_index: 2 },
    };
    let error = normalize_package_type("api", &private, &units, &refs, &[]).unwrap_err();
    assert!(
        error.contains("PrivateDetail") && error.contains("private or nonexported"),
        "{error}"
    );
}

#[test]
fn package_schema_promotion_precedes_service_symbol_export_validation() {
    let (mut units, refs) = fixture();
    units[0].link_targets.types.remove("PublicError");
    assert_eq!(
        normalize_package_type(
            "api",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index: 0 },
            },
            &units,
            &refs,
            &[],
        )
        .unwrap(),
        PackageTypeRef::PackageSchema {
            package_id: "example.pkg".into(),
            stable_schema_key: "errors.PublicError".into(),
            package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
        }
    );
}

#[test]
fn public_signature_normalization_preserves_applied_wrapper_and_normalizes_arguments() {
    let (units, refs) = fixture();
    let applied = PackageTypeRef::Local {
        local_type: TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
            arguments: vec![TypeRefIr::LocalType { type_index: 0 }],
        },
    };

    assert_eq!(
        normalize_package_type("api", &applied, &units, &refs, &[]).unwrap(),
        PackageTypeRef::Local {
            local_type: TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "api".to_string(),
                        symbol: "LocalHandle".to_string(),
                    },
                },
                arguments: vec![TypeRefIr::PackageSchema {
                    package_id: "example.pkg".into(),
                    stable_schema_key: "errors.PublicError".into(),
                    package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
                }],
            },
        }
    );
}

#[test]
fn public_signature_normalization_covers_every_nested_package_and_local_shape() {
    let (units, refs) = fixture();
    let local_handle = TypeRefIr::LocalType { type_index: 1 };
    let schema_type = TypeRefIr::LocalType { type_index: 0 };
    let inner_any_interface = TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: type_ref_abi_key(&local_handle),
            canonical_type_args: vec![schema_type.clone()],
        },
    };
    let nested_function = TypeRefIr::Function {
        params: vec![
            FunctionTypeParamIr {
                name: "builtin".into(),
                ty: TypeRefIr::Builtin {
                    name: "Array".into(),
                    args: vec![local_handle.clone()],
                },
            },
            FunctionTypeParamIr {
                name: "record".into(),
                ty: TypeRefIr::Record {
                    fields: BTreeMap::from([(
                        "choice".into(),
                        TypeRefIr::Union {
                            items: vec![
                                TypeRefIr::PublicationType {
                                    module_path: "api".into(),
                                    type_index: 1,
                                },
                                TypeRefIr::Nullable {
                                    inner: Box::new(schema_type.clone()),
                                },
                            ],
                        },
                    )]),
                },
            },
            FunctionTypeParamIr {
                name: "applied".into(),
                ty: TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
                    arguments: vec![schema_type],
                },
            },
            FunctionTypeParamIr {
                name: "existential".into(),
                ty: inner_any_interface,
            },
        ],
        return_type: Box::new(local_handle.clone()),
    };
    let mut signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![
            PackageCallableParameter {
                name: "direct".into(),
                ty: PackageTypeRef::Local {
                    local_type: local_handle.clone(),
                },
                mode: ParamModeIr::InOut,
            },
            PackageCallableParameter {
                name: "nested".into(),
                ty: PackageTypeRef::Container {
                    name: "Envelope".into(),
                    arguments: vec![PackageTypeRef::Nullable {
                        inner: Box::new(PackageTypeRef::AnyInterface {
                            interface: Box::new(PackageTypeRef::Local {
                                local_type: local_handle.clone(),
                            }),
                            arguments: vec![PackageTypeRef::Local {
                                local_type: nested_function,
                            }],
                        }),
                    }],
                },
                mode: ParamModeIr::Value,
            },
        ],
        return_type: PackageTypeRef::Local {
            local_type: local_handle,
        },
        may_suspend: false,
    };

    normalize_public_signature("api", &mut signature, &units, &refs, &[]).unwrap();

    let value = serde_json::to_value(&signature).unwrap();
    assert_eq!(count_json_kind(&value, "localType"), 0);
    assert_eq!(count_json_kind(&value, "publicationType"), 0);
    for required in [
        "container",
        "nullable",
        "anyInterface",
        "builtin",
        "record",
        "union",
        "function",
        "appliedNominal",
        "serviceSymbol",
        "packageSchema",
    ] {
        assert!(
            count_json_kind(&value, required) > 0,
            "normalized signature lost required `{required}` shape: {value}"
        );
    }
    let exact_handle = PackageTypeRef::Local {
        local_type: TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "api".into(),
                symbol: "LocalHandle".into(),
            },
        },
    };
    assert_eq!(signature.parameters[0].ty, exact_handle);
    assert_eq!(signature.parameters[0].mode, ParamModeIr::InOut);
    assert_eq!(signature.parameters[1].mode, ParamModeIr::Value);
    assert_eq!(signature.return_type, exact_handle);
}

#[test]
fn public_signature_uses_source_module_slots_not_public_display_paths() {
    let (mut units, mut refs) = fixture();
    let mut display = FileIrUnit::empty("public.api", "display-source-hash");
    display.type_table.extend([
        TypeDeclIr {
            name: "Unused".into(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "DisplayHandle".into(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ]);
    display
        .link_targets
        .types
        .insert("DisplayHandle".into(), TypeLinkTargetIr { type_index: 1 });
    units.push(display);
    refs.insert(
        ("public.api".into(), "DisplayHandle".into()),
        ContractTypeRef::package_schema(
            "wrong.pkg",
            "DisplayHandle",
            PackageSchemaTypeId::new("schema:wrong-display"),
        ),
    );

    assert_eq!(
        normalize_package_type(
            "api",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index: 1 },
            },
            &units,
            &refs,
            &[],
        )
        .unwrap(),
        PackageTypeRef::Local {
            local_type: TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "api".into(),
                    symbol: "LocalHandle".into(),
                },
            },
        }
    );

    refs.remove(&("public.api".into(), "DisplayHandle".into()));
    assert_eq!(
        normalize_package_type(
            "api",
            &PackageTypeRef::Local {
                local_type: TypeRefIr::PublicationType {
                    module_path: "public.api".into(),
                    type_index: 1,
                },
            },
            &units,
            &refs,
            &[],
        )
        .unwrap(),
        PackageTypeRef::Local {
            local_type: TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "public.api".into(),
                    symbol: "DisplayHandle".into(),
                },
            },
        }
    );
}

#[test]
fn public_signature_owner_resolution_failures_are_closed() {
    let (units, refs) = fixture();
    let normalize = |owner_module: &str, type_index: u32, units: &[FileIrUnit]| {
        normalize_package_type(
            owner_module,
            &PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index },
            },
            units,
            &refs,
            &[],
        )
        .unwrap_err()
    };

    let missing_module = normalize("wrong.owner", 1, &units);
    assert!(
        missing_module.contains("has no source module"),
        "{missing_module}"
    );
    let missing_slot = normalize("api", 99, &units);
    assert!(
        missing_slot.contains("has no type-table entry"),
        "{missing_slot}"
    );

    let mut missing_symbol = units.clone();
    missing_symbol[0].type_table[1].name = "MissingSymbol".into();
    let missing_symbol = normalize("api", 1, &missing_symbol);
    assert!(
        missing_symbol.contains("MissingSymbol")
            && missing_symbol.contains("private or nonexported"),
        "{missing_symbol}"
    );

    let mut wrong_slot = units.clone();
    wrong_slot[0]
        .link_targets
        .types
        .get_mut("LocalHandle")
        .unwrap()
        .type_index = 0;
    let wrong_slot = normalize("api", 1, &wrong_slot);
    assert!(
        wrong_slot.contains("wrong exported owner slot"),
        "{wrong_slot}"
    );

    let mut ambiguous = units.clone();
    ambiguous.push(units[0].clone());
    let ambiguous = normalize("api", 1, &ambiguous);
    assert!(
        ambiguous.contains("ambiguous source modules"),
        "{ambiguous}"
    );
}

#[test]
fn implementation_normalization_preserves_applied_owner_and_ordered_arguments() {
    let mut unit = FileIrUnit::empty("api", "source-hash");
    unit.declarations.types.insert(
        "Box".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "api.Box".to_string(),
            source_span: None,
        },
    );
    let units = vec![unit];
    let applied = |argument| TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 0 },
        arguments: vec![argument],
    };

    let string_box = normalize_implementation_type(
        "example.pkg",
        "api",
        &applied(TypeRefIr::builtin("string")),
        &units,
    )
    .unwrap();
    let number_box = normalize_implementation_type(
        "example.pkg",
        "api",
        &applied(TypeRefIr::builtin("number")),
        &units,
    )
    .unwrap();

    assert_ne!(string_box, number_box);
    assert_ne!(
        skiff_artifact_identity::type_ref_abi_key(&string_box),
        skiff_artifact_identity::type_ref_abi_key(&number_box)
    );
    assert_eq!(
        string_box,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "example.pkg".to_string(),
                    },
                    symbol_path: "api.Box".to_string(),
                    abi_expectation: None,
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        }
    );
}

#[test]
fn same_symbol_path_from_distinct_package_owners_does_not_merge() {
    let applied = |package_id: &str| TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path: "models.Box".to_string(),
                abi_expectation: Some("abi:shared".to_string()),
            },
        },
        arguments: vec![TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: format!("{package_id}/nested"),
                    },
                    symbol_path: "models.Value".to_string(),
                    abi_expectation: Some("abi:nested-shared".to_string()),
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        }],
    };

    let first =
        normalize_implementation_type("consumer", "api", &applied("example.one"), &[]).unwrap();
    let second =
        normalize_implementation_type("consumer", "api", &applied("example.two"), &[]).unwrap();

    assert_ne!(first, second);
    assert_eq!(first, applied("example.one"));
    assert_eq!(second, applied("example.two"));
}

#[test]
fn reachable_and_direct_return_origins_are_normalized_independently() {
    let field = ValueProvenance::CallerParameterProjection {
        index: 1,
        path: skiff_artifact_model::ValueProjectionPath::field("payload").unwrap(),
    };
    let element = ValueProvenance::CallerParameterProjection {
        index: 1,
        path: skiff_artifact_model::ValueProjectionPath::container_element(),
    };
    let mut facts = CallableSemanticFacts {
        effects: CallableEffectSummary::analysis_pending(),
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: vec![
                field.clone(),
                ValueProvenance::Fresh,
                field.clone(),
                element.clone(),
            ],
            direct_return_origins: vec![
                ValueProvenance::DependencyReturn {
                    callable_id: "pkg-callable:z".into(),
                },
                ValueProvenance::Constant,
                element.clone(),
                ValueProvenance::Fresh,
                ValueProvenance::Constant,
            ],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
    };

    facts = normalize_semantic_facts(facts);
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        direct_return_origins,
        ..
    } = facts.provenance
    else {
        panic!("fixture provenance must remain analyzed")
    };
    assert_eq!(
        return_origins,
        vec![ValueProvenance::Fresh, element.clone(), field]
    );
    assert_eq!(
        direct_return_origins,
        vec![
            ValueProvenance::Fresh,
            ValueProvenance::Constant,
            element,
            ValueProvenance::DependencyReturn {
                callable_id: "pkg-callable:z".into(),
            },
        ]
    );
}

fn count_json_kind(value: &serde_json::Value, expected: &str) -> usize {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| count_json_kind(item, expected))
            .sum(),
        serde_json::Value::Object(fields) => {
            usize::from(fields.get("kind").and_then(serde_json::Value::as_str) == Some(expected))
                + fields
                    .values()
                    .map(|field| count_json_kind(field, expected))
                    .sum::<usize>()
        }
        _ => 0,
    }
}
