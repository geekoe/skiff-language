use crate::{
    LiteralIr, NativeValueDropPlan, NativeValueEmbedding, NativeValueLifecycleConcrete,
    NativeValueLifecycleResolution, NominalTypeRefBaseIr, TypeRefIr,
};

use super::{
    contract::{
        PositionalTypeEnvironment, ValueLifecycleFactResolver, ValueLifecyclePolicyBudget,
        ValueLifecyclePolicyError,
    },
    native::classify_native,
    normalize::normalize_type,
    schema::{classify_package_schema, classify_package_symbol},
    traversal::ClassificationContext,
};

pub fn normalize_value_lifecycle_type(
    ty: &TypeRefIr,
    environment: &PositionalTypeEnvironment,
    budget: &mut ValueLifecyclePolicyBudget,
) -> Result<TypeRefIr, ValueLifecyclePolicyError> {
    normalize_type(ty, environment, budget, 1)
}

pub fn classify_value_lifecycle<R: ValueLifecycleFactResolver>(
    ty: &TypeRefIr,
    environment: &PositionalTypeEnvironment,
    resolver: &mut R,
    budget: &mut ValueLifecyclePolicyBudget,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    let normalized = normalize_type(ty, environment, budget, 1)?;
    let mut context = ClassificationContext::new(resolver, budget);
    classify_type(
        &normalized,
        &PositionalTypeEnvironment::empty(),
        &mut context,
        1,
    )
}

pub(super) fn classify_type<R: ValueLifecycleFactResolver>(
    ty: &TypeRefIr,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    context.budget.charge(ty, depth)?;
    if let Some(result) = classify_native(ty, environment, context, depth)? {
        return Ok(result);
    }
    match ty {
        TypeRefIr::Record { fields } => aggregate(fields.values(), environment, context, depth),
        TypeRefIr::Union { items } => aggregate(items.iter(), environment, context, depth),
        TypeRefIr::Nullable { inner } => {
            require_ordinary_snapshot(classify_type(inner, environment, context, depth + 1)?, 0)?;
            Ok(snapshot_root())
        }
        TypeRefIr::Literal { value } => {
            let builtin = match value {
                LiteralIr::Null => "null",
                LiteralIr::Bool { .. } => "bool",
                LiteralIr::Number { .. } => "number",
                LiteralIr::String { .. } => "string",
            };
            classify_type(
                &TypeRefIr::builtin(builtin),
                environment,
                context,
                depth + 1,
            )
        }
        TypeRefIr::AnyInterface { interface } => {
            for (ordinal, argument) in interface.canonical_type_args.iter().enumerate() {
                require_ordinary_snapshot(
                    classify_type(argument, environment, context, depth + 1)?,
                    ordinal,
                )?;
            }
            context
                .resolver
                .validate_interface(interface)
                .map_err(|source| ValueLifecyclePolicyError::Authority { source })?;
            Ok(snapshot_root())
        }
        TypeRefIr::PackageSymbol { symbol } => {
            classify_package_symbol(symbol, &[], environment, context, depth)
        }
        TypeRefIr::PackageSchema {
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
        TypeRefIr::AppliedNominal { base, arguments } => match base {
            NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                classify_package_symbol(symbol, arguments, environment, context, depth)
            }
            NominalTypeRefBaseIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => classify_package_schema(
                package_id,
                stable_schema_key,
                package_schema_type_id,
                arguments,
                environment,
                context,
                depth,
            ),
            _ => Err(ValueLifecyclePolicyError::UnnormalizedOwner {
                kind: "appliedNominal",
            }),
        },
        TypeRefIr::TypeParam { name } => environment
            .resolve(name)
            .ok_or_else(|| ValueLifecyclePolicyError::UnknownTypeParameter { name: name.clone() })
            .and_then(|value| classify_type(value, environment, context, depth + 1)),
        TypeRefIr::Builtin { .. } => Err(ValueLifecyclePolicyError::UnsupportedType {
            kind: "unregisteredBuiltin",
        }),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. } => {
            Err(ValueLifecyclePolicyError::UnnormalizedOwner { kind: "owner" })
        }
        TypeRefIr::Function { .. } => {
            Err(ValueLifecyclePolicyError::UnsupportedType { kind: "function" })
        }
    }
}

pub(super) fn aggregate<'a, R: ValueLifecycleFactResolver>(
    children: impl IntoIterator<Item = &'a TypeRefIr>,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    for (ordinal, child) in children.into_iter().enumerate() {
        require_ordinary_snapshot(
            classify_type(child, environment, context, depth + 1)?,
            ordinal,
        )?;
    }
    Ok(snapshot_root())
}

pub(super) fn contract_aggregate<'a, R: ValueLifecycleFactResolver>(
    children: impl IntoIterator<Item = &'a crate::ContractTypeRef>,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError> {
    for (ordinal, child) in children.into_iter().enumerate() {
        require_ordinary_snapshot(
            super::schema::classify_contract_type(child, environment, context, depth + 1)?,
            ordinal,
        )?;
    }
    Ok(snapshot_root())
}

pub(super) fn require_ordinary_snapshot(
    resolution: NativeValueLifecycleResolution,
    ordinal: usize,
) -> Result<(), ValueLifecyclePolicyError> {
    if resolution.embedding != NativeValueEmbedding::Ordinary {
        return Err(ValueLifecyclePolicyError::ArgumentPolicy {
            ordinal,
            message: "child embedding is not Ordinary",
        });
    }
    if !matches!(
        resolution.lifecycle,
        NativeValueLifecycleConcrete::SnapshotShare { .. }
    ) {
        return Err(ValueLifecyclePolicyError::ArgumentPolicy {
            ordinal,
            message: "child lifecycle is not SnapshotShare",
        });
    }
    Ok(())
}

pub(super) fn snapshot_root() -> NativeValueLifecycleResolution {
    NativeValueLifecycleResolution {
        lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
            drop: NativeValueDropPlan::SnapshotRelease,
        },
        embedding: NativeValueEmbedding::Ordinary,
    }
}
