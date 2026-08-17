use skiff_artifact_identity::type_ref_abi_key;
use skiff_artifact_model::{
    ContractTypeRef, PackageRefIr, PackageSymbolRef, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::type_ref::{
    contract_type_ref_to_ir, package_type_ref_to_ir, BuiltinShape,
};

pub(super) use crate::contract_type_resolution::package_type_contains_contract;

use crate::{
    contract_type_resolution::package_type_ref_from_source_type, shared::ast::TypeRef,
    ResolvedTypeRef, SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

pub(super) struct ContractCallTypeProjection<'a, 'ctx> {
    type_resolution: &'a TypeResolutionModel,
    dependency_analysis: &'a SourceDependencyAnalysisInput,
    type_context: &'a TypeResolutionContext<'ctx>,
}

impl<'a, 'ctx> ContractCallTypeProjection<'a, 'ctx> {
    pub(super) fn new(
        type_resolution: &'a TypeResolutionModel,
        dependency_analysis: &'a SourceDependencyAnalysisInput,
        type_context: &'a TypeResolutionContext<'ctx>,
    ) -> Self {
        Self {
            type_resolution,
            dependency_analysis,
            type_context,
        }
    }

    pub(super) fn try_source_package_type_ref(
        &self,
        ty: &TypeRef,
    ) -> Result<PackageTypeRef, String> {
        package_type_ref_from_source_type(
            ty,
            self.type_context,
            self.type_resolution,
            self.dependency_analysis,
        )
    }

    pub(super) fn try_resolved_package_type(
        &self,
        ty: &ResolvedTypeRef,
    ) -> Result<PackageTypeRef, String> {
        crate::type_projection::package_type_ref_from_ir(&ty.ir, self.dependency_analysis)
    }
}

pub(crate) fn contract_source_assignability(
    actual: &ResolvedTypeRef,
    actual_projected: Option<&PackageTypeRef>,
    expected: &ResolvedTypeRef,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    type_context: &TypeResolutionContext<'_>,
) -> Result<Option<bool>, String> {
    let Some(dependency_analysis) = dependency_analysis else {
        return Ok(None);
    };
    if matches!(expected.ir, TypeRefIr::AnyInterface { .. }) {
        return Ok(Some(type_resolution.assignable_in_context(
            actual,
            expected,
            type_context,
        )));
    }
    let projection =
        ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context);
    let actual = match actual_projected {
        Some(actual) => actual.clone(),
        None => projection.try_resolved_package_type(actual)?,
    };
    let expected = projection.try_resolved_package_type(expected)?;
    Ok(
        (package_type_contains_contract(&actual) || package_type_contains_contract(&expected))
            .then(|| package_type_target_assignable(&actual, &expected, dependency_analysis)),
    )
}

pub(crate) fn contract_source_assignability_with_projections(
    actual: &ResolvedTypeRef,
    actual_projected: Option<&PackageTypeRef>,
    expected: &ResolvedTypeRef,
    expected_projected: Option<&PackageTypeRef>,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: Option<&SourceDependencyAnalysisInput>,
    type_context: &TypeResolutionContext<'_>,
) -> Result<Option<bool>, String> {
    let Some(dependency_analysis) = dependency_analysis else {
        return Ok(None);
    };
    if matches!(expected.ir, TypeRefIr::AnyInterface { .. }) {
        return Ok(Some(type_resolution.assignable_in_context(
            actual,
            expected,
            type_context,
        )));
    }
    let projection =
        ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context);
    let actual = match actual_projected {
        Some(actual) => actual.clone(),
        None => projection.try_resolved_package_type(actual)?,
    };
    let expected = match expected_projected {
        Some(expected) => expected.clone(),
        None => projection.try_resolved_package_type(expected)?,
    };
    Ok(
        (package_type_contains_contract(&actual) || package_type_contains_contract(&expected))
            .then(|| package_type_target_assignable(&actual, &expected, dependency_analysis)),
    )
}

pub fn package_type_ref_from_contract_type(ty: &ContractTypeRef) -> PackageTypeRef {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(package_type_ref_from_contract_type)
                .collect(),
        },
        ContractTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(package_type_ref_from_contract_type(inner)),
        },
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => PackageTypeRef::AnyInterface {
            interface: Box::new(package_type_ref_from_contract_type(interface)),
            arguments: arguments
                .iter()
                .map(package_type_ref_from_contract_type)
                .collect(),
        },
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => PackageTypeRef::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
        ContractTypeRef::TypeParam { name } => PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam { name: name.clone() },
        },
        ContractTypeRef::Record { fields } => PackageTypeRef::Local {
            local_type: TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), contract_type_ref_to_ir(ty)))
                    .collect(),
            },
        },
        ContractTypeRef::StructuralUnion { variants } => PackageTypeRef::Local {
            local_type: TypeRefIr::Union {
                items: variants.iter().map(contract_type_ref_to_ir).collect(),
            },
        },
        ContractTypeRef::Literal { value } => PackageTypeRef::Local {
            local_type: TypeRefIr::Literal {
                value: match value {
                    skiff_artifact_model::ContractLiteral::String { value } => {
                        skiff_artifact_model::LiteralIr::String {
                            value: value.clone(),
                        }
                    }
                },
            },
        },
    }
}

pub(super) fn package_type_assignable(actual: &PackageTypeRef, expected: &PackageTypeRef) -> bool {
    match (actual, expected) {
        (
            PackageTypeRef::PackageSchema {
                package_id: actual_package,
                stable_schema_key: actual_key,
                package_schema_type_id: actual_id,
            },
            PackageTypeRef::PackageSchema {
                package_id: expected_package,
                stable_schema_key: expected_key,
                package_schema_type_id: expected_id,
            },
        ) => {
            actual_package == expected_package
                && actual_key == expected_key
                && actual_id == expected_id
        }
        (
            PackageTypeRef::Container {
                name: actual_name,
                arguments: actual_arguments,
            },
            PackageTypeRef::Container {
                name: expected_name,
                arguments: expected_arguments,
            },
        ) => {
            if actual_name == "integer"
                && actual_arguments.is_empty()
                && expected_name == "number"
                && expected_arguments.is_empty()
            {
                return true;
            }
            actual_name == expected_name
                && actual_arguments.len() == expected_arguments.len()
                && actual_arguments
                    .iter()
                    .zip(expected_arguments)
                    .all(|(actual, expected)| package_type_assignable(actual, expected))
        }
        (
            PackageTypeRef::Nullable { inner: actual },
            PackageTypeRef::Nullable { inner: expected },
        ) => package_type_assignable(actual, expected),
        (
            PackageTypeRef::AnyInterface {
                interface: actual,
                arguments: actual_arguments,
            },
            PackageTypeRef::AnyInterface {
                interface: expected,
                arguments: expected_arguments,
            },
        ) => {
            package_type_assignable(actual, expected)
                && actual_arguments.len() == expected_arguments.len()
                && actual_arguments
                    .iter()
                    .zip(expected_arguments)
                    .all(|(actual, expected)| package_type_assignable(actual, expected))
        }
        (actual, PackageTypeRef::Nullable { inner: expected }) => {
            package_type_is_null(actual) || package_type_assignable(actual, expected)
        }
        (
            PackageTypeRef::Local { local_type: actual },
            PackageTypeRef::Local {
                local_type: expected,
            },
        ) => actual == expected,
        _ => false,
    }
}

pub(crate) fn package_type_target_assignable(
    actual: &PackageTypeRef,
    expected: &PackageTypeRef,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> bool {
    if package_type_assignable(actual, expected) {
        return true;
    }
    if let PackageTypeRef::Local { local_type } = actual {
        if let Ok(projected) =
            crate::type_projection::package_type_ref_from_ir(local_type, dependency_analysis)
        {
            if &projected != actual {
                return package_type_target_assignable(&projected, expected, dependency_analysis);
            }
        }
    }
    if let PackageTypeRef::Local { local_type } = expected {
        if let Ok(projected) =
            crate::type_projection::package_type_ref_from_ir(local_type, dependency_analysis)
        {
            if &projected != expected {
                return package_type_target_assignable(actual, &projected, dependency_analysis);
            }
        }
    }
    if package_type_is_json_target(expected) {
        return package_type_json_compatible(
            actual,
            dependency_analysis,
            package_type_is_json_object_target(expected),
        );
    }
    match (actual, expected) {
        (PackageTypeRef::PackageSchema { .. }, PackageTypeRef::PackageSchema { .. }) => false,
        (
            _,
            PackageTypeRef::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            },
        ) => dependency_analysis
            .exact_package_type(package_id, stable_schema_key, package_schema_type_id)
            .and_then(package_schema_representation)
            .is_some_and(|representation| {
                package_type_target_assignable(actual, &representation, dependency_analysis)
            }),
        (
            PackageTypeRef::Container {
                name: actual_name,
                arguments: actual_arguments,
            },
            PackageTypeRef::Container {
                name: expected_name,
                arguments: expected_arguments,
            },
        ) => {
            actual_name == expected_name
                && actual_arguments.len() == expected_arguments.len()
                && actual_arguments
                    .iter()
                    .zip(expected_arguments)
                    .all(|(actual, expected)| {
                        package_type_target_assignable(actual, expected, dependency_analysis)
                    })
        }
        (
            PackageTypeRef::Nullable { inner: actual },
            PackageTypeRef::Nullable { inner: expected },
        ) => package_type_target_assignable(actual, expected, dependency_analysis),
        (
            PackageTypeRef::AnyInterface {
                interface: actual,
                arguments: actual_arguments,
            },
            PackageTypeRef::AnyInterface {
                interface: expected,
                arguments: expected_arguments,
            },
        ) => {
            package_type_assignable(actual, expected)
                && actual_arguments.len() == expected_arguments.len()
                && actual_arguments
                    .iter()
                    .zip(expected_arguments)
                    .all(|(actual, expected)| package_type_assignable(actual, expected))
        }
        (actual, PackageTypeRef::Nullable { inner: expected }) => {
            package_type_is_null(actual)
                || package_type_target_assignable(actual, expected, dependency_analysis)
        }
        (
            PackageTypeRef::Local { local_type: actual },
            PackageTypeRef::Local {
                local_type: expected,
            },
        ) => local_ir_target_assignable(actual, expected),
        (
            PackageTypeRef::Local { local_type: actual },
            PackageTypeRef::Container { name, arguments },
        ) => local_ir_target_assignable(
            actual,
            &TypeRefIr::Builtin {
                name: name.clone(),
                args: arguments
                    .iter()
                    .cloned()
                    .map(|argument| package_type_ref_to_ir(&argument))
                    .collect(),
            },
        ),
        _ => false,
    }
}

fn package_type_is_json_target(ty: &PackageTypeRef) -> bool {
    matches!(
        ty,
        PackageTypeRef::Container { name, arguments }
            if arguments.is_empty()
                && matches!(
                    BuiltinShape::of_name(name),
                    Some(BuiltinShape::Json | BuiltinShape::JsonObject)
                )
    )
}

fn package_type_is_json_object_target(ty: &PackageTypeRef) -> bool {
    matches!(
        ty,
        PackageTypeRef::Container { name, arguments }
            if arguments.is_empty() && name == BuiltinShape::JsonObject.name()
    )
}

fn package_type_json_compatible(
    ty: &PackageTypeRef,
    dependency_analysis: &SourceDependencyAnalysisInput,
    object_only: bool,
) -> bool {
    match ty {
        PackageTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let record = dependency_analysis.exact_package_type(
                package_id,
                stable_schema_key,
                package_schema_type_id,
            );
            record
                .and_then(package_schema_representation)
                .is_some_and(|representation| {
                    package_type_json_compatible(&representation, dependency_analysis, object_only)
                })
        }
        PackageTypeRef::Container { name, arguments } => match BuiltinShape::of_name(name) {
            Some(BuiltinShape::JsonObject) if arguments.is_empty() => true,
            Some(BuiltinShape::Json) if arguments.is_empty() => !object_only,
            Some(
                BuiltinShape::String
                | BuiltinShape::Integer
                | BuiltinShape::Number
                | BuiltinShape::Bool
                | BuiltinShape::Null,
            ) if arguments.is_empty() => !object_only,
            Some(BuiltinShape::Array)
                if arguments.len() == 1 && name == BuiltinShape::Array.name() =>
            {
                !object_only
                    && package_type_json_compatible(&arguments[0], dependency_analysis, false)
            }
            Some(BuiltinShape::Map) if arguments.len() == 2 && name == BuiltinShape::Map.name() => {
                let string_key = matches!(
                    &arguments[0],
                    PackageTypeRef::Container { name, arguments } if
                        name == BuiltinShape::String.name() && arguments.is_empty()
                );
                string_key
                    && package_type_json_compatible(&arguments[1], dependency_analysis, false)
            }
            _ => false,
        },
        PackageTypeRef::Nullable { inner } => {
            !object_only && package_type_json_compatible(inner, dependency_analysis, false)
        }
        PackageTypeRef::AnyInterface { .. } => false,
        PackageTypeRef::Local { local_type } => {
            local_ir_json_compatible(local_type, dependency_analysis, object_only)
        }
    }
}

pub(crate) fn local_ir_json_compatible(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
    object_only: bool,
) -> bool {
    match ty {
        TypeRefIr::Literal {
            value:
                skiff_artifact_model::LiteralIr::String { .. }
                | skiff_artifact_model::LiteralIr::Bool { .. }
                | skiff_artifact_model::LiteralIr::Number { .. }
                | skiff_artifact_model::LiteralIr::Null,
        } => !object_only,
        TypeRefIr::Builtin { name, args } => match BuiltinShape::of_name(name) {
            Some(BuiltinShape::JsonObject) if args.is_empty() => true,
            Some(BuiltinShape::Json) if args.is_empty() => !object_only,
            Some(
                BuiltinShape::String
                | BuiltinShape::Integer
                | BuiltinShape::Number
                | BuiltinShape::Bool
                | BuiltinShape::Null,
            ) if args.is_empty() => !object_only,
            Some(BuiltinShape::Array) if args.len() == 1 && name == BuiltinShape::Array.name() => {
                !object_only && local_ir_json_compatible(&args[0], dependency_analysis, false)
            }
            Some(BuiltinShape::Map) if args.len() == 2 && name == BuiltinShape::Map.name() => {
                matches!(&args[0], TypeRefIr::Builtin { name, args } if
                    name == BuiltinShape::String.name() && args.is_empty())
                    && local_ir_json_compatible(&args[1], dependency_analysis, false)
            }
            _ => false,
        },
        TypeRefIr::Nullable { inner } => {
            !object_only && local_ir_json_compatible(inner, dependency_analysis, false)
        }
        TypeRefIr::Union { items } => {
            !object_only
                && !items.is_empty()
                && items
                    .iter()
                    .all(|item| local_ir_json_compatible(item, dependency_analysis, false))
        }
        TypeRefIr::Record { fields } => fields
            .values()
            .all(|field| local_ir_json_compatible(field, dependency_analysis, false)),
        TypeRefIr::PackageSymbol { symbol } => {
            let compatible = match &symbol.package {
                PackageRefIr::PackageId { package_id } => dependency_analysis
                    .package_type_by_owner_and_stable_key(package_id, &symbol.symbol_path),
                PackageRefIr::Dependency { dependency_ref } => {
                    dependency_analysis.direct_package_type(dependency_ref, &symbol.symbol_path)
                }
            }
            .and_then(package_schema_representation)
            .is_some_and(|representation| {
                package_type_json_compatible(&representation, dependency_analysis, object_only)
            });
            compatible
        }
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let compatible = dependency_analysis
                .package_type_by_owner_and_stable_key(package_id, stable_schema_key)
                .filter(|record| &record.package_schema_type_id == package_schema_type_id)
                .and_then(package_schema_representation)
                .is_some_and(|representation| {
                    package_type_json_compatible(&representation, dependency_analysis, object_only)
                });
            compatible
        }
        TypeRefIr::ServiceSymbol { symbol } => dependency_analysis
            .public_package_type_by_stable_key(&symbol.module_path, &symbol.symbol)
            .ok()
            .and_then(package_schema_representation)
            .is_some_and(|representation| {
                package_type_json_compatible(&representation, dependency_analysis, object_only)
            }),
        _ => false,
    }
}

fn local_ir_target_assignable(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    if actual == expected {
        return true;
    }
    match (actual, expected) {
        (_, TypeRefIr::Union { items }) => items
            .iter()
            .any(|candidate| local_ir_target_assignable(actual, candidate)),
        (TypeRefIr::Union { items }, _) => items
            .iter()
            .all(|candidate| local_ir_target_assignable(candidate, expected)),
        (
            TypeRefIr::Record {
                fields: actual_fields,
            },
            TypeRefIr::Record {
                fields: expected_fields,
            },
        ) => {
            actual_fields.len() == expected_fields.len()
                && expected_fields.iter().all(|(name, expected)| {
                    actual_fields
                        .get(name)
                        .is_some_and(|actual| local_ir_target_assignable(actual, expected))
                })
        }
        (
            TypeRefIr::Builtin {
                name: actual_name,
                args: actual_args,
            },
            TypeRefIr::Builtin {
                name: expected_name,
                args: expected_args,
            },
        ) => {
            if actual_name == BuiltinShape::Integer.name()
                && actual_args.is_empty()
                && expected_name == BuiltinShape::Number.name()
                && expected_args.is_empty()
            {
                return true;
            }
            actual_name == expected_name
                && actual_args.len() == expected_args.len()
                && actual_args
                    .iter()
                    .zip(expected_args)
                    .all(|(actual, expected)| local_ir_target_assignable(actual, expected))
        }
        (actual, TypeRefIr::Nullable { inner }) => {
            matches!(
                actual,
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::Null
                }
            ) || local_ir_target_assignable(actual, inner)
        }
        (
            TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::String { .. },
            },
            TypeRefIr::Builtin { name, args },
        ) => name == BuiltinShape::String.name() && args.is_empty(),
        (
            TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::Bool { .. },
            },
            TypeRefIr::Builtin { name, args },
        ) => name == BuiltinShape::Bool.name() && args.is_empty(),
        (
            TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number { .. },
            },
            TypeRefIr::Builtin { name, args },
        ) => name == BuiltinShape::Number.name() && args.is_empty(),
        _ => false,
    }
}

fn package_schema_representation(
    record: &skiff_artifact_model::PackageSchemaTypeRecord,
) -> Option<PackageTypeRef> {
    use skiff_artifact_model::ContractTypeDescriptor;
    match &record.canonical_descriptor.descriptor {
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => {
            Some(package_type_ref_from_contract_type(target))
        }
        ContractTypeDescriptor::Enumeration { variants } => Some(PackageTypeRef::Local {
            local_type: TypeRefIr::Union {
                items: variants
                    .iter()
                    .map(|value| TypeRefIr::Literal {
                        value: skiff_artifact_model::LiteralIr::String {
                            value: value.clone(),
                        },
                    })
                    .collect(),
            },
        }),
        ContractTypeDescriptor::StructuralUnion { variants } => Some(PackageTypeRef::Local {
            local_type: TypeRefIr::Union {
                items: variants.iter().map(contract_type_ref_to_ir).collect(),
            },
        }),
        ContractTypeDescriptor::Record { fields } => Some(PackageTypeRef::Local {
            local_type: TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), contract_type_ref_to_ir(ty)))
                    .collect(),
            },
        }),
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            Some(PackageTypeRef::Local {
                local_type: TypeRefIr::Union {
                    items: branches
                        .iter()
                        .map(|branch| contract_type_ref_to_ir(&branch.branch_type))
                        .collect(),
                },
            })
        }
        ContractTypeDescriptor::CallbackInterface { .. } => None,
    }
}

fn package_type_is_null(ty: &PackageTypeRef) -> bool {
    matches!(
        ty,
        PackageTypeRef::Container { name, arguments }
            if name == BuiltinShape::Null.name() && arguments.is_empty()
    ) || matches!(
        ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::Literal {
                value: skiff_artifact_model::LiteralIr::Null,
            },
        }
    )
}

pub(super) fn resolved_contract_type(
    ty: &ContractTypeRef,
    alias: &str,
) -> Result<ResolvedTypeRef, String> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            let arguments = arguments
                .iter()
                .map(|argument| resolved_contract_type(argument, alias))
                .collect::<Result<Vec<_>, _>>()?;
            let text = if arguments.is_empty() {
                name.clone()
            } else {
                format!(
                    "{name}<{}>",
                    arguments
                        .iter()
                        .map(|argument| argument.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            Ok(ResolvedTypeRef::with_text(
                TypeRefIr::Builtin {
                    name: name.clone(),
                    args: arguments.into_iter().map(|argument| argument.ir).collect(),
                },
                text,
            ))
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => Ok(ResolvedTypeRef::with_text(
            TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: package_id.clone(),
                    },
                    symbol_path: stable_schema_key.clone(),
                    abi_expectation: None,
                },
            },
            format!("{alias}.{stable_schema_key}"),
        )),
        ContractTypeRef::TypeParam { name } => Ok(ResolvedTypeRef::with_text(
            TypeRefIr::TypeParam { name: name.clone() },
            name.clone(),
        )),
        ContractTypeRef::Nullable { inner } => {
            let inner = resolved_contract_type(inner, alias)?;
            let text = format!("{inner}?");
            Ok(ResolvedTypeRef::with_text(
                TypeRefIr::Nullable {
                    inner: Box::new(inner.ir),
                },
                text,
            ))
        }
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            let interface = resolved_contract_type(interface, alias)?;
            Ok(ResolvedTypeRef::with_text(
                TypeRefIr::AnyInterface {
                    interface: skiff_artifact_model::InterfaceInstantiationRef {
                        interface_abi_id: type_ref_abi_key(&interface.ir),
                        canonical_type_args: arguments
                            .iter()
                            .map(|argument| {
                                resolved_contract_type(argument, alias).map(|value| value.ir)
                            })
                            .collect::<Result<_, _>>()?,
                    },
                },
                format!("any {interface}"),
            ))
        }
        ContractTypeRef::Record { .. } => {
            Err("inline contract record has no exact source type representation".to_string())
        }
        ContractTypeRef::StructuralUnion { .. } => Err(
            "inline contract structural union has no exact source type representation".to_string(),
        ),
        ContractTypeRef::Literal { .. } => {
            Err("inline contract literal has no exact source type representation".to_string())
        }
    }
}
