//! Validation for retained actor declaration authority.

use std::collections::BTreeSet;

use skiff_artifact_model::{ExecutableKind, FileIrUnit};

use crate::mir::MirBuildError;

pub(super) fn validate_actor_declarations(unit: &FileIrUnit) -> Result<(), MirBuildError> {
    let mut abi_identities = BTreeSet::new();
    let mut implementation_identities = BTreeSet::new();
    for declaration in &unit.actor_declarations {
        let canonical_abi =
            skiff_artifact_identity::actor_abi_identity(&declaration.abi).map_err(|error| {
                MirBuildError::InvalidActorDeclaration {
                    module_path: unit.module_path.clone(),
                    actor: declaration.abi.actor_name.clone(),
                    message: format!("cannot compute canonical actor ABI identity: {error}"),
                }
            })?;
        if declaration.actor_abi_identity != canonical_abi {
            return Err(MirBuildError::InvalidActorDeclaration {
                module_path: unit.module_path.clone(),
                actor: declaration.abi.actor_name.clone(),
                message: "stored actor ABI identity is not canonical for its owned ABI".to_string(),
            });
        }
        if !abi_identities.insert(declaration.actor_abi_identity.as_str()) {
            return Err(MirBuildError::InvalidActorDeclaration {
                module_path: unit.module_path.clone(),
                actor: declaration.abi.actor_name.clone(),
                message: "duplicate actor ABI identity".to_string(),
            });
        }
        if !implementation_identities.insert(declaration.actor_implementation_identity.as_str()) {
            return Err(MirBuildError::InvalidActorDeclaration {
                module_path: unit.module_path.clone(),
                actor: declaration.abi.actor_name.clone(),
                message: "duplicate actor implementation identity".to_string(),
            });
        }
        let public_methods = declaration
            .abi
            .public_methods
            .iter()
            .map(|method| method.method_identity.clone())
            .collect::<BTreeSet<_>>();
        let implementation_methods = declaration
            .method_implementations
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if public_methods != implementation_methods {
            return Err(MirBuildError::InvalidActorDeclaration {
                module_path: unit.module_path.clone(),
                actor: declaration.abi.actor_name.clone(),
                message: "public method identities do not exactly match implementation rows"
                    .to_string(),
            });
        }
        if declaration.abi.create.is_some() != declaration.create_implementation.is_some() {
            return Err(MirBuildError::InvalidActorDeclaration {
                module_path: unit.module_path.clone(),
                actor: declaration.abi.actor_name.clone(),
                message: "create signature and implementation row presence disagree".to_string(),
            });
        }
        if declaration
            .create_implementation
            .as_ref()
            .is_some_and(|create| public_methods.contains(&create.identity))
        {
            return Err(MirBuildError::InvalidActorDeclaration {
                module_path: unit.module_path.clone(),
                actor: declaration.abi.actor_name.clone(),
                message: "create identity aliases a public method identity".to_string(),
            });
        }
        let mut executable_indices = BTreeSet::new();
        for executable_index in declaration.method_implementations.values().copied().chain(
            declaration
                .create_implementation
                .iter()
                .map(|create| create.executable_index),
        ) {
            let executable = unit
                .executables
                .get(executable_index as usize)
                .ok_or_else(|| MirBuildError::InvalidActorDeclaration {
                    module_path: unit.module_path.clone(),
                    actor: declaration.abi.actor_name.clone(),
                    message: format!(
                        "implementation references missing executable {executable_index}"
                    ),
                })?;
            if executable.kind != ExecutableKind::ImplMethod {
                return Err(MirBuildError::InvalidActorDeclaration {
                    module_path: unit.module_path.clone(),
                    actor: declaration.abi.actor_name.clone(),
                    message: format!(
                        "implementation executable {executable_index} is not an ImplMethod"
                    ),
                });
            }
            if !executable_indices.insert(executable_index) {
                return Err(MirBuildError::InvalidActorDeclaration {
                    module_path: unit.module_path.clone(),
                    actor: declaration.abi.actor_name.clone(),
                    message: format!(
                        "more than one actor row references executable {executable_index}"
                    ),
                });
            }
        }
    }
    Ok(())
}
