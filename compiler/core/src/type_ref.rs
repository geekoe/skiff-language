use std::collections::BTreeMap;

use skiff_artifact_model::{FunctionTypeParamIr, TypeRefIr};

pub fn walk_type_ref(ty: &TypeRefIr, visit: &mut impl FnMut(&TypeRefIr)) {
    visit(ty);
    for child in type_ref_children(ty) {
        walk_type_ref(child.ty, visit);
    }
}

pub fn any_type_ref(ty: &TypeRefIr, predicate: &mut impl FnMut(&TypeRefIr) -> bool) -> bool {
    if predicate(ty) {
        return true;
    }
    type_ref_children(ty)
        .into_iter()
        .any(|child| any_type_ref(child.ty, predicate))
}

pub fn map_type_ref(ty: TypeRefIr, map: &mut impl FnMut(TypeRefIr) -> TypeRefIr) -> TypeRefIr {
    let ty = match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name,
            args: args.into_iter().map(|arg| map_type_ref(arg, map)).collect(),
        },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PublicationType {
            module_path,
            type_index,
        },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base,
            arguments: arguments
                .into_iter()
                .map(|argument| map_type_ref(argument, map))
                .collect(),
        },
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
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin { name, args },
        TypeRefIr::LocalType { type_index } => TypeRefIr::LocalType { type_index },
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PublicationType {
            module_path,
            type_index,
        },
        TypeRefIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol { symbol },
        TypeRefIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol { symbol },
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        },
        TypeRefIr::AppliedNominal { base, arguments } => {
            TypeRefIr::AppliedNominal { base, arguments }
        }
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
    AppliedNominalArgument { index: usize },
    RecordField { name: String },
    UnionItem { index: usize },
    NullableInner,
    AnyInterfaceTypeArg { index: usize },
    FunctionParam { name: String, index: usize },
    FunctionReturn,
}

#[derive(Clone, Debug)]
pub struct TypeRefChild<'a> {
    pub ty: &'a TypeRefIr,
    pub segment: TypeRefVisitPathSegment,
}

pub fn type_ref_children(ty: &TypeRefIr) -> Vec<TypeRefChild<'_>> {
    match ty {
        TypeRefIr::Builtin { name, args } => args
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::NativeArg {
                    name: name.clone(),
                    index,
                },
            })
            .collect(),
        TypeRefIr::AppliedNominal { arguments, .. } => arguments
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::AppliedNominalArgument { index },
            })
            .collect(),
        TypeRefIr::Record { fields } => fields
            .iter()
            .map(|(name, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::RecordField { name: name.clone() },
            })
            .collect(),
        TypeRefIr::Union { items } => items
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::UnionItem { index },
            })
            .collect(),
        TypeRefIr::Nullable { inner } => vec![TypeRefChild {
            ty: inner,
            segment: TypeRefVisitPathSegment::NullableInner,
        }],
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .enumerate()
            .map(|(index, ty)| TypeRefChild {
                ty,
                segment: TypeRefVisitPathSegment::AnyInterfaceTypeArg { index },
            })
            .collect(),
        TypeRefIr::Function {
            params,
            return_type,
        } => params
            .iter()
            .enumerate()
            .map(|(index, param)| TypeRefChild {
                ty: &param.ty,
                segment: TypeRefVisitPathSegment::FunctionParam {
                    name: param.name.clone(),
                    index,
                },
            })
            .chain(std::iter::once(TypeRefChild {
                ty: return_type,
                segment: TypeRefVisitPathSegment::FunctionReturn,
            }))
            .collect(),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Vec::new(),
    }
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
    for child in type_ref_children(ty) {
        walk_type_ref_with_path_at(child.ty, path.child(child.segment), visit);
    }
}

#[cfg(test)]
mod tests;
