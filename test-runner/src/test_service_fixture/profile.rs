use std::collections::BTreeMap;

use serde::Deserialize;
use skiff_artifact_model::{
    ActivationPolicy, ConfigLiteralBinding, DeploymentPolicy, MetadataValue, PackageArtifact,
    ResourceBinding, ResourcePolicy, SecretRefBinding, ServiceDeployment, StateBindingKind,
};

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::{CanonicalPackageProject, CanonicalTestServiceProfile},
};

pub(super) const TEST_INGRESS_CONFIG_PATH: &str = "skiff.test.ingressUrl";

#[derive(Debug, Clone)]
pub(super) struct SelectedProfileBindings {
    pub(super) config_literals: Vec<ConfigLiteralBinding>,
    pub(super) secret_refs: Vec<SecretRefBinding>,
    pub(super) resource_bindings: Vec<ResourceBinding>,
    pub(super) policy: DeploymentPolicy,
}

/// Project ordinary profile bindings and add only the runner-owned ingress
/// literal that the compiled test service actually requires.
pub(super) fn selected_profile_bindings(
    project: &CanonicalPackageProject,
    implementation: &PackageArtifact,
    state_requirements: &BTreeMap<String, StateBindingKind>,
    owner: Option<&ServiceDeployment>,
    ingress_url: Option<&str>,
) -> Result<SelectedProfileBindings, CanonicalFixtureError> {
    let Some(test_service) = &project.test_service_profile else {
        return Ok(owner
            .map(|deployment| SelectedProfileBindings {
                config_literals: deployment.config_literals.clone(),
                secret_refs: deployment.secret_refs.clone(),
                resource_bindings: deployment.resource_bindings.clone(),
                policy: deployment.policy.clone(),
            })
            .unwrap_or_else(default_test_service_policy));
    };
    let config = profile_map::<serde_json::Value>(test_service, "config")?;
    let secrets = profile_map::<String>(test_service, "secrets")?;
    let resources = profile_map::<TestResourceAuthoring>(test_service, "resources")?;
    let states = profile_map::<TestStateAuthoring>(test_service, "state")?;
    reject_reserved_test_ingress_binding(test_service, &config, &secrets)?;
    validate_test_service_states(test_service, state_requirements, &states)?;
    let policy = test_service_policy(test_service)?;
    if has_http_entries(test_service) && policy.activation.max_concurrency < 2 {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "test service {} lifecycle.maxConcurrency must be at least 2 when http.yml declares an entry",
            test_service.service_id
        )));
    }
    let runner_ingress = implementation
        .runtime_requirements
        .config
        .iter()
        .any(|requirement| requirement.path == TEST_INGRESS_CONFIG_PATH)
        .then(|| {
            ingress_url.map(str::to_string).ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "test service {} requires the runner-owned {TEST_INGRESS_CONFIG_PATH} binding",
                    test_service.service_id
                ))
            })
        })
        .transpose()?;
    let mut config_literals = config
        .into_iter()
        .map(|(path, value)| ConfigLiteralBinding {
            path,
            value: MetadataValue::from_json(value),
        })
        .collect::<Vec<_>>();
    if let Some(value) = &runner_ingress {
        config_literals.push(ConfigLiteralBinding {
            path: TEST_INGRESS_CONFIG_PATH.to_string(),
            value: MetadataValue::String(value.clone()),
        });
    }
    Ok(SelectedProfileBindings {
        config_literals,
        secret_refs: secrets
            .into_iter()
            .map(|(path, secret_ref)| SecretRefBinding { path, secret_ref })
            .collect(),
        resource_bindings: resources
            .into_iter()
            .map(|(requirement_key, binding)| ResourceBinding {
                requirement_key,
                capability: binding.capability,
                resource_ref: binding.resource_ref,
            })
            .collect(),
        policy,
    })
}

fn has_http_entries(test_service: &CanonicalTestServiceProfile) -> bool {
    test_service
        .http
        .as_ref()
        .is_some_and(|document| !document.entries.is_empty())
}

fn reject_reserved_test_ingress_binding(
    test_service: &CanonicalTestServiceProfile,
    config: &BTreeMap<String, serde_json::Value>,
    secrets: &BTreeMap<String, String>,
) -> Result<(), CanonicalFixtureError> {
    if config.contains_key(TEST_INGRESS_CONFIG_PATH)
        || secrets.contains_key(TEST_INGRESS_CONFIG_PATH)
    {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml path {TEST_INGRESS_CONFIG_PATH} is reserved for the test runner",
            test_service.service_id, test_service.profile_name
        )));
    }
    Ok(())
}

fn default_test_service_policy() -> SelectedProfileBindings {
    SelectedProfileBindings {
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        resource_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: Some(30_000),
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            activation: ActivationPolicy {
                max_concurrency: 1,
                idle_timeout_ms: None,
            },
            principal: "test:package-runner".to_string(),
        },
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestStateAuthoring {
    kind: StateBindingKind,
    namespace: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestResourceAuthoring {
    capability: String,
    resource_ref: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestQuotaAuthoring {
    cpu_millis: u32,
    memory_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestLifecycleAuthoring {
    max_concurrency: u32,
    #[serde(default)]
    idle_timeout_ms: Option<u64>,
}

fn profile_map<T: for<'de> Deserialize<'de>>(
    test_service: &CanonicalTestServiceProfile,
    field: &'static str,
) -> Result<BTreeMap<String, T>, CanonicalFixtureError> {
    let value = match field {
        "config" => &test_service.authoring.config,
        "secrets" => &test_service.authoring.secrets,
        "state" => &test_service.authoring.state,
        "resources" => &test_service.authoring.resources,
        _ => unreachable!("profile map field is compiler-owned"),
    };
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_value(value.clone()).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml field {field} must be a path-keyed object: {error}",
            test_service.service_id, test_service.profile_name
        ))
    })
}

fn validate_test_service_states(
    test_service: &CanonicalTestServiceProfile,
    expected: &BTreeMap<String, StateBindingKind>,
    authored: &BTreeMap<String, TestStateAuthoring>,
) -> Result<(), CanonicalFixtureError> {
    for (key, kind) in expected {
        let binding = authored.get(key).ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "test service {} config.{}.yml is missing state binding {key}",
                test_service.service_id, test_service.profile_name
            ))
        })?;
        if &binding.kind != kind {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test service {} config.{}.yml state binding {key} must be {kind:?}, got {:?}",
                test_service.service_id, test_service.profile_name, binding.kind
            )));
        }
        if binding.namespace.trim().is_empty() {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test service {} config.{}.yml state binding {key} namespace must not be empty",
                test_service.service_id, test_service.profile_name
            )));
        }
    }
    if let Some(key) = authored.keys().find(|key| !expected.contains_key(*key)) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml has extra state binding {key}",
            test_service.service_id, test_service.profile_name
        )));
    }
    Ok(())
}

fn test_service_policy(
    test_service: &CanonicalTestServiceProfile,
) -> Result<DeploymentPolicy, CanonicalFixtureError> {
    let quota = profile_value::<TestQuotaAuthoring>(test_service, "quota")?;
    let lifecycle = profile_value::<TestLifecycleAuthoring>(test_service, "lifecycle")?;
    let principal = profile_value::<String>(test_service, "principal")?;
    let timeout_ms = if test_service.authoring.timeout.is_null() {
        None
    } else {
        Some(profile_value::<u64>(test_service, "timeout")?)
    };
    Ok(DeploymentPolicy {
        timeout_ms,
        resources: ResourcePolicy {
            cpu_millis: quota.cpu_millis,
            memory_bytes: quota.memory_bytes,
        },
        activation: ActivationPolicy {
            max_concurrency: lifecycle.max_concurrency,
            idle_timeout_ms: lifecycle.idle_timeout_ms,
        },
        principal,
    })
}

fn profile_value<T: for<'de> Deserialize<'de>>(
    test_service: &CanonicalTestServiceProfile,
    field: &'static str,
) -> Result<T, CanonicalFixtureError> {
    let value = match field {
        "timeout" => &test_service.authoring.timeout,
        "quota" => &test_service.authoring.quota,
        "principal" => &test_service.authoring.principal,
        "lifecycle" => &test_service.authoring.lifecycle,
        _ => unreachable!("profile scalar field is compiler-owned"),
    };
    serde_json::from_value(value.clone()).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml field {field} is invalid: {error}",
            test_service.service_id, test_service.profile_name
        ))
    })
}
