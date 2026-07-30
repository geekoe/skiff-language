use std::{collections::BTreeMap, fmt, path::Path};

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_artifact_model::{
    validate_activation_environment, validate_runtime_config_snapshot_ref, PackageBuildId,
    RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};

use crate::{error::invalid, strict_json::StrictJsonValue, RuntimeConfigSnapshotResult};

pub const RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION: &str =
    "skiff-runtime-config-snapshot-record-v2";
pub const MAX_CONFIG_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_DEPLOYMENTS_PER_SNAPSHOT: usize = 1_024;
pub const MAX_PACKAGES_PER_DEPLOYMENT: usize = 4_096;
pub const MAX_CONFIG_DEPTH: usize = 32;
pub const MAX_CONFIG_NODES: usize = 100_000;
const MAX_IDENTITY_BYTES: usize = 1_024;
const MAX_CONFIG_KEY_BYTES: usize = 512;
const MAX_CONFIG_STRING_BYTES: usize = 4 * 1024 * 1024;
const MAX_CONTAINER_ENTRIES: usize = 16_384;

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigSnapshot {
    schema_version: String,
    environment: String,
    snapshot: RuntimeConfigSnapshotRef,
    deployments: Vec<RuntimeConfigDeployment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeConfigSnapshot {
    schema_version: String,
    environment: String,
    snapshot: RuntimeConfigSnapshotRef,
    deployments: Vec<RuntimeConfigDeployment>,
}

impl RuntimeConfigSnapshot {
    pub fn new(
        environment: impl Into<String>,
        snapshot: RuntimeConfigSnapshotRef,
        deployments: Vec<RuntimeConfigDeployment>,
    ) -> RuntimeConfigSnapshotResult<Self> {
        let record = Self {
            schema_version: RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION.to_string(),
            environment: environment.into(),
            snapshot,
            deployments,
        };
        record.validate(Path::new("<memory>"))?;
        Ok(record)
    }

    pub fn snapshot_ref(&self) -> &RuntimeConfigSnapshotRef {
        &self.snapshot
    }

    pub fn environment(&self) -> &str {
        &self.environment
    }

    pub fn deployments(&self) -> &[RuntimeConfigDeployment] {
        &self.deployments
    }

    pub fn package_count(&self) -> usize {
        self.deployments
            .iter()
            .map(|deployment| deployment.packages.len())
            .sum()
    }

    pub(crate) fn validate(&self, path: &Path) -> RuntimeConfigSnapshotResult<()> {
        if self.schema_version != RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION {
            return Err(invalid(path, "snapshot record schemaVersion mismatch"));
        }
        validate_activation_environment(&self.environment)
            .map_err(|message| invalid(path, message))?;
        validate_runtime_config_snapshot_ref(&self.snapshot)
            .map_err(|message| invalid(path, message))?;
        if self.deployments.len() > MAX_DEPLOYMENTS_PER_SNAPSHOT {
            return Err(invalid(
                path,
                format!("deployments exceeds limit {MAX_DEPLOYMENTS_PER_SNAPSHOT}"),
            ));
        }
        ensure_strictly_sorted_unique(
            self.deployments
                .iter()
                .map(RuntimeConfigDeployment::deployment),
            path,
            "deployments",
        )?;
        let mut budget = ConfigBudget::default();
        for deployment in &self.deployments {
            deployment.validate(path, &mut budget)?;
        }
        let encoded_len = serde_json::to_vec(self)
            .map_err(|error| invalid(path, format!("snapshot serialization failed: {error}")))?
            .len() as u64;
        if encoded_len > MAX_CONFIG_SNAPSHOT_BYTES {
            return Err(invalid(
                path,
                format!("encoded snapshot exceeds {MAX_CONFIG_SNAPSHOT_BYTES} byte limit"),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for RuntimeConfigSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfigSnapshot")
            .field("snapshot_id", &self.snapshot.snapshot_id)
            .field("deployment_count", &self.deployments.len())
            .field("package_count", &self.package_count())
            .finish()
    }
}

impl<'de> Deserialize<'de> for RuntimeConfigSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeConfigSnapshot::deserialize(deserializer)?;
        let record = Self {
            schema_version: raw.schema_version,
            environment: raw.environment,
            snapshot: raw.snapshot,
            deployments: raw.deployments,
        };
        record
            .validate(Path::new("<serde>"))
            .map_err(de::Error::custom)?;
        Ok(record)
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfigDeployment {
    deployment: ServiceDeploymentRef,
    packages: Vec<RuntimeConfigPackage>,
}

impl RuntimeConfigDeployment {
    pub fn new(
        deployment: ServiceDeploymentRef,
        packages: Vec<RuntimeConfigPackage>,
    ) -> RuntimeConfigSnapshotResult<Self> {
        let value = Self {
            deployment,
            packages,
        };
        let mut budget = ConfigBudget::default();
        value.validate(Path::new("<memory>"), &mut budget)?;
        Ok(value)
    }

    pub fn deployment(&self) -> &ServiceDeploymentRef {
        &self.deployment
    }

    pub fn packages(&self) -> &[RuntimeConfigPackage] {
        &self.packages
    }

    fn validate(&self, path: &Path, budget: &mut ConfigBudget) -> RuntimeConfigSnapshotResult<()> {
        validate_deployment_ref(&self.deployment, path)?;
        if self.packages.len() > MAX_PACKAGES_PER_DEPLOYMENT {
            return Err(invalid(
                path,
                format!("packages exceeds per-deployment limit {MAX_PACKAGES_PER_DEPLOYMENT}"),
            ));
        }
        ensure_strictly_sorted_unique(
            self.packages
                .iter()
                .map(RuntimeConfigPackage::package_build_id),
            path,
            "packages",
        )?;
        for package in &self.packages {
            package.validate(path, budget)?;
        }
        Ok(())
    }
}

impl fmt::Debug for RuntimeConfigDeployment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfigDeployment")
            .field("deployment", &self.deployment)
            .field("package_count", &self.packages.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfigPackage {
    package_build_id: PackageBuildId,
    config: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeConfigPackage {
    package_build_id: PackageBuildId,
    config: StrictJsonValue,
}

impl RuntimeConfigPackage {
    pub fn new(
        package_build_id: PackageBuildId,
        config: BTreeMap<String, Value>,
    ) -> RuntimeConfigSnapshotResult<Self> {
        let value = Self {
            package_build_id,
            config,
        };
        let mut budget = ConfigBudget::default();
        value.validate(Path::new("<memory>"), &mut budget)?;
        Ok(value)
    }

    pub fn package_build_id(&self) -> &PackageBuildId {
        &self.package_build_id
    }

    pub fn config(&self) -> &BTreeMap<String, Value> {
        &self.config
    }

    fn validate(&self, path: &Path, budget: &mut ConfigBudget) -> RuntimeConfigSnapshotResult<()> {
        validate_identity(self.package_build_id.as_str(), "packageBuildId", path)?;
        validate_object(&self.config, 1, budget, path)
    }
}

impl fmt::Debug for RuntimeConfigPackage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfigPackage")
            .field("package_build_id", &self.package_build_id)
            .field("top_level_key_count", &self.config.len())
            .finish()
    }
}

impl<'de> Deserialize<'de> for RuntimeConfigPackage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeConfigPackage::deserialize(deserializer)?;
        let Value::Object(config) = raw.config.into_inner() else {
            return Err(de::Error::custom(
                "package config must be a package-local JSON object",
            ));
        };
        let package = Self {
            package_build_id: raw.package_build_id,
            config: config.into_iter().collect(),
        };
        let mut budget = ConfigBudget::default();
        package
            .validate(Path::new("<serde>"), &mut budget)
            .map_err(de::Error::custom)?;
        Ok(package)
    }
}

#[derive(Default)]
struct ConfigBudget {
    nodes: usize,
}

fn validate_deployment_ref(
    reference: &ServiceDeploymentRef,
    path: &Path,
) -> RuntimeConfigSnapshotResult<()> {
    for (label, value) in [
        ("serviceId", reference.service_id.as_str()),
        ("contractVersion", reference.contract_version.as_str()),
        ("deploymentRevision", reference.deployment_revision.as_str()),
        (
            "deploymentArtifactIdentity",
            reference.deployment_artifact_identity.as_str(),
        ),
    ] {
        validate_identity(value, label, path)?;
    }
    Ok(())
}

fn validate_identity(value: &str, label: &str, path: &Path) -> RuntimeConfigSnapshotResult<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTITY_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(invalid(
            path,
            format!("{label} must be a non-empty bounded visible token"),
        ));
    }
    Ok(())
}

fn validate_object(
    object: &BTreeMap<String, Value>,
    depth: usize,
    budget: &mut ConfigBudget,
    path: &Path,
) -> RuntimeConfigSnapshotResult<()> {
    if depth > MAX_CONFIG_DEPTH {
        return Err(invalid(
            path,
            format!("config depth exceeds limit {MAX_CONFIG_DEPTH}"),
        ));
    }
    if object.len() > MAX_CONTAINER_ENTRIES {
        return Err(invalid(path, "config object has too many entries"));
    }
    charge_node(budget, path)?;
    for (key, value) in object {
        if key.is_empty() || key.len() > MAX_CONFIG_KEY_BYTES || key.chars().any(char::is_control) {
            return Err(invalid(path, "config object key is invalid or too large"));
        }
        validate_value(value, depth + 1, budget, path)?;
    }
    Ok(())
}

fn validate_value(
    value: &Value,
    depth: usize,
    budget: &mut ConfigBudget,
    path: &Path,
) -> RuntimeConfigSnapshotResult<()> {
    if depth > MAX_CONFIG_DEPTH {
        return Err(invalid(
            path,
            format!("config depth exceeds limit {MAX_CONFIG_DEPTH}"),
        ));
    }
    charge_node(budget, path)?;
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        Value::String(value) => {
            if value.len() > MAX_CONFIG_STRING_BYTES {
                Err(invalid(path, "config string exceeds size limit"))
            } else {
                Ok(())
            }
        }
        Value::Array(values) => {
            if values.len() > MAX_CONTAINER_ENTRIES {
                return Err(invalid(path, "config array has too many entries"));
            }
            for value in values {
                validate_value(value, depth + 1, budget, path)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            let sorted = values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            validate_object(&sorted, depth, budget, path)
        }
    }
}

fn charge_node(budget: &mut ConfigBudget, path: &Path) -> RuntimeConfigSnapshotResult<()> {
    budget.nodes = budget.nodes.saturating_add(1);
    if budget.nodes > MAX_CONFIG_NODES {
        Err(invalid(
            path,
            format!("config node count exceeds limit {MAX_CONFIG_NODES}"),
        ))
    } else {
        Ok(())
    }
}

fn ensure_strictly_sorted_unique<'a, T: Ord + 'a>(
    values: impl IntoIterator<Item = &'a T>,
    path: &Path,
    label: &str,
) -> RuntimeConfigSnapshotResult<()> {
    let mut previous: Option<&T> = None;
    for value in values {
        if previous.is_some_and(|previous| previous >= value) {
            return Err(invalid(
                path,
                format!("{label} must be strictly sorted and unique"),
            ));
        }
        previous = Some(value);
    }
    Ok(())
}
