use std::collections::BTreeMap;

use skiff_artifact_model::{
    PackageConfigRequirement, PackageRuntimeRequirements, PackageStateRequirement, StateBindingKind,
};
use skiff_compiler_projection_input::{
    ConfigRequirementAccessProjection, ConfigRequirementsSeed, ProjectionLoweringFacts,
};

use crate::error::ProjectionError;

pub(super) fn project_runtime_requirements(
    package_id: &str,
    requirements: &ConfigRequirementsSeed,
    declared_state: &[PackageStateRequirement],
    lowering: &ProjectionLoweringFacts,
) -> Result<PackageRuntimeRequirements, ProjectionError> {
    let mut config_by_path = BTreeMap::<String, PackageConfigRequirement>::new();
    for requirement in requirements.effective().requirements() {
        let (value_type, required) = match requirement.access() {
            ConfigRequirementAccessProjection::Require { ty } => (ty.clone(), true),
            ConfigRequirementAccessProjection::Optional { ty } => (ty.clone(), false),
            ConfigRequirementAccessProjection::Has => continue,
        };
        let projected = PackageConfigRequirement {
            path: requirement.path().to_string(),
            value_type,
            required,
        };
        match config_by_path.get(&projected.path) {
            Some(existing) if existing == &projected => {}
            Some(existing) => {
                return Err(ProjectionError::InvalidPackageArtifact {
                    message: format!(
                        "package {package_id} config requirement {} conflicts: {:?} vs {:?}",
                        projected.path, existing, projected
                    ),
                });
            }
            None => {
                config_by_path.insert(projected.path.clone(), projected);
            }
        }
    }
    let state = project_state_requirements(package_id, declared_state, lowering)?;
    Ok(PackageRuntimeRequirements {
        config: config_by_path.into_values().collect(),
        state,
        resources: Vec::new(),
        runtime_capabilities: Vec::new(),
    })
}

fn project_state_requirements(
    package_id: &str,
    declared: &[PackageStateRequirement],
    lowering: &ProjectionLoweringFacts,
) -> Result<Vec<PackageStateRequirement>, ProjectionError> {
    project_state_requirements_from_facts(
        package_id,
        declared,
        !lowering.service_db_metadata().is_empty(),
    )
}

fn project_state_requirements_from_facts(
    package_id: &str,
    declared: &[PackageStateRequirement],
    has_database_schema: bool,
) -> Result<Vec<PackageStateRequirement>, ProjectionError> {
    let mut by_key = BTreeMap::new();
    for requirement in declared {
        if requirement.key.trim().is_empty() {
            return Err(invalid_state(
                package_id,
                "contains an empty state requirement key",
            ));
        }
        if by_key
            .insert(requirement.key.clone(), requirement.clone())
            .is_some()
        {
            return Err(invalid_state(
                package_id,
                format!("repeats state requirement {}", requirement.key),
            ));
        }
    }

    let database_keys = by_key
        .values()
        .filter(|requirement| requirement.kind == StateBindingKind::Database)
        .map(|requirement| requirement.key.as_str())
        .collect::<Vec<_>>();
    match (database_keys.as_slice(), has_database_schema) {
        ([], true) => {
            return Err(invalid_state(
                package_id,
                "uses database schema but declares no database state requirement",
            ));
        }
        ([key], false) => {
            return Err(invalid_state(
                package_id,
                format!(
                    "declares database state requirement {key} but source has no database schema"
                ),
            ));
        }
        ([..], _) if database_keys.len() > 1 => {
            return Err(invalid_state(
                package_id,
                "declares more than one database state requirement; one activation has one database capability",
            ));
        }
        _ => {}
    }
    Ok(by_key.into_values().collect())
}

fn invalid_state(package_id: &str, message: impl Into<String>) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: format!("package {package_id} {}", message.into()),
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{PackageStateRequirement, StateBindingKind};

    use super::project_state_requirements_from_facts;

    fn state(key: &str, kind: StateBindingKind) -> PackageStateRequirement {
        PackageStateRequirement {
            key: key.to_string(),
            kind,
        }
    }

    #[test]
    fn database_state_requires_matching_source_schema_and_projects_exact_key() {
        let requirement = state("registry-store", StateBindingKind::Database);
        assert_eq!(
            project_state_requirements_from_facts(
                "skiff.run/registry",
                std::slice::from_ref(&requirement),
                true,
            )
            .unwrap(),
            vec![requirement]
        );

        let undeclared =
            project_state_requirements_from_facts("example.com/package", &[], true).unwrap_err();
        assert!(undeclared
            .to_string()
            .contains("uses database schema but declares no database state requirement"));

        let unused = project_state_requirements_from_facts(
            "example.com/package",
            &[state("db", StateBindingKind::Database)],
            false,
        )
        .unwrap_err();
        assert!(unused
            .to_string()
            .contains("declares database state requirement db but source has no database schema"));
    }

    #[test]
    fn database_state_rejects_ambiguous_keys_and_sorts_all_state_requirements() {
        let ambiguous = project_state_requirements_from_facts(
            "example.com/package",
            &[
                state("primary", StateBindingKind::Database),
                state("secondary", StateBindingKind::Database),
            ],
            true,
        )
        .unwrap_err();
        assert!(ambiguous
            .to_string()
            .contains("more than one database state requirement"));

        assert_eq!(
            project_state_requirements_from_facts(
                "example.com/package",
                &[
                    state("queue", StateBindingKind::Queue),
                    state("actor", StateBindingKind::Actor),
                ],
                false,
            )
            .unwrap()
            .into_iter()
            .map(|requirement| requirement.key)
            .collect::<Vec<_>>(),
            ["actor", "queue"]
        );
    }
}
