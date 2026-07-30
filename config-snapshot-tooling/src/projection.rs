use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde_json::Value;
use skiff_artifact_model::{
    ConfigShapeValueType, PackageBuildId, PackageConfigAccess, PackageConfigRequirement,
    RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};
use skiff_runtime_config_snapshot::{
    RuntimeConfigDeployment, RuntimeConfigPackage, RuntimeConfigSnapshot,
};

use crate::error::invalid;
use crate::{ConfigSnapshotToolingResult, ServiceConfigLayers};

#[derive(Debug, Clone)]
pub struct ConfigSnapshotPackageInput {
    pub package_id: String,
    pub package_build_id: PackageBuildId,
    pub requirements: Vec<PackageConfigRequirement>,
}

#[derive(Debug, Clone)]
pub struct ConfigSnapshotDeploymentInput {
    pub deployment: ServiceDeploymentRef,
    pub source_path: PathBuf,
    pub config: ServiceConfigLayers,
    pub packages: Vec<ConfigSnapshotPackageInput>,
}

pub fn project_runtime_config_snapshot(
    snapshot: RuntimeConfigSnapshotRef,
    inputs: Vec<ConfigSnapshotDeploymentInput>,
) -> ConfigSnapshotToolingResult<RuntimeConfigSnapshot> {
    project_runtime_config_snapshot_with_base(snapshot, Vec::new(), inputs)
}

pub fn project_runtime_config_snapshot_with_base(
    snapshot: RuntimeConfigSnapshotRef,
    mut base: Vec<RuntimeConfigDeployment>,
    mut inputs: Vec<ConfigSnapshotDeploymentInput>,
) -> ConfigSnapshotToolingResult<RuntimeConfigSnapshot> {
    inputs.sort_by(|left, right| left.deployment.cmp(&right.deployment));
    ensure_unique(
        inputs.iter().map(|input| &input.deployment),
        "<projection>",
        "deployment",
    )?;
    let mut deployments = inputs
        .into_iter()
        .map(project_deployment)
        .collect::<ConfigSnapshotToolingResult<Vec<_>>>()?;
    deployments.append(&mut base);
    deployments.sort_by(|left, right| left.deployment().cmp(right.deployment()));
    ensure_unique(
        deployments.iter().map(RuntimeConfigDeployment::deployment),
        "<projection>",
        "deployment",
    )?;
    RuntimeConfigSnapshot::new(snapshot, deployments).map_err(Into::into)
}

fn project_deployment(
    mut input: ConfigSnapshotDeploymentInput,
) -> ConfigSnapshotToolingResult<RuntimeConfigDeployment> {
    input.packages.sort_by(|left, right| {
        left.package_build_id
            .cmp(&right.package_build_id)
            .then_with(|| left.package_id.cmp(&right.package_id))
    });
    ensure_unique(
        input
            .packages
            .iter()
            .map(|package| &package.package_build_id),
        &input.source_path,
        "Package build",
    )?;
    ensure_unique(
        input.packages.iter().map(|package| &package.package_id),
        &input.source_path,
        "Package ID",
    )?;

    let expected = input
        .packages
        .iter()
        .map(|package| package.package_id.as_str())
        .collect::<BTreeSet<_>>();
    let unknown = input
        .config
        .keys()
        .filter(|package_id| !expected.contains(package_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(invalid(
            &input.source_path,
            format!(
                "config declares Package ID(s) outside the exact deployment closure: {}",
                unknown.join(", ")
            ),
        ));
    }

    let packages = input
        .packages
        .into_iter()
        .map(|package| {
            let config = input.config.remove(&package.package_id).unwrap_or_default();
            validate_requirements(
                &input.source_path,
                &package.package_id,
                &config,
                &package.requirements,
            )?;
            RuntimeConfigPackage::new(package.package_build_id, config).map_err(Into::into)
        })
        .collect::<ConfigSnapshotToolingResult<Vec<_>>>()?;
    RuntimeConfigDeployment::new(input.deployment, packages).map_err(Into::into)
}

fn validate_requirements(
    source: &std::path::Path,
    package_id: &str,
    config: &BTreeMap<String, Value>,
    requirements: &[PackageConfigRequirement],
) -> ConfigSnapshotToolingResult<()> {
    for requirement in requirements {
        let value = value_at_path(config, &requirement.path).map_err(|message| {
            invalid(
                source,
                format!("{package_id} config path {} {message}", requirement.path),
            )
        })?;
        match &requirement.access {
            PackageConfigAccess::Presence => {}
            PackageConfigAccess::Optional { value_type } => {
                if let Some(value) = value {
                    validate_value_type(source, package_id, &requirement.path, value_type, value)?;
                }
            }
            PackageConfigAccess::Required { value_type } => {
                let value = value.ok_or_else(|| {
                    invalid(
                        source,
                        format!(
                            "{package_id} is missing required config path {}",
                            requirement.path
                        ),
                    )
                })?;
                validate_value_type(source, package_id, &requirement.path, value_type, value)?;
            }
        }
    }
    Ok(())
}

fn value_at_path<'a>(
    config: &'a BTreeMap<String, Value>,
    path: &str,
) -> Result<Option<&'a Value>, &'static str> {
    let mut segments = path.split('.');
    let Some(first) = segments.next() else {
        return Ok(None);
    };
    let Some(mut value) = config.get(first) else {
        return Ok(None);
    };
    for segment in segments {
        let Value::Object(object) = value else {
            return Err("is nested below a non-object value");
        };
        let Some(next) = object.get(segment) else {
            return Ok(None);
        };
        value = next;
    }
    Ok(Some(value))
}

fn validate_value_type(
    source: &std::path::Path,
    package_id: &str,
    path: &str,
    value_type: &str,
    value: &Value,
) -> ConfigSnapshotToolingResult<()> {
    let ty = ConfigShapeValueType::try_from(value_type).map_err(|error| {
        invalid(
            source,
            format!("{package_id} config requirement {path} is invalid: {error}"),
        )
    })?;
    let valid = match ty {
        ConfigShapeValueType::String => value.is_string(),
        ConfigShapeValueType::Number => value.is_number(),
        ConfigShapeValueType::Bool => value.is_boolean(),
        ConfigShapeValueType::Json => !value.is_null(),
        ConfigShapeValueType::JsonObject => value.is_object(),
    };
    if !valid {
        return Err(invalid(
            source,
            format!("{package_id} config path {path} must be {value_type}"),
        ));
    }
    Ok(())
}

fn ensure_unique<'a, T: Ord + ?Sized + 'a>(
    values: impl Iterator<Item = &'a T>,
    path: impl Into<PathBuf>,
    label: &str,
) -> ConfigSnapshotToolingResult<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(invalid(path, format!("{label} appears more than once")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use serde_json::{json, Value};
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId, PackageConfigAccess,
        PackageConfigRequirement, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
        ServiceDeploymentRef,
    };

    use super::{
        project_runtime_config_snapshot, ConfigSnapshotDeploymentInput, ConfigSnapshotPackageInput,
    };

    #[test]
    fn projection_validates_required_optional_types_unknown_packages_and_nested_conflicts() {
        let valid = input(
            "service-a",
            json!({
                "required": "yes",
                "nested": {"count": 2}
            }),
        );
        let snapshot = project_runtime_config_snapshot(snapshot_ref(), vec![valid]).unwrap();
        assert_eq!(snapshot.package_count(), 1);

        let missing = input("service-a", json!({"nested": {"count": 2}}));
        assert!(
            project_runtime_config_snapshot(snapshot_ref(), vec![missing])
                .unwrap_err()
                .to_string()
                .contains("missing required")
        );

        let mismatch = input("service-a", json!({"required": 1, "nested": {"count": 2}}));
        assert!(
            project_runtime_config_snapshot(snapshot_ref(), vec![mismatch])
                .unwrap_err()
                .to_string()
                .contains("must be string")
        );

        let conflict = input(
            "service-a",
            json!({"required": "yes", "nested": "not-an-object"}),
        );
        assert!(
            project_runtime_config_snapshot(snapshot_ref(), vec![conflict])
                .unwrap_err()
                .to_string()
                .contains("nested below a non-object")
        );

        let mut unknown = input("service-a", json!({"required": "yes"}));
        unknown
            .config
            .insert("packages".to_string(), BTreeMap::new());
        assert!(
            project_runtime_config_snapshot(snapshot_ref(), vec![unknown])
                .unwrap_err()
                .to_string()
                .contains("outside the exact deployment closure")
        );
    }

    #[test]
    fn same_build_is_isolated_across_deployments_and_duplicate_builds_are_rejected_within_one() {
        let left = input("service-a", json!({"required": "left"}));
        let right = input("service-b", json!({"required": "right"}));
        let snapshot = project_runtime_config_snapshot(snapshot_ref(), vec![right, left]).unwrap();
        let values = snapshot
            .deployments()
            .iter()
            .map(|deployment| {
                deployment.packages()[0].config()["required"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["left", "right"]);

        let mut duplicate = input("service-a", json!({"required": "left"}));
        duplicate.packages.push(duplicate.packages[0].clone());
        assert!(
            project_runtime_config_snapshot(snapshot_ref(), vec![duplicate])
                .unwrap_err()
                .to_string()
                .contains("Package build appears more than once")
        );
    }

    fn input(service_id: &str, config: Value) -> ConfigSnapshotDeploymentInput {
        let Value::Object(config) = config else {
            panic!("config fixture must be object");
        };
        ConfigSnapshotDeploymentInput {
            deployment: deployment(service_id),
            source_path: PathBuf::from(format!("/{service_id}")),
            config: BTreeMap::from([(
                "example.com/shared".to_string(),
                config.into_iter().collect(),
            )]),
            packages: vec![ConfigSnapshotPackageInput {
                package_id: "example.com/shared".to_string(),
                package_build_id: PackageBuildId::new("build-shared"),
                requirements: vec![
                    PackageConfigRequirement {
                        path: "required".to_string(),
                        access: PackageConfigAccess::Required {
                            value_type: "string".to_string(),
                        },
                    },
                    PackageConfigRequirement {
                        path: "nested.count".to_string(),
                        access: PackageConfigAccess::Optional {
                            value_type: "number".to_string(),
                        },
                    },
                ],
            }],
        }
    }

    fn deployment(service_id: &str) -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: service_id.to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("dev"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "skiff-service-deployment-v2:sha256:{}",
                if service_id == "service-a" {
                    "a".repeat(64)
                } else {
                    "b".repeat(64)
                }
            )),
        }
    }

    fn snapshot_ref() -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef",
            )
            .unwrap(),
        }
    }
}
