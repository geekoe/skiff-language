use crate::{
    native_value_lifecycle_registry, NativeValueArgumentPolicy, NativeValueLifecycleConcrete,
    NativeValueLifecycleResolution, NativeValueLifecycleTemplate, NativeValueTypeConstructor,
    NominalTypeRefBaseIr, PackageRefIr, TypeRefIr,
};

use super::{
    classify::{classify_type, require_ordinary_snapshot},
    contract::{PositionalTypeEnvironment, ValueLifecycleFactResolver, ValueLifecyclePolicyError},
    traversal::ClassificationContext,
};

pub(super) fn classify_native<R: ValueLifecycleFactResolver>(
    ty: &TypeRefIr,
    environment: &PositionalTypeEnvironment,
    context: &mut ClassificationContext<'_, R>,
    depth: u32,
) -> Result<Option<NativeValueLifecycleResolution>, ValueLifecyclePolicyError> {
    let Some((constructor, arguments)) = native_constructor(ty)? else {
        return Ok(None);
    };
    let entries = native_value_lifecycle_registry().entries();
    let matching_constructor = entries
        .iter()
        .filter(|entry| entry.pattern.constructor == constructor)
        .collect::<Vec<_>>();
    if matching_constructor.is_empty() {
        return Ok(None);
    }
    let entry = matching_constructor
        .into_iter()
        .find(|entry| entry.pattern.argument_policies.len() == arguments.len())
        .ok_or_else(|| ValueLifecyclePolicyError::GenericArity {
            expected: entries
                .iter()
                .find(|entry| entry.pattern.constructor == constructor)
                .map_or(0, |entry| entry.pattern.argument_policies.len()),
            actual: arguments.len(),
        })?;
    let mut resolved = vec![None; arguments.len()];
    for (ordinal, (argument, policy)) in arguments
        .iter()
        .zip(&entry.pattern.argument_policies)
        .enumerate()
    {
        if *policy == NativeValueArgumentPolicy::RequireSnapshotShare {
            let resolution = classify_type(argument, environment, context, depth + 1)?;
            require_ordinary_snapshot(resolution.clone(), ordinal)?;
            resolved[ordinal] = Some(resolution.lifecycle);
        }
    }
    let lifecycle = match &entry.lifecycle {
        NativeValueLifecycleTemplate::SnapshotShare { drop } => {
            NativeValueLifecycleConcrete::SnapshotShare { drop: drop.clone() }
        }
        NativeValueLifecycleTemplate::MoveOnly { drop } => {
            NativeValueLifecycleConcrete::MoveOnly { drop: drop.clone() }
        }
        NativeValueLifecycleTemplate::AffineResource { drop } => {
            NativeValueLifecycleConcrete::AffineResource { drop: drop.clone() }
        }
        NativeValueLifecycleTemplate::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter: clone_adapter.clone(),
            drop: drop.clone(),
        },
        NativeValueLifecycleTemplate::FromType { argument_index } => resolved
            .get(*argument_index as usize)
            .and_then(Clone::clone)
            .ok_or(ValueLifecyclePolicyError::ArgumentPolicy {
                ordinal: *argument_index as usize,
                message: "FromType argument was not classified",
            })?,
    };
    Ok(Some(NativeValueLifecycleResolution {
        lifecycle,
        embedding: entry.embedding,
    }))
}

fn native_constructor(
    ty: &TypeRefIr,
) -> Result<Option<(NativeValueTypeConstructor, &[TypeRefIr])>, ValueLifecyclePolicyError> {
    match ty {
        TypeRefIr::Builtin { name, args } => Ok(Some((
            NativeValueTypeConstructor::Builtin { name: name.clone() },
            args,
        ))),
        TypeRefIr::PackageSymbol { symbol } => Ok(Some((package_constructor(symbol)?, &[]))),
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol { symbol },
            arguments,
        } => Ok(Some((package_constructor(symbol)?, arguments))),
        TypeRefIr::AppliedNominal { .. } => Ok(None),
        _ => Ok(None),
    }
}

fn package_constructor(
    symbol: &crate::PackageSymbolRef,
) -> Result<NativeValueTypeConstructor, ValueLifecyclePolicyError> {
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        return Err(ValueLifecyclePolicyError::UnnormalizedOwner {
            kind: "dependencyPackageRef",
        });
    };
    let abi_identity = symbol
        .abi_expectation
        .as_deref()
        .filter(|identity| !identity.is_empty())
        .ok_or_else(|| ValueLifecyclePolicyError::MissingAbiIdentity {
            symbol_path: symbol.symbol_path.clone(),
        })?;
    Ok(NativeValueTypeConstructor::PackageSymbol {
        package_id: package_id.clone(),
        symbol_path: symbol.symbol_path.clone(),
        abi_identity: abi_identity.to_string(),
    })
}
