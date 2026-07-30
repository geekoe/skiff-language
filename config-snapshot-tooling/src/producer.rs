use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    validate_activation_environment, PackageArtifact, PackageArtifactRef, PackageBuildId,
    RuntimeAssembly, RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};
use skiff_runtime_config_snapshot::{new_runtime_config_snapshot_ref, RuntimeConfigSnapshotStore};

use crate::error::invalid;
use crate::{
    load_service_config, project_runtime_config_snapshot, ConfigSnapshotDeploymentInput,
    ConfigSnapshotPackageInput, ConfigSnapshotToolingResult,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceConfigSource {
    pub deployment: ServiceDeploymentRef,
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ConfigSnapshotProductionInput {
    pub environment: String,
    pub profile: String,
    pub assembly: RuntimeAssembly,
    pub package_artifacts: BTreeMap<PackageArtifactRef, PackageArtifact>,
    pub sources: Vec<ServiceConfigSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSnapshotProductionReceipt {
    pub snapshot: RuntimeConfigSnapshotRef,
    pub record_path: PathBuf,
    pub deployment_count: usize,
    pub package_count: usize,
}

pub fn produce_runtime_config_snapshot(
    input: ConfigSnapshotProductionInput,
    artifact_root: &Path,
) -> ConfigSnapshotToolingResult<ConfigSnapshotProductionReceipt> {
    validate_activation_environment(&input.environment)
        .map_err(|message| invalid("<environment>", message))?;
    let projection = projection_inputs(&input)?;
    let snapshot_ref = new_runtime_config_snapshot_ref();
    let snapshot =
        project_runtime_config_snapshot(&input.environment, snapshot_ref.clone(), projection)?;
    let store_root = artifact_root.join("runtime-config");
    let store = RuntimeConfigSnapshotStore::create(&store_root)?;
    let published = store.publish(&snapshot)?;
    let canonical_artifact_root = std::fs::canonicalize(artifact_root).map_err(|error| {
        invalid(
            artifact_root,
            format!("failed to resolve artifact root: {error}"),
        )
    })?;
    let record_path = published
        .strip_prefix(&canonical_artifact_root)
        .map_err(|_| invalid(&published, "snapshot record escaped artifact root"))?
        .to_path_buf();
    Ok(ConfigSnapshotProductionReceipt {
        snapshot: snapshot_ref,
        record_path,
        deployment_count: snapshot.deployments().len(),
        package_count: snapshot.package_count(),
    })
}

fn projection_inputs(
    input: &ConfigSnapshotProductionInput,
) -> ConfigSnapshotToolingResult<Vec<ConfigSnapshotDeploymentInput>> {
    let mut source_by_deployment = BTreeMap::new();
    for source in &input.sources {
        if source_by_deployment
            .insert(source.deployment.clone(), source.root.clone())
            .is_some()
        {
            return Err(invalid(
                &source.root,
                "more than one config source claims the same exact ServiceDeployment",
            ));
        }
    }
    let expected_deployments = input
        .assembly
        .activation_templates
        .iter()
        .map(|template| template.deployment.clone())
        .collect::<BTreeSet<_>>();
    let actual_deployments = source_by_deployment
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_deployments != expected_deployments {
        let missing = expected_deployments
            .difference(&actual_deployments)
            .map(|deployment| deployment.service_id.as_str())
            .collect::<Vec<_>>();
        let extra = actual_deployments
            .difference(&expected_deployments)
            .map(|deployment| deployment.service_id.as_str())
            .collect::<Vec<_>>();
        return Err(invalid(
            "<sources>",
            format!(
                "config sources must exactly match assembly deployments; missing [{}], extra [{}]",
                missing.join(", "),
                extra.join(", ")
            ),
        ));
    }

    let resolved_refs = input
        .assembly
        .resolved_packages
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let supplied_refs = input
        .package_artifacts
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if supplied_refs != resolved_refs {
        return Err(invalid(
            "<packages>",
            "supplied PackageArtifact records must exactly match RuntimeAssembly.resolvedPackages",
        ));
    }
    for (reference, artifact) in &input.package_artifacts {
        if artifact_reference(artifact) != *reference {
            return Err(invalid(
                "<packages>",
                format!(
                    "PackageArtifact {} does not match its exact assembly reference",
                    reference.package_id
                ),
            ));
        }
    }

    let reference_by_build = resolved_refs
        .iter()
        .map(|reference| (reference.package_build_id.clone(), reference.clone()))
        .collect::<BTreeMap<_, _>>();
    if reference_by_build.len() != resolved_refs.len() {
        return Err(invalid(
            "<assembly>",
            "RuntimeAssembly resolves one PackageBuildId to more than one PackageArtifact",
        ));
    }
    let mut links_by_caller = BTreeMap::<PackageBuildId, Vec<PackageArtifactRef>>::new();
    for binding in &input.assembly.package_link_plan.package_links {
        links_by_caller
            .entry(binding.key.caller_package_build_id.clone())
            .or_default()
            .push(binding.package.clone());
    }

    input
        .assembly
        .activation_templates
        .iter()
        .map(|template| {
            let source_root = source_by_deployment
                .get(&template.deployment)
                .expect("exact source set was validated");
            let config = load_service_config(source_root, &input.profile)?;
            let package_refs = deployment_package_closure(
                &template.implementation_package_build_id,
                &reference_by_build,
                &links_by_caller,
            )?;
            let packages = package_refs
                .into_iter()
                .map(|reference| {
                    let artifact = input
                        .package_artifacts
                        .get(&reference)
                        .expect("exact artifact set was validated");
                    Ok(ConfigSnapshotPackageInput {
                        package_id: artifact.package_id.clone(),
                        package_build_id: artifact.package_build_id.clone(),
                        requirements: artifact.runtime_requirements.config.clone(),
                    })
                })
                .collect::<ConfigSnapshotToolingResult<Vec<_>>>()?;
            Ok(ConfigSnapshotDeploymentInput {
                deployment: template.deployment.clone(),
                source_path: source_root.clone(),
                config,
                packages,
            })
        })
        .collect()
}

fn deployment_package_closure(
    implementation: &PackageBuildId,
    reference_by_build: &BTreeMap<PackageBuildId, PackageArtifactRef>,
    links_by_caller: &BTreeMap<PackageBuildId, Vec<PackageArtifactRef>>,
) -> ConfigSnapshotToolingResult<Vec<PackageArtifactRef>> {
    if !reference_by_build.contains_key(implementation) {
        return Err(invalid(
            "<assembly>",
            format!("activation implementation Package build {implementation:?} is unresolved"),
        ));
    }
    let mut queue = VecDeque::from([implementation.clone()]);
    let mut visited = BTreeSet::new();
    while let Some(build) = queue.pop_front() {
        if !visited.insert(build.clone()) {
            continue;
        }
        for dependency in links_by_caller.get(&build).into_iter().flatten() {
            if !reference_by_build.contains_key(&dependency.package_build_id) {
                return Err(invalid(
                    "<assembly>",
                    format!(
                        "Package link from {build:?} targets an unresolved Package build {:?}",
                        dependency.package_build_id
                    ),
                ));
            }
            queue.push_back(dependency.package_build_id.clone());
        }
    }
    Ok(visited
        .into_iter()
        .map(|build| {
            reference_by_build
                .get(&build)
                .expect("visited builds were validated")
                .clone()
        })
        .collect())
}

fn artifact_reference(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}
