use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical_fixture::CanonicalFixtureError;

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
    let mut paths = Vec::<PathBuf>::new();
    if let Some(parent) = package_root.parent() {
        paths.push(parent.join(MANIFEST_NAME));
    }
    paths.push(package_root.join(MANIFEST_NAME));

    let mut tests = HashMap::new();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "failed to read {}: {error}",
                path.display()
            ))
        })?;
        let manifest: TestDoublesManifest = serde_json::from_str(&text).map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!("invalid {}: {error}", path.display()))
        })?;
        let _ = (manifest.config, manifest.configs);
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
