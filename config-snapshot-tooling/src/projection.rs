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
    environment: &str,
    snapshot: RuntimeConfigSnapshotRef,
    inputs: Vec<ConfigSnapshotDeploymentInput>,
) -> ConfigSnapshotToolingResult<RuntimeConfigSnapshot> {
    project_runtime_config_snapshot_with_base(environment, snapshot, None, inputs)
}

pub fn project_runtime_config_snapshot_with_base(
    environment: &str,
    snapshot: RuntimeConfigSnapshotRef,
    base: Option<&RuntimeConfigSnapshot>,
    mut inputs: Vec<ConfigSnapshotDeploymentInput>,
) -> ConfigSnapshotToolingResult<RuntimeConfigSnapshot> {
    if let Some(base) = base {
        if base.environment() != environment {
            return Err(invalid(
                "<projection>",
                format!(
                    "base config snapshot environment {:?} does not match target environment {:?}",
                    base.environment(),
                    environment
                ),
            ));
        }
    }
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
    deployments.extend(
        base.into_iter()
            .flat_map(RuntimeConfigSnapshot::deployments)
            .cloned(),
    );
    deployments.sort_by(|left, right| left.deployment().cmp(right.deployment()));
    ensure_unique(
        deployments.iter().map(RuntimeConfigDeployment::deployment),
        "<projection>",
        "deployment",
    )?;
    RuntimeConfigSnapshot::new(environment, snapshot, deployments).map_err(Into::into)
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
mod tests;
