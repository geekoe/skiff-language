use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    FileIrUnit, PackageActorCreateBinding, PackageActorImplementation, ServiceSymbolRef,
    TypeDescriptorIr,
};

use crate::error::ProjectionError;

use super::{projection_error, ImplementationCallableIndex};

pub(super) fn project_actor_implementations(
    package_id: &str,
    units: &[FileIrUnit],
    callables: &ImplementationCallableIndex,
) -> Result<Vec<PackageActorImplementation>, ProjectionError> {
    let mut actors = BTreeMap::new();
    for unit in units {
        for declaration in &unit.actor_declarations {
            validate_actor_declaration(package_id, unit, declaration)?;
            let actor = ServiceSymbolRef {
                module_path: unit.module_path.clone(),
                symbol: declaration.abi.actor_name.clone(),
            };
            let actor_label = format!("actor {}", actor.symbol_path());
            let methods = declaration
                .method_implementations
                .iter()
                .map(|(method_identity, executable_index)| {
                    Ok((
                        method_identity.clone(),
                        callables.actor_method(
                            package_id,
                            unit,
                            *executable_index,
                            &format!("{actor_label} method {method_identity:?}"),
                        )?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?;
            let create = declaration
                .create_implementation
                .as_ref()
                .map(|create| {
                    Ok(PackageActorCreateBinding {
                        method_identity: create.identity.clone(),
                        package_callable_id: callables.actor_method(
                            package_id,
                            unit,
                            create.executable_index,
                            &format!("{actor_label} create {:?}", create.identity),
                        )?,
                    })
                })
                .transpose()?;
            let row = PackageActorImplementation {
                actor: actor.clone(),
                actor_implementation_identity: declaration.actor_implementation_identity.clone(),
                methods,
                create,
            };
            let key = (actor.module_path.clone(), actor.symbol.clone());
            if actors.insert(key, row).is_some() {
                return Err(projection_error(
                    package_id,
                    format!("duplicate actor implementation authority for {actor_label}"),
                ));
            }
        }
    }
    Ok(actors.into_values().collect())
}

fn validate_actor_declaration(
    package_id: &str,
    unit: &FileIrUnit,
    declaration: &skiff_artifact_model::ActorDeclarationIr,
) -> Result<(), ProjectionError> {
    let actor_name = declaration.abi.actor_name.as_str();
    let type_declaration = unit.declarations.types.get(actor_name).ok_or_else(|| {
        projection_error(
            package_id,
            format!(
                "actor {}.{actor_name} has no exact attached type declaration",
                unit.module_path
            ),
        )
    })?;
    let attached_type = unit
        .type_table
        .get(type_declaration.type_index as usize)
        .ok_or_else(|| {
            projection_error(
                package_id,
                format!(
                    "actor {}.{actor_name} attached type index {} is missing",
                    unit.module_path, type_declaration.type_index
                ),
            )
        })?;
    if attached_type.name.as_str() != actor_name
        || !matches!(&attached_type.descriptor, TypeDescriptorIr::Record { .. })
    {
        return Err(projection_error(
            package_id,
            format!(
                "actor {}.{actor_name} must attach to its exact nominal record declaration",
                unit.module_path
            ),
        ));
    }

    let mut abi_methods = BTreeSet::new();
    for method in &declaration.abi.public_methods {
        if !abi_methods.insert(method.method_identity.clone()) {
            return Err(projection_error(
                package_id,
                format!(
                    "actor {}.{actor_name} repeats ABI method identity {:?}",
                    unit.module_path, method.method_identity
                ),
            ));
        }
    }
    let implementation_methods = declaration
        .method_implementations
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if abi_methods != implementation_methods {
        return Err(projection_error(
            package_id,
            format!(
                "actor {}.{actor_name} ABI method identities do not exactly match implementation identities",
                unit.module_path
            ),
        ));
    }
    match (
        declaration.abi.create.as_ref(),
        declaration.create_implementation.as_ref(),
    ) {
        (None, None) | (Some(_), Some(_)) => {}
        _ => {
            return Err(projection_error(
                package_id,
                format!(
                    "actor {}.{actor_name} create ABI and implementation presence disagree",
                    unit.module_path
                ),
            ));
        }
    }
    if declaration
        .create_implementation
        .as_ref()
        .is_some_and(|create| abi_methods.contains(&create.identity))
    {
        return Err(projection_error(
            package_id,
            format!(
                "actor {}.{actor_name} create identity must be independent from public method identities",
                unit.module_path
            ),
        ));
    }
    Ok(())
}
