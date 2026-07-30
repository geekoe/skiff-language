use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use skiff_artifact_model::{PackageBuildId, PackageRequirementKey, ServiceDeploymentRef};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
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

pub(crate) fn materialize_snapshot_config(
    candidate: &AssemblyLinkedCandidate,
    snapshot: &RuntimeConfigSnapshot,
) -> anyhow::Result<BTreeMap<ServiceDeploymentRef, ActivationConfigViews>> {
    let snapshot_deployments = snapshot
        .deployments()
        .iter()
        .map(|deployment| (deployment.deployment().clone(), deployment))
        .collect::<BTreeMap<_, _>>();
    let candidate_deployments = candidate
        .activations()
        .map(|(deployment, _)| deployment.clone())
        .collect::<BTreeSet<_>>();
    if snapshot_deployments
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != candidate_deployments
    {
        anyhow::bail!(
            "RuntimeConfigSnapshot deployments do not exactly match RuntimeAssembly activations"
        );
    }

    let image = candidate.execution_image();
    let mut materialized = BTreeMap::new();
    for (deployment, activation) in candidate.activations() {
        let partition = snapshot_deployments
            .get(deployment)
            .expect("exact deployment set was checked above");
        let closure = activation_package_closure(candidate, deployment)?;
        let package_values = partition
            .packages()
            .iter()
            .map(|package| (package.package_build_id().clone(), package))
            .collect::<BTreeMap<_, _>>();
        if package_values.keys().cloned().collect::<BTreeSet<_>>() != closure {
            anyhow::bail!(
                "RuntimeConfigSnapshot deployment {:?} packages do not exactly match its Package closure",
                deployment
            );
        }

        let mut package_views = Vec::with_capacity(image.execution_packages().len());
        for (slot, package) in image.execution_packages().iter().enumerate() {
            if package.code_slot().index() != slot {
                anyhow::bail!(
                    "active execution image package slot mismatch: expected {slot}, got {}",
                    package.code_slot().index()
                );
            }
            let Some(config) = package_values.get(package.package_build_id()) else {
                package_views.push(RuntimeConfigView::empty());
                continue;
            };
            let shape = skiff_artifact_model::config_shape_from_package_requirements(
                &package.artifact().runtime_requirements.config,
            )
            .with_context(|| {
                format!(
                    "Package {} has invalid config requirements",
                    package.package_build_id()
                )
            })?;
            package_views.push(
                RuntimeConfigView::from_resolved_config(config.config_value(), shape).with_context(
                    || {
                    format!(
                        "RuntimeConfigSnapshot deployment {:?} does not satisfy Package {} config requirements",
                        deployment,
                        package.package_build_id()
                    )
                    },
                )?,
            );
        }
        let implementation_slot = image
            .code_by_build(activation.implementation_package_build_id())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {:?} implementation Package is not in the execution image",
                    deployment
                )
            })?
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

pub(crate) fn validate_snapshot_environment(
    snapshot: &RuntimeConfigSnapshot,
    trusted_environment: &str,
) -> anyhow::Result<()> {
    if snapshot.environment() != trusted_environment {
        anyhow::bail!(
            "RuntimeConfigSnapshot {} rejected: environment mismatch",
            snapshot.snapshot_ref().snapshot_id
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn materialize_empty_config_for_test(
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
            .ok_or_else(|| anyhow::anyhow!("test activation implementation Package is missing"))?
            .code_slot()
            .index();
        let service = package_views
            .get(implementation_slot)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("test implementation Package config slot is missing"))?;
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

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct TestSnapshotResolveError;

#[cfg(test)]
impl std::fmt::Display for TestSnapshotResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("test config snapshot ref mismatch")
    }
}

#[cfg(test)]
impl std::error::Error for TestSnapshotResolveError {}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct TestSnapshotResolver {
    snapshot: RuntimeConfigSnapshot,
}

#[cfg(test)]
impl TestSnapshotResolver {
    pub(crate) fn new(snapshot: RuntimeConfigSnapshot) -> Self {
        Self { snapshot }
    }
}

#[cfg(test)]
impl skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver for TestSnapshotResolver {
    type Error = TestSnapshotResolveError;

    fn resolve(
        &self,
        reference: &skiff_artifact_model::RuntimeConfigSnapshotRef,
    ) -> Result<RuntimeConfigSnapshot, Self::Error> {
        (self.snapshot.snapshot_ref() == reference)
            .then(|| self.snapshot.clone())
            .ok_or(TestSnapshotResolveError)
    }
}

#[cfg(test)]
pub(crate) fn snapshot_for_assembly<R>(
    environment: &str,
    assembly: &skiff_artifact_model::RuntimeAssembly,
    resolver: &R,
) -> (
    skiff_artifact_model::RuntimeConfigSnapshotRef,
    TestSnapshotResolver,
)
where
    R: skiff_runtime_loader::RuntimeAssemblyContentResolver + ?Sized,
{
    let reference = skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref();
    let deployments = assembly
        .resolved_deployments
        .iter()
        .map(|deployment_ref| {
            let deployment = resolver
                .resolve_deployment(deployment_ref)
                .expect("test deployment should resolve");
            let mut packages = BTreeSet::from([deployment.implementation.package_build_id.clone()]);
            packages.extend(
                deployment
                    .package_bindings
                    .iter()
                    .map(|binding| binding.package.package_build_id.clone()),
            );
            skiff_runtime_config_snapshot::RuntimeConfigDeployment::new(
                deployment_ref.clone(),
                packages
                    .into_iter()
                    .map(|package_build_id| {
                        skiff_runtime_config_snapshot::RuntimeConfigPackage::new(
                            package_build_id,
                            Default::default(),
                        )
                        .expect("empty test Package config should be valid")
                    })
                    .collect(),
            )
            .expect("test config deployment should be valid")
        })
        .collect();
    let snapshot = RuntimeConfigSnapshot::new(environment, reference.clone(), deployments)
        .expect("test config snapshot should be valid");
    (reference, TestSnapshotResolver::new(snapshot))
}
