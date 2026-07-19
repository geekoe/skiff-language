use std::collections::{BTreeMap, BTreeSet, VecDeque};

use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, PackageBuildId, PackageRequirementKey,
    ServiceDeploymentInput,
};

use super::{ProjectionError, ProjectionResult};

pub(super) struct PackageClosure<'a> {
    by_build: BTreeMap<PackageBuildId, &'a PackageArtifact>,
}

impl<'a> PackageClosure<'a> {
    pub(super) fn resolve(
        input: &ServiceDeploymentInput,
        artifacts: &'a [PackageArtifact],
    ) -> ProjectionResult<Self> {
        let mut by_build = BTreeMap::new();
        for artifact in artifacts {
            skiff_artifact_identity::validate_package_artifact_identities(artifact).map_err(
                |error| ProjectionError::InvalidTypedArtifact {
                    artifact: "PackageArtifact",
                    identity_error: error,
                },
            )?;
            if by_build
                .insert(artifact.package_build_id.clone(), artifact)
                .is_some()
            {
                return Err(ProjectionError::DuplicatePackageBuild {
                    build_id: artifact.package_build_id.clone(),
                });
            }
        }

        let implementation = by_build
            .get(&input.implementation.package_build_id)
            .copied()
            .ok_or_else(|| ProjectionError::MissingImplementation {
                build_id: input.implementation.package_build_id.clone(),
            })?;
        validate_package_ref(&input.implementation, implementation)?;

        let mut bindings = BTreeMap::new();
        for binding in &input.package_bindings {
            if bindings.insert(binding.key.clone(), binding).is_some() {
                return Err(ProjectionError::ConflictingRequirement {
                    kind: "package",
                    key: package_key(&binding.key),
                    message: "binding key is repeated".to_string(),
                });
            }
        }

        let mut reachable = BTreeSet::new();
        let mut pending = VecDeque::from([implementation.package_build_id.clone()]);
        while let Some(caller_build_id) = pending.pop_front() {
            if !reachable.insert(caller_build_id.clone()) {
                continue;
            }
            let caller = by_build.get(&caller_build_id).copied().ok_or_else(|| {
                ProjectionError::MissingRequirementBinding {
                    kind: "package artifact",
                    key: caller_build_id.to_string(),
                }
            })?;
            let expected_aliases = caller
                .package_requirements
                .iter()
                .map(|requirement| requirement.alias.as_str())
                .collect::<BTreeSet<_>>();

            for requirement in &caller.package_requirements {
                let key = PackageRequirementKey {
                    caller_package_build_id: caller_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                };
                let binding = bindings.get(&key).copied().ok_or_else(|| {
                    ProjectionError::MissingRequirementBinding {
                        kind: "package",
                        key: package_key(&key),
                    }
                })?;
                let target = by_build
                    .get(&binding.package.package_build_id)
                    .copied()
                    .ok_or_else(|| ProjectionError::MissingRequirementBinding {
                        kind: "package artifact",
                        key: binding.package.package_build_id.to_string(),
                    })?;
                validate_package_ref(&binding.package, target)?;
                validate_requirement_target(requirement, &binding.package, &key)?;
                pending.push_back(target.package_build_id.clone());
            }

            for key in bindings.keys().filter(|key| {
                key.caller_package_build_id == caller_build_id
                    && !expected_aliases.contains(key.package_requirement_alias.as_str())
            }) {
                return Err(ProjectionError::ExtraRequirementBinding {
                    kind: "package",
                    key: package_key(key),
                });
            }
        }

        for key in bindings.keys() {
            if !reachable.contains(&key.caller_package_build_id) {
                return Err(ProjectionError::ExtraRequirementBinding {
                    kind: "package",
                    key: package_key(key),
                });
            }
        }
        for build_id in by_build.keys() {
            if !reachable.contains(build_id) {
                return Err(ProjectionError::UnreachablePackage {
                    build_id: build_id.clone(),
                });
            }
        }

        Ok(Self { by_build })
    }

    pub(super) fn implementation(&self, input: &ServiceDeploymentInput) -> &'a PackageArtifact {
        self.by_build[&input.implementation.package_build_id]
    }

    pub(super) fn artifacts(&self) -> impl Iterator<Item = &'a PackageArtifact> + '_ {
        self.by_build.values().copied()
    }
}

fn validate_package_ref(
    reference: &PackageArtifactRef,
    artifact: &PackageArtifact,
) -> ProjectionResult<()> {
    for (field, expected, actual) in [
        (
            "packageId",
            artifact.package_id.as_str(),
            reference.package_id.as_str(),
        ),
        (
            "packageVersion",
            artifact.package_version.as_str(),
            reference.package_version.as_str(),
        ),
        (
            "packageLocalAbiIdentity",
            artifact.package_local_abi.local_abi_identity.as_str(),
            reference.package_local_abi_identity.as_str(),
        ),
    ] {
        if expected != actual {
            return Err(ProjectionError::PackageReferenceMismatch {
                build_id: artifact.package_build_id.clone(),
                field,
                expected: expected.to_string(),
                actual: actual.to_string(),
            });
        }
    }
    Ok(())
}

fn validate_requirement_target(
    requirement: &skiff_artifact_model::PackageRequirement,
    target: &PackageArtifactRef,
    key: &PackageRequirementKey,
) -> ProjectionResult<()> {
    let mismatch = if target.package_id != requirement.package_id {
        Some(format!(
            "packageId must be {}, got {}",
            requirement.package_id, target.package_id
        ))
    } else if target.package_version != requirement.exact_version {
        Some(format!(
            "packageVersion must be {}, got {}",
            requirement.exact_version, target.package_version
        ))
    } else if target.package_local_abi_identity != requirement.expected_local_abi {
        Some(format!(
            "packageLocalAbiIdentity must be {}, got {}",
            requirement.expected_local_abi, target.package_local_abi_identity
        ))
    } else {
        None
    };
    if let Some(message) = mismatch {
        return Err(ProjectionError::RequirementBindingMismatch {
            kind: "package",
            key: package_key(key),
            message,
        });
    }
    Ok(())
}

pub(super) fn package_key(key: &PackageRequirementKey) -> String {
    format!(
        "{}:{}",
        key.caller_package_build_id, key.package_requirement_alias
    )
}
