use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableProvenanceSummary, CallableSemanticFacts, ContractTypeRef, FileIrUnit,
    PackageCallableSignature, PackageTypeRef, TypeRefIr, ValueProjectionStep, ValueProvenance,
};

use crate::package_artifact::boundary::ordering::escape_lane_rank;

pub(super) fn normalize_semantic_facts(mut facts: CallableSemanticFacts) -> CallableSemanticFacts {
    if let CallableProvenanceSummary::Analyzed {
        return_origins,
        direct_return_origins,
        throw_origins,
        escape_lanes,
    } = &mut facts.provenance
    {
        return_origins.sort_by_key(provenance_sort_key);
        return_origins.dedup();
        direct_return_origins.sort_by_key(provenance_sort_key);
        direct_return_origins.dedup();
        throw_origins.sort_by_key(provenance_sort_key);
        throw_origins.dedup();
        escape_lanes.sort_by_key(|lane| escape_lane_rank(*lane));
        escape_lanes.dedup();
    }
    facts
}

pub(super) fn normalize_public_signature(
    owner_module: &str,
    signature: &mut PackageCallableSignature,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
) {
    for parameter in &mut signature.parameters {
        parameter.ty =
            normalize_package_type(owner_module, &parameter.ty, file_ir_units, public_type_ids);
    }
    signature.return_type = normalize_package_type(
        owner_module,
        &signature.return_type,
        file_ir_units,
        public_type_ids,
    );
    signature.throw_types = signature
        .throw_types
        .iter()
        .map(|ty| normalize_package_type(owner_module, ty, file_ir_units, public_type_ids))
        .collect();
}

fn normalize_package_type(
    owner_module: &str,
    ty: &PackageTypeRef,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
) -> PackageTypeRef {
    match ty {
        PackageTypeRef::Local { local_type } => {
            let local_type =
                normalize_local_type(owner_module, local_type, file_ir_units, public_type_ids);
            lift_local_type(local_type)
        }
        PackageTypeRef::Container { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(owner_module, argument, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        PackageTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(normalize_package_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
            )),
        },
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => PackageTypeRef::AnyInterface {
            interface: Box::new(normalize_package_type(
                owner_module,
                interface,
                file_ir_units,
                public_type_ids,
            )),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_package_type(owner_module, argument, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        exact @ PackageTypeRef::PackageSchema { .. } => exact.clone(),
    }
}

fn normalize_local_type(
    owner_module: &str,
    ty: &TypeRefIr,
    file_ir_units: &[FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), ContractTypeRef>,
) -> TypeRefIr {
    if let Some((module_path, type_index)) = nominal_source(owner_module, ty) {
        if let Some(binding_name) = local_type_binding(file_ir_units, module_path, type_index) {
            if let Some(exact) =
                public_type_ids.get(&(module_path.to_string(), binding_name.to_string()))
            {
                if let ContractTypeRef::PackageSchema {
                    package_id,
                    stable_schema_key,
                    package_schema_type_id,
                } = exact
                {
                    return TypeRefIr::PackageSchema {
                        package_id: package_id.clone(),
                        stable_schema_key: stable_schema_key.clone(),
                        package_schema_type_id: package_schema_type_id.clone(),
                    };
                }
            }
        }
        return ty.clone();
    }
    match ty {
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| normalize_local_type(owner_module, arg, file_ir_units, public_type_ids))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    (
                        name.clone(),
                        normalize_local_type(owner_module, field, file_ir_units, public_type_ids),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| {
                    normalize_local_type(owner_module, item, file_ir_units, public_type_ids)
                })
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(normalize_local_type(
                owner_module,
                inner,
                file_ir_units,
                public_type_ids,
            )),
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| skiff_artifact_model::FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: normalize_local_type(
                        owner_module,
                        &param.ty,
                        file_ir_units,
                        public_type_ids,
                    ),
                })
                .collect(),
            return_type: Box::new(normalize_local_type(
                owner_module,
                return_type,
                file_ir_units,
                public_type_ids,
            )),
        },
        _ => ty.clone(),
    }
}

fn lift_local_type(ty: TypeRefIr) -> PackageTypeRef {
    match ty {
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        },
        TypeRefIr::Builtin { name, args } if !args.is_empty() => PackageTypeRef::Container {
            name,
            arguments: args.into_iter().map(lift_local_type).collect(),
        },
        TypeRefIr::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(lift_local_type(*inner)),
        },
        local_type => PackageTypeRef::Local { local_type },
    }
}

fn nominal_source<'a>(owner_module: &'a str, ty: &'a TypeRefIr) -> Option<(&'a str, u32)> {
    match ty {
        TypeRefIr::LocalType { type_index } => Some((owner_module, *type_index)),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => Some((module_path, *type_index)),
        _ => None,
    }
}

fn local_type_binding<'a>(
    file_ir_units: &'a [FileIrUnit],
    module_path: &str,
    type_index: u32,
) -> Option<&'a str> {
    let mut units = file_ir_units
        .iter()
        .filter(|unit| unit.module_path == module_path);
    let unit = units.next()?;
    if units.next().is_some() {
        return None;
    }
    let mut declarations = unit
        .declarations
        .types
        .iter()
        .filter(|(_, declaration)| declaration.type_index == type_index);
    let (binding_name, _) = declarations.next()?;
    if declarations.next().is_some() {
        return None;
    }
    Some(binding_name)
}

fn provenance_sort_key(origin: &ValueProvenance) -> (u8, String) {
    match origin {
        ValueProvenance::Fresh => (0, String::new()),
        ValueProvenance::Constant => (1, String::new()),
        ValueProvenance::CallerParameter { index } => (2, format!("{index:010}")),
        ValueProvenance::CallerParameterProjection { index, path } => {
            let mut key = format!("{index:010}:");
            for step in path.steps() {
                match step {
                    ValueProjectionStep::Field { name } => {
                        key.push_str(&format!("f{}:{name};", name.len()));
                    }
                    ValueProjectionStep::ContainerElement {} => key.push_str("e;"),
                }
            }
            (3, key)
        }
        ValueProvenance::DependencyReturn { callable_id } => (4, callable_id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        CallableEffectSummary, PackageCallableParameter, PackageSchemaTypeId, TypeDeclarationIr,
    };

    fn fixture() -> (Vec<FileIrUnit>, BTreeMap<(String, String), ContractTypeRef>) {
        let mut unit = FileIrUnit::empty("api", "source-hash");
        unit.declarations.types.insert(
            "PublicError".into(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "PublicError".into(),
                source_span: None,
            },
        );
        unit.declarations.types.insert(
            "PrivateDetail".into(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "PrivateDetail".into(),
                source_span: None,
            },
        );
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
    fn public_nominals_are_exact_through_parameters_return_and_throws() {
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
            parameters: vec![PackageCallableParameter {
                name: "values".into(),
                ty: nested,
            }],
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::PublicationType {
                    module_path: "api".into(),
                    type_index: 0,
                },
            },
            throw_types: vec![PackageTypeRef::Local {
                local_type: TypeRefIr::LocalType { type_index: 0 },
            }],
            may_suspend: false,
        };

        normalize_public_signature("api", &mut signature, &units, &refs);

        let exact = PackageTypeRef::PackageSchema {
            package_id: "example.pkg".into(),
            stable_schema_key: "errors.PublicError".into(),
            package_schema_type_id: PackageSchemaTypeId::new("schema:public-error"),
        };
        assert_eq!(signature.return_type, exact);
        assert_eq!(signature.throw_types, vec![exact.clone()]);
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
    fn private_or_unresolved_local_nominal_remains_local_only() {
        let (units, refs) = fixture();
        let private = PackageTypeRef::Local {
            local_type: TypeRefIr::LocalType { type_index: 1 },
        };
        assert_eq!(
            normalize_package_type("api", &private, &units, &refs),
            private
        );
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
}
