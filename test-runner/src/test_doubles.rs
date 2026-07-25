use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_model::MetadataValue;

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::CanonicalPackageProject,
    package_test_assembly::PackageTestConfigLiteral,
};

const MANIFEST_NAME: &str = "skiff.test-doubles.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TestEffectDouble {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expect_request: Option<Value>,
    pub(crate) response: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestDoublesManifest {
    #[serde(default)]
    config: Value,
    #[serde(default)]
    configs: HashMap<String, Value>,
    #[serde(default)]
    tests: HashMap<String, HashMap<String, TestDoubleDefinition>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TestDoubleDefinition {
    Single(TestEffectDouble),
    Sequence { sequence: Vec<TestEffectDouble> },
}

pub(crate) type TestEffectDoubles = HashMap<String, HashMap<String, Vec<TestEffectDouble>>>;

pub(crate) fn load(package_root: &Path) -> Result<TestEffectDoubles, CanonicalFixtureError> {
    let manifests = read_manifests(package_root)?;
    let mut tests = HashMap::new();
    for (_, manifest) in manifests {
        for (test, targets) in manifest.tests {
            if tests.contains_key(&test) {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "duplicate test double key {test}"
                )));
            }
            let mut normalized = HashMap::new();
            for (target, definition) in targets {
                let sequence = match definition {
                    TestDoubleDefinition::Single(double) => vec![double],
                    TestDoubleDefinition::Sequence { sequence } if !sequence.is_empty() => sequence,
                    TestDoubleDefinition::Sequence { .. } => {
                        return Err(CanonicalFixtureError::InvalidInput(format!(
                            "test double {test}.{target} has an empty sequence"
                        )));
                    }
                };
                normalized.insert(target, sequence);
            }
            tests.insert(test, normalized);
        }
    }
    Ok(tests)
}

pub(crate) fn load_config_literals(
    package_root: &Path,
    project: &CanonicalPackageProject,
) -> Result<Vec<PackageTestConfigLiteral>, CanonicalFixtureError> {
    let manifests = read_manifests(package_root)?;
    let mut supplied = HashMap::<(String, String), PackageTestConfigLiteral>::new();
    for (path, manifest) in manifests {
        if !manifest.config.is_null() {
            let config = manifest.config.as_object().ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "{} config must be a JSON object",
                    path.display()
                ))
            })?;
            for (key, value) in config {
                let matches = project
                    .artifacts()
                    .filter(|package| {
                        package
                            .runtime_requirements
                            .config
                            .iter()
                            .any(|requirement| requirement.path == *key)
                    })
                    .collect::<Vec<_>>();
                if matches.len() > 1 {
                    return Err(CanonicalFixtureError::InvalidInput(format!(
                        "{} config key {key} matches multiple packages; use configs.<package-id>.{key}",
                        path.display()
                    )));
                }
                if let Some(package) = matches.first() {
                    insert_config_literal(&mut supplied, package, key, value, &path)?;
                }
            }
        }
        for (package_id, value) in manifest.configs {
            let Some(package) = project.artifacts().find(|package| package.package_id == package_id)
            else {
                continue;
            };
            let config = value.as_object().ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "{} configs.{package_id} must be a JSON object",
                    path.display()
                ))
            })?;
            for (key, value) in config {
                if !package
                    .runtime_requirements
                    .config
                    .iter()
                    .any(|requirement| requirement.path == *key)
                {
                    return Err(CanonicalFixtureError::InvalidInput(format!(
                        "{} configs.{package_id} names unknown requirement {key}",
                        path.display()
                    )));
                }
                insert_config_literal(&mut supplied, package, key, value, &path)?;
            }
        }
    }
    Ok(supplied.into_values().collect())
}

fn insert_config_literal(
    supplied: &mut HashMap<(String, String), PackageTestConfigLiteral>,
    package: &skiff_artifact_model::PackageArtifact,
    key: &str,
    value: &Value,
    path: &Path,
) -> Result<(), CanonicalFixtureError> {
    let package = package_artifact_ref(package)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let literal = PackageTestConfigLiteral {
        package: package.clone(),
        key: key.to_string(),
        value: serde_json::from_value::<MetadataValue>(value.clone()).map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "{} config value {key} is invalid: {error}",
                path.display()
            ))
        })?,
    };
    if supplied
        .insert((package.package_build_id.to_string(), key.to_string()), literal)
        .is_some()
    {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "duplicate test config literal for package {} key {key}",
            package.package_build_id
        )));
    }
    Ok(())
}

fn read_manifests(
    package_root: &Path,
) -> Result<Vec<(PathBuf, TestDoublesManifest)>, CanonicalFixtureError> {
    let mut paths = Vec::<PathBuf>::new();
    if let Some(parent) = package_root.parent() {
        paths.push(parent.join(MANIFEST_NAME));
    }
    paths.push(package_root.join(MANIFEST_NAME));
    paths
        .into_iter()
        .filter(|path| path.is_file())
        .map(|path| {
            let text = fs::read_to_string(&path).map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!(
                    "failed to read {}: {error}",
                    path.display()
                ))
            })?;
            let manifest = serde_json::from_str(&text).map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!("invalid {}: {error}", path.display()))
            })?;
            Ok((path, manifest))
        })
        .collect()
}
