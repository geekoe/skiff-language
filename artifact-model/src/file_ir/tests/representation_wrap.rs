use super::*;
use crate::{
    ExecutableKind, FunctionTypeParamIr, InterfaceInstantiationRef, LiteralIr, PackageRefIr,
    PackageSchemaTypeId, PackageSymbolRef, ServiceSymbolRef, SlotLayout,
};

fn representation_declaration(name: &str, type_params: &[&str]) -> TypeDeclIr {
    TypeDeclIr {
        name: name.to_string(),
        descriptor: TypeDescriptorIr::Representation {
            representation: if let Some(parameter) = type_params.first() {
                TypeRefIr::TypeParam {
                    name: (*parameter).to_string(),
                }
            } else {
                TypeRefIr::builtin("string")
            },
        },
        type_params: type_params
            .iter()
            .map(|parameter| (*parameter).to_string())
            .collect(),
        implements: Vec::new(),
        source_span: None,
    }
}

fn representation_unit(target: TypeRefIr) -> FileIrUnit {
    let mut unit = FileIrUnit::empty("wrap.main", "source");
    unit.type_table = vec![
        representation_declaration("Plain", &[]),
        representation_declaration("Generic", &["T"]),
        TypeDeclIr {
            name: "Record".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "Union".to_string(),
            descriptor: TypeDescriptorIr::Union {
                branches: Vec::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "Alias".to_string(),
            descriptor: TypeDescriptorIr::Alias {
                target: TypeRefIr::builtin("string"),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "Interface".to_string(),
            descriptor: TypeDescriptorIr::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ];
    unit.constants.push(ConstIr {
        name: "wrapped".to_string(),
        ty: TypeRefIr::builtin("string"),
        body: ExecutableBody {
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "payload".to_string(),
                    },
                },
                ExprIr::RepresentationWrap {
                    value: crate::ExprRefIr { expression: 0 },
                    type_ref: target,
                },
            ],
            ..ExecutableBody::default()
        },
        source_span: None,
    });
    unit
}

fn wrap_target(unit: &mut FileIrUnit) -> &mut TypeRefIr {
    let ExprIr::RepresentationWrap { type_ref, .. } = &mut unit.constants[0].body.expressions[1]
    else {
        unreachable!()
    };
    type_ref
}

#[test]
fn file_ir_admits_plain_generic_and_nested_representation_wrap_targets() {
    validate_file_ir_type_refs(&representation_unit(TypeRefIr::LocalType { type_index: 0 }))
        .unwrap();
    validate_file_ir_type_refs(&representation_unit(TypeRefIr::PublicationType {
        module_path: "wrap.main".to_string(),
        type_index: 0,
    }))
    .unwrap();
    validate_file_ir_type_refs(&representation_unit(TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::builtin("string")],
    }))
    .unwrap();
    validate_file_ir_type_refs(&representation_unit(TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
            arguments: vec![TypeRefIr::builtin("string")],
        }],
    }))
    .unwrap();
}

#[test]
fn file_ir_rejects_missing_representation_wrap_child() {
    let mut unit = representation_unit(TypeRefIr::LocalType { type_index: 0 });
    let ExprIr::RepresentationWrap { value, .. } = &mut unit.constants[0].body.expressions[1]
    else {
        unreachable!()
    };
    value.expression = 2;

    let error = validate_file_ir_type_refs(&unit).unwrap_err();
    assert!(error.location.ends_with(".value"));
    assert!(error.message.contains("does not exist"));
}

#[test]
fn file_ir_rejects_every_non_representation_and_unresolved_target_class() {
    let invalid = [
        ("record", TypeRefIr::LocalType { type_index: 2 }),
        ("union", TypeRefIr::LocalType { type_index: 3 }),
        ("alias", TypeRefIr::LocalType { type_index: 4 }),
        ("interface", TypeRefIr::LocalType { type_index: 5 }),
        ("missing local", TypeRefIr::LocalType { type_index: 99 }),
        ("primitive", TypeRefIr::builtin("string")),
        (
            "service symbol",
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "other.module".to_string(),
                    symbol: "Wrapped".to_string(),
                },
            },
        ),
        (
            "package symbol",
            TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: "model".to_string(),
                    },
                    symbol_path: "Wrapped".to_string(),
                    abi_expectation: Some("abi:model".to_string()),
                },
            },
        ),
        (
            "db object",
            TypeRefIr::DbObjectSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "wrap.main".to_string(),
                    symbol: "Stored".to_string(),
                },
            },
        ),
        (
            "structural record",
            TypeRefIr::Record {
                fields: BTreeMap::new(),
            },
        ),
        (
            "structural union",
            TypeRefIr::Union {
                items: vec![TypeRefIr::builtin("string")],
            },
        ),
        (
            "nullable",
            TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::builtin("string")),
            },
        ),
        (
            "literal",
            TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "literal".to_string(),
                },
            },
        ),
        (
            "type parameter",
            TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        ),
        (
            "interface ref",
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: "interface:wrapped".to_string(),
                    canonical_type_args: Vec::new(),
                },
            },
        ),
        (
            "function",
            TypeRefIr::Function {
                params: vec![FunctionTypeParamIr {
                    name: "value".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
                return_type: Box::new(TypeRefIr::builtin("string")),
            },
        ),
        (
            "unresolved plain publication",
            TypeRefIr::PublicationType {
                module_path: "other.module".to_string(),
                type_index: 0,
            },
        ),
        (
            "unresolved applied publication",
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PublicationType {
                    module_path: "other.module".to_string(),
                    type_index: 1,
                },
                arguments: vec![TypeRefIr::builtin("string")],
            },
        ),
        (
            "unresolved applied service symbol",
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "other.module".to_string(),
                        symbol: "Wrapped".to_string(),
                    },
                },
                arguments: vec![TypeRefIr::builtin("string")],
            },
        ),
        (
            "unresolved applied package symbol",
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::Dependency {
                            dependency_ref: "model".to_string(),
                        },
                        symbol_path: "Wrapped".to_string(),
                        abi_expectation: Some("abi:model".to_string()),
                    },
                },
                arguments: vec![TypeRefIr::builtin("string")],
            },
        ),
        (
            "plain package schema",
            TypeRefIr::PackageSchema {
                package_id: "example.com/pkg".to_string(),
                stable_schema_key: "Wrapped".to_string(),
                package_schema_type_id: PackageSchemaTypeId::new("schema:wrapped"),
            },
        ),
        (
            "applied package schema",
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSchema {
                    package_id: "example.com/pkg".to_string(),
                    stable_schema_key: "Wrapped".to_string(),
                    package_schema_type_id: PackageSchemaTypeId::new("schema:wrapped"),
                },
                arguments: vec![TypeRefIr::builtin("string")],
            },
        ),
    ];

    for (class, target) in invalid {
        let error = validate_file_ir_type_refs(&representation_unit(target)).expect_err(class);
        assert!(
            error.message.contains("representation")
                || error.message.contains("alias/interface")
                || error.message.contains("PackageSchema")
                || error.message.contains("does not exist")
                || error.message.contains("outside"),
            "{class} must fail closed through exact representation admission: {error}"
        );
    }
}

#[test]
fn file_ir_rejects_representation_wrap_wrong_arity_and_unresolved_type_param() {
    let wrong_arity = representation_unit(TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
    });
    assert!(validate_file_ir_type_refs(&wrong_arity)
        .unwrap_err()
        .message
        .contains("arity 2"));
    assert!(
        validate_file_ir_type_refs(&representation_unit(TypeRefIr::LocalType { type_index: 1 }))
            .unwrap_err()
            .message
            .contains("requires 1 type arguments")
    );

    let mut unresolved = representation_unit(TypeRefIr::LocalType { type_index: 0 });
    unresolved.constants.clear();
    unresolved.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "wrapGeneric".to_string(),
        type_params: vec!["T".to_string()],
        params: Vec::new(),
        return_type: TypeRefIr::builtin("string"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "payload".to_string(),
                    },
                },
                ExprIr::RepresentationWrap {
                    value: crate::ExprRefIr { expression: 0 },
                    type_ref: TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
                        arguments: vec![TypeRefIr::TypeParam {
                            name: "T".to_string(),
                        }],
                    },
                },
            ],
            ..ExecutableBody::default()
        },
        source_span: None,
    });
    let error = validate_file_ir_type_refs(&unresolved).unwrap_err();
    assert!(error.message.contains("unresolved type parameter T"));
}

#[test]
fn file_ir_rejects_rust_constructed_empty_applied_representation_target() {
    let mut unit = representation_unit(TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
        arguments: vec![TypeRefIr::builtin("string")],
    });
    let TypeRefIr::AppliedNominal { arguments, .. } = wrap_target(&mut unit) else {
        unreachable!()
    };
    arguments.clear();

    assert!(validate_file_ir_type_refs(&unit)
        .unwrap_err()
        .message
        .contains("non-empty"));
}
