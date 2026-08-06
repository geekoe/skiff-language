use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::UNIX_EPOCH;

use skiff_artifact_model::{PackageBuildId, PackageRequirementKey, ServiceDeploymentRef};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
use skiff_runtime_linker::AssemblyLinkedCandidate;
use tracing::{info, warn};

use crate::config_view::RuntimeConfigView;

/// Deployment-owned immutable config projection. Package slots stay aligned
/// with the shared execution image, but every deployment owns a distinct set
/// of views even when two deployments use the same exact Package build.
#[derive(Clone, Debug)]
pub(crate) struct ActivationConfigViews {
    service: RuntimeConfigView,
    packages: Vec<RuntimeConfigView>,
}

impl ActivationConfigViews {
    pub(crate) fn service(&self) -> &RuntimeConfigView {
        &self.service
    }

    pub(crate) fn packages(&self) -> &[RuntimeConfigView] {
        &self.packages
    }
}

/// Materializes config views for every deployment of a candidate from the
/// published runtime-config snapshot records (M5: lazy-load config source).
///
/// The deployment's config is looked up by exact deployment identity in the
/// newest snapshot record of the artifact root's `runtime-config` store
/// (snapshots are whole-profile merges, so the newest record covers every
/// published deployment). Missing records degrade to empty views (pre-M5
/// artifacts) with a warning; the shape still comes from the package
/// requirements.
pub(crate) fn materialize_config(
    candidate: &AssemblyLinkedCandidate,
    artifact_root: &Path,
) -> anyhow::Result<BTreeMap<ServiceDeploymentRef, ActivationConfigViews>> {
    let snapshot = match latest_snapshot(artifact_root) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            warn!(
                event = "runtime.config_snapshot_missing",
                artifact_root = %artifact_root.display()
            );
            return materialize_empty_config(candidate);
        }
        Err(error) => {
            warn!(
                event = "runtime.config_snapshot_unreadable",
                artifact_root = %artifact_root.display(),
                error = %error
            );
            return materialize_empty_config(candidate);
        }
    };
    let config_by_deployment = snapshot
        .deployments()
        .iter()
        .map(|deployment| (deployment.deployment(), deployment.packages()))
        .collect::<BTreeMap<_, _>>();
    let image = candidate.execution_image();
    let mut materialized = BTreeMap::new();
    for (deployment, activation) in candidate.activations() {
        let closure = activation_package_closure(candidate, deployment)?;
        let mut package_views = Vec::with_capacity(image.execution_packages().len());
        for package in image.execution_packages() {
            if closure.contains(package.package_build_id()) {
                let shape = skiff_artifact_model::config_shape_from_package_requirements(
                    &package.artifact().runtime_requirements.config,
                )?;
                let config = config_by_deployment
                    .get(deployment)
                    .and_then(|packages| {
                        packages
                            .iter()
                            .find(|entry| entry.package_build_id() == package.package_build_id())
                    })
                    .map(|entry| entry.config())
                    .cloned()
                    .unwrap_or_default();
                let view = if config.is_empty() {
                    RuntimeConfigView::empty_unvalidated_with_shape(shape)
                } else {
                    let value = serde_json::Value::Object(
                        config.into_iter().collect::<serde_json::Map<_, _>>(),
                    );
                    RuntimeConfigView::from_resolved_config(value, shape)?
                };
                package_views.push(view);
            } else {
                package_views.push(RuntimeConfigView::empty());
            }
        }
        let implementation_slot = image
            .code_by_build(activation.implementation_package_build_id())
            .ok_or_else(|| anyhow::anyhow!("activation implementation Package is missing"))?
            .code_slot()
            .index();
        let service = package_views
            .get(implementation_slot)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("implementation Package config slot is missing"))?;
        materialized.insert(
            deployment.clone(),
            ActivationConfigViews {
                service,
                packages: package_views,
            },
        );
    }
    Ok(materialized)
}

/// Newest published snapshot record of the artifact root (by file mtime).
fn latest_snapshot(artifact_root: &Path) -> anyhow::Result<Option<RuntimeConfigSnapshot>> {
    let snapshots_dir = artifact_root.join("runtime-config").join("snapshots");
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&snapshots_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        entries.push((modified, path));
    }
    entries.sort_by_key(|(modified, _)| *modified);
    let Some((_, latest)) = entries.last() else {
        return Ok(None);
    };
    let bytes = std::fs::read(latest)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let snapshot: RuntimeConfigSnapshot = serde_json::from_value(value)?;
    Ok(Some(snapshot))
}

/// Materializes empty config views for every deployment of a candidate.
///
/// Fallback when no snapshot record is available (pre-M5 artifacts).
pub(crate) fn materialize_empty_config(
    candidate: &AssemblyLinkedCandidate,
) -> anyhow::Result<BTreeMap<ServiceDeploymentRef, ActivationConfigViews>> {
    let image = candidate.execution_image();
    let mut materialized = BTreeMap::new();
    for (deployment, activation) in candidate.activations() {
        let closure = activation_package_closure(candidate, deployment)?;
        let mut package_views = Vec::with_capacity(image.execution_packages().len());
        for package in image.execution_packages() {
            if closure.contains(package.package_build_id()) {
                let shape = skiff_artifact_model::config_shape_from_package_requirements(
                    &package.artifact().runtime_requirements.config,
                )?;
                package_views.push(RuntimeConfigView::empty_unvalidated_with_shape(shape));
            } else {
                package_views.push(RuntimeConfigView::empty());
            }
        }
        let implementation_slot = image
            .code_by_build(activation.implementation_package_build_id())
            .ok_or_else(|| anyhow::anyhow!("activation implementation Package is missing"))?
            .code_slot()
            .index();
        let service = package_views
            .get(implementation_slot)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("implementation Package config slot is missing"))?;
        materialized.insert(
            deployment.clone(),
            ActivationConfigViews {
                service,
                packages: package_views,
            },
        );
    }
    Ok(materialized)
}

fn activation_package_closure(
    candidate: &AssemblyLinkedCandidate,
    deployment: &ServiceDeploymentRef,
) -> anyhow::Result<BTreeSet<PackageBuildId>> {
    let activation = candidate
        .activation(deployment)
        .ok_or_else(|| anyhow::anyhow!("config projection targets an unknown activation"))?;
    let bindings = activation
        .deployment()
        .package_bindings
        .iter()
        .map(|binding| (binding.key.clone(), &binding.package.package_build_id))
        .collect::<BTreeMap<_, _>>();
    let image = candidate.execution_image();
    let mut closure = BTreeSet::new();
    let mut pending = vec![activation.implementation_package_build_id().clone()];
    while let Some(build_id) = pending.pop() {
        if !closure.insert(build_id.clone()) {
            continue;
        }
        let package = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation Package closure targets missing build {build_id}")
        })?;
        for requirement in &package.artifact().package_requirements {
            let key = PackageRequirementKey {
                caller_package_build_id: build_id.clone(),
                package_requirement_alias: requirement.alias.clone(),
            };
            let provider = bindings.get(&key).ok_or_else(|| {
                anyhow::anyhow!("activation Package requirement {key:?} has no exact binding")
            })?;
            pending.push((*provider).clone());
        }
    }
    Ok(closure)
}
