use std::collections::BTreeSet;

use skiff_artifact_model::TypeRefIr;

use crate::type_ref::{walk_type_ref_with_path, TypeRefVisitPath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeGraphNodeKind {
    Native { name: String },
    LocalType,
    PublicationType,
    ServiceSymbol,
    PackageSymbol,
    DbObjectSymbol,
    Record,
    Union,
    Nullable,
    Literal,
    TypeParam,
    AnyInterface,
    Function,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeGraphNodeFact {
    pub path: TypeRefVisitPath,
    pub kind: TypeGraphNodeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeGraphFacts {
    pub contains_function: bool,
    pub contains_any_interface: bool,
    pub contains_native: bool,
    pub contains_local_type: bool,
    pub contains_package_symbol: bool,
    pub contains_service_symbol: bool,
    pub contains_db_object_symbol: bool,
    pub contains_type_param: bool,
    pub schema_projectable_plain_data: bool,
}

impl Default for TypeGraphFacts {
    fn default() -> Self {
        Self {
            contains_function: false,
            contains_any_interface: false,
            contains_native: false,
            contains_local_type: false,
            contains_package_symbol: false,
            contains_service_symbol: false,
            contains_db_object_symbol: false,
            contains_type_param: false,
            schema_projectable_plain_data: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeGraphAnalysis {
    pub nodes: Vec<TypeGraphNodeFact>,
    pub facts: TypeGraphFacts,
}

#[derive(Clone, Debug)]
pub struct TypeGraphAnalyzer {
    schema_projectable_native_names: BTreeSet<String>,
}

impl Default for TypeGraphAnalyzer {
    fn default() -> Self {
        Self {
            schema_projectable_native_names: [
                "Json", "Array", "Map", "bool", "boolean", "float", "int", "null", "number",
                "string", "unit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

impl TypeGraphAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn analyze(&self, ty: &TypeRefIr) -> TypeGraphAnalysis {
        let mut analysis = TypeGraphAnalysis::default();
        walk_type_ref_with_path(ty, &mut |visit| {
            let kind = self.node_kind(visit.ty);
            self.apply_facts(&mut analysis.facts, visit.ty);
            analysis.nodes.push(TypeGraphNodeFact {
                path: visit.path,
                kind,
            });
        });
        analysis
    }

    fn node_kind(&self, ty: &TypeRefIr) -> TypeGraphNodeKind {
        match ty {
            TypeRefIr::Native { name, .. } => TypeGraphNodeKind::Native { name: name.clone() },
            TypeRefIr::LocalType { .. } => TypeGraphNodeKind::LocalType,
            TypeRefIr::PublicationType { .. } => TypeGraphNodeKind::PublicationType,
            TypeRefIr::ServiceSymbol { .. } => TypeGraphNodeKind::ServiceSymbol,
            TypeRefIr::PackageSymbol { .. } => TypeGraphNodeKind::PackageSymbol,
            TypeRefIr::DbObjectSymbol { .. } => TypeGraphNodeKind::DbObjectSymbol,
            TypeRefIr::Record { .. } => TypeGraphNodeKind::Record,
            TypeRefIr::Union { .. } => TypeGraphNodeKind::Union,
            TypeRefIr::Nullable { .. } => TypeGraphNodeKind::Nullable,
            TypeRefIr::Literal { .. } => TypeGraphNodeKind::Literal,
            TypeRefIr::TypeParam { .. } => TypeGraphNodeKind::TypeParam,
            TypeRefIr::AnyInterface { .. } => TypeGraphNodeKind::AnyInterface,
            TypeRefIr::Function { .. } => TypeGraphNodeKind::Function,
        }
    }

    fn apply_facts(&self, facts: &mut TypeGraphFacts, ty: &TypeRefIr) {
        match ty {
            TypeRefIr::Native { name, .. } => {
                facts.contains_native = true;
                if !self.schema_projectable_native_names.contains(name) {
                    facts.schema_projectable_plain_data = false;
                }
            }
            TypeRefIr::LocalType { .. } => {
                facts.contains_local_type = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::PublicationType { .. } => {
                facts.contains_local_type = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::ServiceSymbol { .. } => {
                facts.contains_service_symbol = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::PackageSymbol { .. } => {
                facts.contains_package_symbol = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::DbObjectSymbol { .. } => {
                facts.contains_db_object_symbol = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::TypeParam { .. } => {
                facts.contains_type_param = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::AnyInterface { .. } => {
                facts.contains_any_interface = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::Function { .. } => {
                facts.contains_function = true;
                facts.schema_projectable_plain_data = false;
            }
            TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
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
        TypeRefIr::native(name)
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
                TypeRefIr::Native {
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
}
