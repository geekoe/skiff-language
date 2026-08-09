use crate::{
    ContractLiteral, ContractTypeDescriptor, ContractTypeRef, LiteralIr, NamedUnionBranchIr,
    NativeValueLifecycleResolution, PackageSchemaTypeRecord, TypeDescriptorIr, TypeRefIr,
};

use super::{
    classify::{classify_type, contract_aggregate, require_ordinary_snapshot, snapshot_root},
    contract::{PositionalTypeEnvironment, ValueLifecycleFactResolver, ValueLifecyclePolicyError},
    normalize::normalize_type,
    traversal::{ClassificationContext, TraversalState},
};

pub(super) fn classify_package_symbol<R: ValueLifecycleFactResolver>(
    symbol: &crate::PackageSymbolRef,
    arguments: &[TypeRefIr],
    outer_environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    let arguments = arguments
        .iter()
        .map(|argument| normalize_type(argument, outer_environment, context.budget, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    let key = TraversalState::exact_key("packageSymbol", &(symbol, &arguments))?;
    if let Some(cached) = context.state.begin(&key)? {
        return Ok(cached);
    }
    let resolved = context
        .resolver
        .resolve_package_symbol(symbol)
        .map_err(|source| ValueLifecyclePolicyError::Authority { source });
    let result = resolved.and_then(|resolved| {
        let environment = PositionalTypeEnvironment::new(resolved.type_parameters, arguments)?;
        classify_descriptor(&resolved.descriptor, &environment, context, depth + 1)
    });
    context.state.finish(key, &result);
    result
}

pub(super) fn classify_package_schema<R: ValueLifecycleFactResolver>(
    package_id: &str,
    stable_schema_key: &str,
    package_schema_type_id: &crate::PackageSchemaTypeId,
    arguments: &[TypeRefIr],
    outer_environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    let arguments = arguments
        .iter()
        .map(|argument| normalize_type(argument, outer_environment, context.budget, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    let key = TraversalState::exact_key(
        "packageSchema",
        &(
            package_id,
            stable_schema_key,
            package_schema_type_id,
            &arguments,
        ),
    )?;
    if let Some(cached) = context.state.begin(&key)? {
        return Ok(cached);
    }
    let record = context
        .resolver
        .resolve_package_schema(package_id, stable_schema_key, package_schema_type_id)
        .map_err(|source| ValueLifecyclePolicyError::Authority { source });
    let result = record.and_then(|record| {
        verify_schema_identity(
            &record,
            package_id,
            stable_schema_key,
            package_schema_type_id,
        )?;
        let environment =
            PositionalTypeEnvironment::new(record.canonical_descriptor.type_params, arguments)?;
        classify_contract_descriptor(
            &record.canonical_descriptor.descriptor,
            &environment,
            context,
            depth + 1,
        )
    });
    context.state.finish(key, &result);
    result
}

fn verify_schema_identity(
    record: &PackageSchemaTypeRecord,
    package_id: &str,
    stable_schema_key: &str,
    package_schema_type_id: &crate::PackageSchemaTypeId,
) -> Result<(), ValueLifecyclePolicyError> {
    if record.package_id != package_id
        || record.stable_schema_key != stable_schema_key
        || record.package_schema_type_id != *package_schema_type_id
    {
        return Err(ValueLifecyclePolicyError::AuthorityMismatch {
            message: "resolved PackageSchema record does not match the requested exact identity"
                .to_string(),
        });
    }
    Ok(())
}

fn classify_descriptor<R: ValueLifecycleFactResolver>(
    descriptor: &TypeDescriptorIr,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    context.budget.charge(descriptor, depth)?;
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            super::classify::aggregate(fields.values(), environment, context, depth)
        }
        TypeDescriptorIr::Representation { representation } => {
            let resolution = classify_type(representation, environment, context, depth + 1)?;
            require_ordinary_snapshot(resolution.clone(), 0)?;
            Ok(resolution)
        }
        TypeDescriptorIr::Union { branches } => {
            for (ordinal, branch) in branches.iter().enumerate() {
                let resolution = match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        classify_type(nominal_type, environment, context, depth + 1)?
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        classify_type(payload_type, environment, context, depth + 1)?
                    }
                    NamedUnionBranchIr::Literal { value } => classify_type(
                        &TypeRefIr::Literal {
                            value: value.clone(),
                        },
                        environment,
                        context,
                        depth + 1,
                    )?,
                };
                require_ordinary_snapshot(resolution, ordinal)?;
            }
            Ok(snapshot_root())
        }
        TypeDescriptorIr::Alias { target } => {
            classify_type(target, environment, context, depth + 1)
        }
        TypeDescriptorIr::Interface => Err(ValueLifecyclePolicyError::UnsupportedType {
            kind: "bareInterfaceDescriptor",
        }),
    }
}

fn classify_contract_descriptor<R: ValueLifecycleFactResolver>(
    descriptor: &ContractTypeDescriptor,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    context.budget.charge(descriptor, depth)?;
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            contract_aggregate(fields.values(), environment, context, depth)
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            contract_aggregate(variants.iter(), environment, context, depth)
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => contract_aggregate(
            branches.iter().map(|branch| &branch.branch_type),
            environment,
            context,
            depth,
        ),
        ContractTypeDescriptor::Representation { target } => {
            let resolution = classify_contract_type(target, environment, context, depth + 1)?;
            require_ordinary_snapshot(resolution.clone(), 0)?;
            Ok(resolution)
        }
        ContractTypeDescriptor::Alias { target } => {
            classify_contract_type(target, environment, context, depth + 1)
        }
        ContractTypeDescriptor::Enumeration { .. } => Ok(snapshot_root()),
        ContractTypeDescriptor::CallbackInterface { .. } => {
            Err(ValueLifecyclePolicyError::UnsupportedType {
                kind: "bareCallbackInterfaceDescriptor",
            })
        }
    }
}

pub(super) fn classify_contract_type<R: ValueLifecycleFactResolver>(
    ty: &ContractTypeRef,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    context.budget.charge(ty, depth)?;
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            let arguments = arguments
                .iter()
                .map(contract_type_to_type_ref)
                .collect::<Result<Vec<_>, _>>()?;
            classify_type(
                &TypeRefIr::Builtin {
                    name: name.clone(),
                    args: arguments,
                },
                environment,
                context,
                depth + 1,
            )
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => classify_package_schema(
            package_id,
            stable_schema_key,
            package_schema_type_id,
            &[],
            environment,
            context,
            depth,
        ),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            context
                .resolver
                .validate_contract_interface(interface, arguments)
                .map_err(|source| ValueLifecyclePolicyError::Authority { source })?;
            contract_aggregate(arguments.iter(), environment, context, depth)
        }
        ContractTypeRef::TypeParam { name } => environment
            .resolve(name)
            .ok_or_else(|| ValueLifecyclePolicyError::UnknownTypeParameter { name: name.clone() })
            .and_then(|value| classify_type(value, environment, context, depth + 1)),
        ContractTypeRef::Record { fields } => {
            contract_aggregate(fields.values(), environment, context, depth)
        }
        ContractTypeRef::StructuralUnion { variants } => {
            contract_aggregate(variants.iter(), environment, context, depth)
        }
        ContractTypeRef::Nullable { inner } => {
            require_ordinary_snapshot(
                classify_contract_type(inner, environment, context, depth + 1)?,
                0,
            )?;
            Ok(snapshot_root())
        }
        ContractTypeRef::Literal { value } => {
            let builtin = match value {
                ContractLiteral::String { .. } => "string",
            };
            classify_type(
                &TypeRefIr::builtin(builtin),
                environment,
                context,
                depth + 1,
            )
        }
    }
}

fn contract_type_to_type_ref(ty: &ContractTypeRef) -> Result<TypeRefIr, ValueLifecyclePolicyError> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => Ok(TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(contract_type_to_type_ref)
                .collect::<Result<_, _>>()?,
        }),
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        }),
        ContractTypeRef::TypeParam { name } => Ok(TypeRefIr::TypeParam { name: name.clone() }),
        ContractTypeRef::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, field)| Ok((name.clone(), contract_type_to_type_ref(field)?)))
                .collect::<Result<_, ValueLifecyclePolicyError>>()?,
        }),
        ContractTypeRef::StructuralUnion { variants } => Ok(TypeRefIr::Union {
            items: variants
                .iter()
                .map(contract_type_to_type_ref)
                .collect::<Result<_, _>>()?,
        }),
        ContractTypeRef::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(contract_type_to_type_ref(inner)?),
        }),
        ContractTypeRef::Literal { value } => Ok(TypeRefIr::Literal {
            value: match value {
                ContractLiteral::String { value } => LiteralIr::String {
                    value: value.clone(),
                },
            },
        }),
        ContractTypeRef::AnyInterface { .. } => Err(ValueLifecyclePolicyError::UnsupportedType {
            kind: "contractAnyInterfaceTypeExpression",
        }),
    }
}
