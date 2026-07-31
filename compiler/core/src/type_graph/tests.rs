use std::collections::BTreeMap;

use skiff_artifact_model::{FunctionTypeParamIr, InterfaceInstantiationRef};

use super::*;
use crate::type_ref::{TypeRefVisitPath, TypeRefVisitPathSegment};

fn param(name: &str, ty: TypeRefIr) -> FunctionTypeParamIr {
    FunctionTypeParamIr {
        name: name.to_string(),
        ty,
    }
}

fn native(name: &str) -> TypeRefIr {
    TypeRefIr::builtin(name)
}

#[test]
fn type_graph_records_nested_paths_and_node_kinds() {
    let ty = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "items".to_string(),
            TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::Union {
                    items: vec![
                        TypeRefIr::AnyInterface {
                            interface: InterfaceInstantiationRef {
                                interface_abi_id: "iface".to_string(),
                                canonical_type_args: vec![native("string")],
                            },
                        },
                        TypeRefIr::Function {
                            params: vec![param("input", native("number"))],
                            return_type: Box::new(TypeRefIr::TypeParam {
                                name: "T".to_string(),
                            }),
                        },
                    ],
                }),
            },
        )]),
    };

    let analysis = TypeGraphAnalyzer::new().analyze(&ty);

    assert!(analysis.nodes.iter().any(|node| {
        node.kind == TypeGraphNodeKind::AnyInterface
            && node.path
                == TypeRefVisitPath::empty()
                    .child(TypeRefVisitPathSegment::RecordField {
                        name: "items".to_string(),
                    })
                    .child(TypeRefVisitPathSegment::NullableInner)
                    .child(TypeRefVisitPathSegment::UnionItem { index: 0 })
    }));
    assert!(analysis.nodes.iter().any(|node| {
        node.kind == TypeGraphNodeKind::TypeParam
            && node.path
                == TypeRefVisitPath::empty()
                    .child(TypeRefVisitPathSegment::RecordField {
                        name: "items".to_string(),
                    })
                    .child(TypeRefVisitPathSegment::NullableInner)
                    .child(TypeRefVisitPathSegment::UnionItem { index: 1 })
                    .child(TypeRefVisitPathSegment::FunctionReturn)
    }));
}

#[test]
fn type_graph_summary_facts_mark_non_plain_schema_shapes() {
    let ty = TypeRefIr::Record {
        fields: BTreeMap::from([
            ("local".to_string(), TypeRefIr::LocalType { type_index: 7 }),
            (
                "package".to_string(),
                TypeRefIr::PackageSymbol {
                    symbol: skiff_artifact_model::PackageSymbolRef {
                        package: skiff_artifact_model::PackageRefIr::Dependency {
                            dependency_ref: "dep".to_string(),
                        },
                        symbol_path: "pkg.Type".to_string(),
                        abi_expectation: None,
                    },
                },
            ),
            (
                "service".to_string(),
                TypeRefIr::ServiceSymbol {
                    symbol: skiff_artifact_model::ServiceSymbolRef {
                        module_path: "svc".to_string(),
                        symbol: "Thing".to_string(),
                    },
                },
            ),
            (
                "db".to_string(),
                TypeRefIr::DbObjectSymbol {
                    symbol: skiff_artifact_model::ServiceSymbolRef {
                        module_path: "db".to_string(),
                        symbol: "Row".to_string(),
                    },
                },
            ),
            ("unknown".to_string(), native("CustomNative")),
        ]),
    };

    let facts = TypeGraphAnalyzer::new().analyze(&ty).facts;

    assert!(facts.contains_native);
    assert!(facts.contains_local_type);
    assert!(facts.contains_package_symbol);
    assert!(facts.contains_service_symbol);
    assert!(facts.contains_db_object_symbol);
    assert!(!facts.schema_projectable_plain_data);
}

#[test]
fn type_graph_allows_plain_data_shape_from_allowlisted_natives() {
    let ty = TypeRefIr::Record {
        fields: BTreeMap::from([(
            "values".to_string(),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::Nullable {
                    inner: Box::new(native("string")),
                }],
            },
        )]),
    };

    let facts = TypeGraphAnalyzer::new().analyze(&ty).facts;

    assert!(facts.contains_native);
    assert!(facts.schema_projectable_plain_data);
}
