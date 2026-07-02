use std::collections::BTreeMap;

use skiff_artifact_model::{FunctionTypeParamIr, TypeRefIr};

pub fn walk_type_ref(ty: &TypeRefIr, visit: &mut impl FnMut(&TypeRefIr)) {
    visit(ty);
    match ty {
        TypeRefIr::Native { args, .. } => {
            for arg in args {
                walk_type_ref(arg, visit);
            }
        }
        TypeRefIr::LocalType { .. } => {}
        TypeRefIr::ServiceSymbol { .. } => {}
        TypeRefIr::PackageSymbol { .. } => {}
        TypeRefIr::DbObjectSymbol { .. } => {}
        TypeRefIr::Record { fields } => {
            for field_ty in fields.values() {
                walk_type_ref(field_ty, visit);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                walk_type_ref(item, visit);
            }
        }
        TypeRefIr::Nullable { inner } => walk_type_ref(inner, visit),
        TypeRefIr::Literal { .. } => {}
        TypeRefIr::TypeParam { .. } => {}
        TypeRefIr::AnyInterface { interface } => {
            for arg in &interface.canonical_type_args {
                walk_type_ref(arg, visit);
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                walk_type_ref(&param.ty, visit);
            }
            walk_type_ref(return_type, visit);
        }
    }
}

pub fn any_type_ref(ty: &TypeRefIr, predicate: &mut impl FnMut(&TypeRefIr) -> bool) -> bool {
    if predicate(ty) {
        return true;
    }
    match ty {
        TypeRefIr::Native { args, .. } => args.iter().any(|arg| any_type_ref(arg, predicate)),
        TypeRefIr::LocalType { .. } => false,
        TypeRefIr::ServiceSymbol { .. } => false,
        TypeRefIr::PackageSymbol { .. } => false,
        TypeRefIr::DbObjectSymbol { .. } => false,
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field_ty| any_type_ref(field_ty, predicate)),
        TypeRefIr::Union { items } => items.iter().any(|item| any_type_ref(item, predicate)),
        TypeRefIr::Nullable { inner } => any_type_ref(inner, predicate),
        TypeRefIr::Literal { .. } => false,
        TypeRefIr::TypeParam { .. } => false,
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(|arg| any_type_ref(arg, predicate)),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| any_type_ref(&param.ty, predicate))
                || any_type_ref(return_type, predicate)
        }
    }
}

pub fn map_type_ref(ty: TypeRefIr, map: &mut impl FnMut(TypeRefIr) -> TypeRefIr) -> TypeRefIr {
    let ty = match ty {
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name,
            args: args.into_iter().map(|arg| map_type_ref(arg, map)).collect(),
        },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
        TypeRefIr::DbObjectSymbol { symbol } => TypeRefIr::DbObjectSymbol { symbol },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, field_ty)| (name, map_type_ref(field_ty, map)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .into_iter()
                .map(|item| map_type_ref(item, map))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(map_type_ref(*inner, map)),
        },
        TypeRefIr::Literal { value } => TypeRefIr::Literal { value },
        TypeRefIr::TypeParam { name } => TypeRefIr::TypeParam { name },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id,
                canonical_type_args: interface
                    .canonical_type_args
                    .into_iter()
                    .map(|arg| map_type_ref(arg, map))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .into_iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name,
                    ty: map_type_ref(param.ty, map),
                })
                .collect(),
            return_type: Box::new(map_type_ref(*return_type, map)),
        },
    };
    map(ty)
}

pub fn substitute_type_params_in_type_ref(
    ty: TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> TypeRefIr {
    map_type_ref(ty, &mut |ty| match ty {
        TypeRefIr::TypeParam { name } => substitutions
            .get(&name)
            .cloned()
            .unwrap_or(TypeRefIr::TypeParam { name }),
        TypeRefIr::Native { name, args } => TypeRefIr::Native { name, args },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
        TypeRefIr::DbObjectSymbol { symbol } => TypeRefIr::DbObjectSymbol { symbol },
        TypeRefIr::Record { fields } => TypeRefIr::Record { fields },
        TypeRefIr::Union { items } => TypeRefIr::Union { items },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable { inner },
        TypeRefIr::Literal { value } => TypeRefIr::Literal { value },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface { interface },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params,
            return_type,
        },
    })
}

pub fn substitute_type_params_in_type_ref_ref(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> TypeRefIr {
    substitute_type_params_in_type_ref(ty.clone(), substitutions)
}

pub fn contains_any_interface(ty: &TypeRefIr) -> bool {
    any_type_ref(ty, &mut |ty| matches!(ty, TypeRefIr::AnyInterface { .. }))
}

pub fn contains_boundary_unsafe_type(ty: &TypeRefIr) -> bool {
    contains_any_interface(ty)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeRefVisitPathSegment {
    NativeArg { name: String, index: usize },
    RecordField { name: String },
    UnionItem { index: usize },
    NullableInner,
    AnyInterfaceTypeArg { index: usize },
    FunctionParam { name: String, index: usize },
    FunctionReturn,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRefVisitPath {
    segments: Vec<TypeRefVisitPathSegment>,
}

impl TypeRefVisitPath {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn segments(&self) -> &[TypeRefVisitPathSegment] {
        &self.segments
    }

    pub fn child(&self, segment: TypeRefVisitPathSegment) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment);
        Self { segments }
    }
}

#[derive(Clone, Debug)]
pub struct TypeRefVisit<'a> {
    pub ty: &'a TypeRefIr,
    pub path: TypeRefVisitPath,
}

pub fn walk_type_ref_with_path(ty: &TypeRefIr, visit: &mut impl FnMut(TypeRefVisit<'_>)) {
    walk_type_ref_with_path_at(ty, TypeRefVisitPath::empty(), visit);
}

fn walk_type_ref_with_path_at(
    ty: &TypeRefIr,
    path: TypeRefVisitPath,
    visit: &mut impl FnMut(TypeRefVisit<'_>),
) {
    visit(TypeRefVisit {
        ty,
        path: path.clone(),
    });
    match ty {
        TypeRefIr::Native { name, args } => {
            for (index, arg) in args.iter().enumerate() {
                walk_type_ref_with_path_at(
                    arg,
                    path.child(TypeRefVisitPathSegment::NativeArg {
                        name: name.clone(),
                        index,
                    }),
                    visit,
                );
            }
        }
        TypeRefIr::LocalType { .. } => {}
        TypeRefIr::ServiceSymbol { .. } => {}
        TypeRefIr::PackageSymbol { .. } => {}
        TypeRefIr::DbObjectSymbol { .. } => {}
        TypeRefIr::Record { fields } => {
            for (name, field_ty) in fields {
                walk_type_ref_with_path_at(
                    field_ty,
                    path.child(TypeRefVisitPathSegment::RecordField { name: name.clone() }),
                    visit,
                );
            }
        }
        TypeRefIr::Union { items } => {
            for (index, item) in items.iter().enumerate() {
                walk_type_ref_with_path_at(
                    item,
                    path.child(TypeRefVisitPathSegment::UnionItem { index }),
                    visit,
                );
            }
        }
        TypeRefIr::Nullable { inner } => walk_type_ref_with_path_at(
            inner,
            path.child(TypeRefVisitPathSegment::NullableInner),
            visit,
        ),
        TypeRefIr::Literal { .. } => {}
        TypeRefIr::TypeParam { .. } => {}
        TypeRefIr::AnyInterface { interface } => {
            for (index, arg) in interface.canonical_type_args.iter().enumerate() {
                walk_type_ref_with_path_at(
                    arg,
                    path.child(TypeRefVisitPathSegment::AnyInterfaceTypeArg { index }),
                    visit,
                );
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for (index, param) in params.iter().enumerate() {
                walk_type_ref_with_path_at(
                    &param.ty,
                    path.child(TypeRefVisitPathSegment::FunctionParam {
                        name: param.name.clone(),
                        index,
                    }),
                    visit,
                );
            }
            walk_type_ref_with_path_at(
                return_type,
                path.child(TypeRefVisitPathSegment::FunctionReturn),
                visit,
            );
        }
    }
}

#[cfg(test)]
mod tests {
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
        TypeRefIr::native(name)
    }

    fn any_interface(args: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: "iface".to_string(),
                canonical_type_args: args,
            },
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
        let ty = TypeRefIr::Native {
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
            TypeRefIr::Native {
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
        let ty = TypeRefIr::Native {
            name: "Box".to_string(),
            args: vec![type_param("T")],
        };
        let mut visited = Vec::new();

        let actual = map_type_ref(ty, &mut |ty| {
            match &ty {
                TypeRefIr::TypeParam { name } => visited.push(format!("param:{name}")),
                TypeRefIr::Native { name, .. } => visited.push(format!("native:{name}")),
                TypeRefIr::LocalType { .. } => visited.push("local".to_string()),
                TypeRefIr::ServiceSymbol { .. } => visited.push("service".to_string()),
                TypeRefIr::PackageSymbol { .. } => visited.push("package".to_string()),
                TypeRefIr::DbObjectSymbol { .. } => visited.push("db".to_string()),
                TypeRefIr::Record { .. } => visited.push("record".to_string()),
                TypeRefIr::Union { .. } => visited.push("union".to_string()),
                TypeRefIr::Nullable { .. } => visited.push("nullable".to_string()),
                TypeRefIr::Literal { .. } => visited.push("literal".to_string()),
                TypeRefIr::AnyInterface { .. } => visited.push("anyInterface".to_string()),
                TypeRefIr::Function { .. } => visited.push("function".to_string()),
            }
            match ty {
                TypeRefIr::TypeParam { name } if name == "T" => TypeRefIr::Native {
                    name: "Wrapper".to_string(),
                    args: vec![type_param("SHOULD_NOT_VISIT")],
                },
                other => other,
            }
        });

        assert_eq!(visited, vec!["param:T", "native:Box"]);
        assert_eq!(
            actual,
            TypeRefIr::Native {
                name: "Box".to_string(),
                args: vec![TypeRefIr::Native {
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
                fields: BTreeMap::from([(
                    "item".to_string(),
                    any_interface(vec![type_param("T")]),
                )]),
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
            substitute_type_params_in_type_ref(
                any_interface(vec![type_param("T")]),
                &substitutions
            ),
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
}
