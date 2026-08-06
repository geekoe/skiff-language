use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{PackageBuildId, PackageRequirementKey, ServiceDeploymentRef};
use skiff_runtime_linker::AssemblyLinkedCandidate;

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

/// Materializes empty config views for every deployment of a candidate.
///
/// The runtime no longer receives a router-supplied config snapshot; config
/// baked into deployment records is a later milestone, so deployments
/// materialize empty views until that lands.
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
