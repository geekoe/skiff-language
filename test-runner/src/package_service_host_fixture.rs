use std::{collections::BTreeMap, fs, path::Path};

use serde_json::{json, Value};
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, RuntimeAssemblyRef, RuntimeConfigSnapshotRef,
    ServiceContractRef, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, project_runtime_assembly, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_config_snapshot_tooling::{
    produce_runtime_config_snapshot, ConfigSnapshotProductionInput, ServiceConfigSource,
};
use skiff_deployment::storage::CanonicalArtifactStore;

pub const PACKAGE_SERVICE_HOST_FIXTURE_SCHEMA_VERSION: &str =
    "skiff-package-service-host-fixture-v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageServiceHostFixtureReceipt {
    pub environment: String,
    pub payments_contract: ServiceContractRef,
    pub consumer_contract: ServiceContractRef,
    pub helper_package: PackageArtifactRef,
    pub provider_package: PackageArtifactRef,
    pub consumer_package: PackageArtifactRef,
    pub provider_deployment: ServiceDeploymentRef,
    pub consumer_deployment: ServiceDeploymentRef,
    pub base_assembly: RuntimeAssemblyRef,
    pub base_config_snapshot: RuntimeConfigSnapshotRef,
}

impl PackageServiceHostFixtureReceipt {
    pub fn to_json(&self) -> Value {
        json!({
            "schemaVersion": PACKAGE_SERVICE_HOST_FIXTURE_SCHEMA_VERSION,
            "environment": self.environment,
            "contracts": {
                "payments": self.payments_contract,
                "consumer": self.consumer_contract,
            },
            "packages": {
                "helper": self.helper_package,
                "provider": self.provider_package,
                "consumer": self.consumer_package,
            },
            "deployments": {
                "provider": self.provider_deployment,
                "consumer": self.consumer_deployment,
            },
            "baseAssembly": self.base_assembly,
            "baseConfigSnapshot": self.base_config_snapshot,
        })
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&self.to_json())?),
        )?;
        Ok(())
    }
}

pub fn prepare_package_service_host_fixture(
    platform_sources: &CompilerPlatformSources,
    fixture_root: &Path,
    work_root: &Path,
    artifact_root: &Path,
    environment: &str,
) -> anyhow::Result<PackageServiceHostFixtureReceipt> {
    if environment.trim().is_empty() {
        anyhow::bail!("host fixture environment must not be empty");
    }
    fs::create_dir_all(work_root)?;

    let helper_package = publish_package(
        platform_sources,
        &fixture_root.join("helper"),
        artifact_root,
        environment,
    )?;
    let provider_root = prepare_service_root(
        &fixture_root.join("provider"),
        &work_root.join("provider"),
        environment,
        false,
    )?;
    let provider =
        publish_service_package(platform_sources, &provider_root, artifact_root, environment)?;
    let consumer_root = prepare_service_root(
        &fixture_root.join("consumer"),
        &work_root.join("consumer"),
        environment,
        true,
    )?;
    let consumer =
        publish_service_package(platform_sources, &consumer_root, artifact_root, environment)?;

    let root_deployments = [provider.deployment.clone(), consumer.deployment.clone()];
    let base_assembly = project_assembly(artifact_root, environment, &root_deployments)?;
    let base_config_snapshot = project_config_snapshot(
        artifact_root,
        environment,
        &base_assembly,
        [
            (provider.deployment.clone(), provider_root),
            (consumer.deployment.clone(), consumer_root),
        ],
    )?;

    Ok(PackageServiceHostFixtureReceipt {
        environment: environment.to_string(),
        payments_contract: provider.contract,
        consumer_contract: consumer.contract,
        helper_package,
        provider_package: provider.package,
        consumer_package: consumer.package,
        provider_deployment: provider.deployment,
        consumer_deployment: consumer.deployment,
        base_assembly,
        base_config_snapshot,
    })
}

struct ServicePackageReceipt {
    package: PackageArtifactRef,
    contract: ServiceContractRef,
    deployment: ServiceDeploymentRef,
}

fn publish_service_package(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
    environment: &str,
) -> anyhow::Result<ServicePackageReceipt> {
    let receipt = author(
        platform_sources,
        AuthoringObject::Package,
        root,
        artifact_root,
        environment,
    )?;
    Ok(ServicePackageReceipt {
        package: serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())?,
        contract: serde_json::from_value(receipt["serviceContractReceipt"]["contract"].clone())?,
        deployment: serde_json::from_value(
            receipt["serviceDeploymentReceipt"]["deployment"].clone(),
        )?,
    })
}

fn prepare_service_root(
    source: &Path,
    target: &Path,
    environment: &str,
    configured: bool,
) -> anyhow::Result<std::path::PathBuf> {
    copy_fixture_tree(source, target)?;
    if configured {
        fs::write(
            target.join(format!("config.{environment}.yml")),
            "\"example.com/consumer\":\n  app:\n    token: owned-by-base\n",
        )?;
    }
    Ok(target.to_path_buf())
}

fn project_config_snapshot(
    artifact_root: &Path,
    profile: &str,
    assembly_ref: &RuntimeAssemblyRef,
    sources: [(ServiceDeploymentRef, std::path::PathBuf); 2],
) -> anyhow::Result<RuntimeConfigSnapshotRef> {
    let store = CanonicalArtifactStore::open(artifact_root)?;
    let assembly = store.read_runtime_assembly(assembly_ref)?.as_ref().clone();
    let package_artifacts = assembly
        .resolved_packages
        .iter()
        .map(|reference| {
            let artifact = store.read_package_artifact(reference)?.as_ref().clone();
            Ok((reference.clone(), artifact))
        })
        .collect::<Result<BTreeMap<PackageArtifactRef, PackageArtifact>, _>>()?;
    let receipt = produce_runtime_config_snapshot(
        ConfigSnapshotProductionInput {
            profile: profile.to_string(),
            assembly,
            package_artifacts,
            sources: sources
                .into_iter()
                .map(|(deployment, root)| ServiceConfigSource { deployment, root })
                .collect(),
        },
        artifact_root,
    )?;
    Ok(receipt.snapshot)
}

fn copy_fixture_tree(source: &Path, target: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_fixture_tree(&source_path, &target_path)?;
        } else {
            fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

fn publish_package(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
    environment: &str,
) -> anyhow::Result<PackageArtifactRef> {
    let receipt = author(
        platform_sources,
        AuthoringObject::Package,
        root,
        artifact_root,
        environment,
    )?;
    Ok(serde_json::from_value(
        receipt["packageArtifactReceipt"]["artifact"].clone(),
    )?)
}

fn project_assembly(
    artifact_root: &Path,
    environment: &str,
    root_deployments: &[ServiceDeploymentRef],
) -> anyhow::Result<RuntimeAssemblyRef> {
    let receipt = project_runtime_assembly(artifact_root, environment, root_deployments, true)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(serde_json::from_value(
        receipt["runtimeAssemblyReceipt"]["assembly"].clone(),
    )?)
}

fn author(
    platform_sources: &CompilerPlatformSources,
    object: AuthoringObject,
    root: &Path,
    artifact_root: &Path,
    environment: &str,
) -> anyhow::Result<Value> {
    build_authoring_object(
        platform_sources,
        object,
        root,
        artifact_root,
        environment,
        true,
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::copy_fixture_tree;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn recursive_copy_fixture_tree_receipt_preserves_external_control_files() {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skiff-host-fixture-copy-receipt-{}-{sequence}",
            std::process::id()
        ));
        let source = root.join("source");
        let target = root.join("target");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("http.yml"), "probe: { method: GET }\n").unwrap();
        fs::write(source.join("nested/websocket.yml"), "path: /socket\n").unwrap();
        fs::write(
            source.join("nested/source.skiff"),
            "function marker() -> bool { return true }\n",
        )
        .unwrap();

        copy_fixture_tree(&source, &target).unwrap();

        let receipt = ["http.yml", "nested/websocket.yml"].map(|path| {
            (
                path,
                fs::read(source.join(path)).unwrap(),
                fs::read(target.join(path)).unwrap(),
            )
        });
        assert!(
            receipt
                .iter()
                .all(|(_, source_bytes, copied_bytes)| source_bytes == copied_bytes),
            "recursive copy receipt must retain exact external control-file bytes"
        );
        assert_eq!(
            fs::read(target.join("nested/source.skiff")).unwrap(),
            fs::read(source.join("nested/source.skiff")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
