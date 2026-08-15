use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    OperationCallableKind, OperationTargetRef, PackageArtifact, PackageCallableId,
    PackageExecutableCoordinate, PackageLocalAbiSymbol,
};
use thiserror::Error;

/// Exact canonical-callable resolution against the compiler-owned PackageArtifact manifest.
#[derive(Debug, Error)]
pub enum CanonicalCallableError {
    #[error("package callable {callable_id} has no exact callableLinks entry")]
    MissingCallableLink { callable_id: PackageCallableId },
    #[error("package callable {callable_id} has an inconsistent callable link: {message}")]
    InvalidCallableLink {
        callable_id: PackageCallableId,
        message: String,
    },
    #[error(
        "public callable {callable_id} has no canonical implementation callable for executable coordinate {coordinate:?}"
    )]
    MissingCanonicalImplementation {
        callable_id: PackageCallableId,
        coordinate: PackageExecutableCoordinate,
    },
    #[error(
        "public callable {callable_id} has ambiguous canonical implementation callables {first} and {second}"
    )]
    AmbiguousCanonicalImplementation {
        callable_id: PackageCallableId,
        first: PackageCallableId,
        second: PackageCallableId,
    },
    #[error(
        "canonical callable {callable_id} has no public callable for executable coordinate {coordinate:?}"
    )]
    MissingPublicCallable {
        callable_id: PackageCallableId,
        coordinate: PackageExecutableCoordinate,
    },
    #[error(
        "canonical callable {callable_id} is ambiguous among public callables {candidates:?} for executable coordinate {coordinate:?}"
    )]
    AmbiguousPublicCallable {
        callable_id: PackageCallableId,
        candidates: Vec<PackageCallableId>,
        coordinate: PackageExecutableCoordinate,
    },
}

/// Resolves one public alias to its exact canonical implementation callable.
///
/// The resolution reads only the compiler-owned `implementationSymbols` and
/// `callableLinks` manifests; it does not derive identities from source paths
/// or guess an implementation owner.
pub fn canonical_implementation_callable(
    implementation: &PackageArtifact,
    public_callable: &PackageCallableId,
) -> Result<PackageCallableId, CanonicalCallableError> {
    let public_target = exact_callable_target(implementation, public_callable)?;
    let public_coordinate = executable_coordinate(&public_target);
    let index = CanonicalImplementationCallableIndex::checked(implementation, public_callable)?;
    index
        .by_coordinate
        .get(&public_coordinate)
        .cloned()
        .ok_or_else(|| CanonicalCallableError::MissingCanonicalImplementation {
            callable_id: public_callable.clone(),
            coordinate: public_coordinate,
        })
}

/// Recovers the public callable whose exact boundary facts correspond to one
/// canonical operation binding.
///
/// When the contract stable key names a public symbol, that exact alias wins.
/// Otherwise a unique public callable sharing the same executable coordinate
/// is accepted; multiple aliases fail closed instead of guessing facts.
pub fn canonical_binding_public_callable(
    implementation: &PackageArtifact,
    stable_key: &str,
    canonical_callable: &PackageCallableId,
) -> Result<PackageCallableId, CanonicalCallableError> {
    let canonical_target = exact_callable_target(implementation, canonical_callable)?;
    let canonical_coordinate = executable_coordinate(&canonical_target);

    if let Some(public_callable) = public_callable_for_path(implementation, stable_key) {
        if let Ok(target) = exact_callable_target(implementation, &public_callable) {
            if executable_coordinate(&target) == canonical_coordinate {
                return Ok(public_callable);
            }
        }
    }

    let candidates = public_callables_for_coordinate(implementation, &canonical_coordinate)?;
    match candidates.as_slice() {
        [] => Err(CanonicalCallableError::MissingPublicCallable {
            callable_id: canonical_callable.clone(),
            coordinate: canonical_coordinate,
        }),
        [public_callable] => Ok(public_callable.clone()),
        _ => Err(CanonicalCallableError::AmbiguousPublicCallable {
            callable_id: canonical_callable.clone(),
            candidates,
            coordinate: canonical_coordinate,
        }),
    }
}

fn exact_callable_target(
    implementation: &PackageArtifact,
    callable_id: &PackageCallableId,
) -> Result<OperationTargetRef, CanonicalCallableError> {
    let link = implementation
        .callable_links
        .get(callable_id)
        .ok_or_else(|| CanonicalCallableError::MissingCallableLink {
            callable_id: callable_id.clone(),
        })?;
    if link.callable_id != *callable_id {
        return Err(CanonicalCallableError::InvalidCallableLink {
            callable_id: callable_id.clone(),
            message: format!("nested callable id is {}", link.callable_id),
        });
    }
    if link.target.callable_abi_id != callable_id.as_str() {
        return Err(CanonicalCallableError::InvalidCallableLink {
            callable_id: callable_id.clone(),
            message: format!("target callable ABI id is {}", link.target.callable_abi_id),
        });
    }
    Ok(link.target.clone())
}

fn public_callable_for_path(
    implementation: &PackageArtifact,
    path: &str,
) -> Option<PackageCallableId> {
    match implementation.package_local_abi.public_symbols.get(path) {
        Some(PackageLocalAbiSymbol::Callable { callable_id, .. }) => Some(callable_id.clone()),
        _ => None,
    }
}

fn public_callables_for_coordinate(
    implementation: &PackageArtifact,
    coordinate: &PackageExecutableCoordinate,
) -> Result<Vec<PackageCallableId>, CanonicalCallableError> {
    let mut candidates = BTreeSet::new();
    for symbol in implementation.package_local_abi.public_symbols.values() {
        match symbol {
            PackageLocalAbiSymbol::Callable { callable_id, .. } => {
                if executable_coordinate(&exact_callable_target(implementation, callable_id)?)
                    == *coordinate
                {
                    candidates.insert(callable_id.clone());
                }
            }
            PackageLocalAbiSymbol::PublicInstance { methods, .. } => {
                for callable_id in methods.values() {
                    if executable_coordinate(&exact_callable_target(implementation, callable_id)?)
                        == *coordinate
                    {
                        candidates.insert(callable_id.clone());
                    }
                }
            }
            PackageLocalAbiSymbol::Type { .. } | PackageLocalAbiSymbol::Constant { .. } => {}
        }
    }
    Ok(candidates.into_iter().collect())
}

fn executable_coordinate(target: &OperationTargetRef) -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: target.file_ref.file_ir_identity.clone(),
        module_path: target.file_ref.module_path.clone(),
        executable_index: target.executable_index,
    }
}

struct CanonicalImplementationCallableIndex {
    by_coordinate: BTreeMap<PackageExecutableCoordinate, PackageCallableId>,
}

impl CanonicalImplementationCallableIndex {
    fn checked(
        implementation: &PackageArtifact,
        public_callable: &PackageCallableId,
    ) -> Result<Self, CanonicalCallableError> {
        let mut by_coordinate = BTreeMap::new();
        for symbol in implementation
            .package_local_abi
            .implementation_symbols
            .values()
        {
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
                continue;
            };
            let link = implementation
                .callable_links
                .get(callable_id)
                .ok_or_else(|| CanonicalCallableError::InvalidCallableLink {
                    callable_id: callable_id.clone(),
                    message: "canonical implementation callable has no callable link".to_string(),
                })?;
            if link.callable_id != *callable_id {
                return Err(CanonicalCallableError::InvalidCallableLink {
                    callable_id: callable_id.clone(),
                    message: format!(
                        "canonical implementation nested callable id is {}",
                        link.callable_id
                    ),
                });
            }
            if link.target.callable_abi_id != callable_id.as_str() {
                return Err(CanonicalCallableError::InvalidCallableLink {
                    callable_id: callable_id.clone(),
                    message: format!(
                        "canonical implementation target callable ABI id is {}",
                        link.target.callable_abi_id
                    ),
                });
            }
            if !matches!(
                link.target.callable_kind,
                OperationCallableKind::InternalFunction | OperationCallableKind::ImplMethod
            ) {
                return Err(CanonicalCallableError::InvalidCallableLink {
                    callable_id: callable_id.clone(),
                    message: format!(
                        "canonical implementation target kind is {:?}",
                        link.target.callable_kind
                    ),
                });
            }
            let coordinate = executable_coordinate(&link.target);
            if let Some(previous) = by_coordinate.insert(coordinate, callable_id.clone()) {
                return Err(CanonicalCallableError::AmbiguousCanonicalImplementation {
                    callable_id: public_callable.clone(),
                    first: previous,
                    second: callable_id.clone(),
                });
            }
        }
        Ok(Self { by_coordinate })
    }
}
