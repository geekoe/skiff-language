use std::{fs, path::Path};

use serde_json::{json, Value};
use skiff_artifact_model::{
    PackageArtifactRef, RuntimeAssemblyAuthoring, RuntimeAssemblyRef, ServiceContractRef,
    ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};

pub const PACKAGE_SERVICE_HOST_FIXTURE_SCHEMA_VERSION: &str =
    "skiff-package-service-host-fixture-v1";

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

    let base_assembly = publish_assembly(
        platform_sources,
        &work_root.join("base-assembly"),
        artifact_root,
        &RuntimeAssemblyAuthoring {
            environment: environment.to_string(),
            root_deployments: vec![consumer.deployment.clone()],
        },
        environment,
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
    let config_values = if configured {
        "config:\n  app.token: owned-by-base\n"
    } else {
        ""
    };
    let principal = if configured {
        "service:consumer"
    } else {
        "service:provider"
    };
    fs::write(
        target.join(format!("config.{environment}.yml")),
        format!(
            "{config_values}timeout: 1000\nquota:\n  cpuMillis: 100\n  memoryBytes: 1048576\nlifecycle:\n  maxConcurrency: 1\nprincipal: {principal}\n"
        ),
    )?;
    Ok(target.to_path_buf())
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

fn publish_assembly(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
    input: &RuntimeAssemblyAuthoring,
    environment: &str,
) -> anyhow::Result<RuntimeAssemblyRef> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("assembly.yml"),
        format!("{}\n", serde_json::to_string_pretty(input)?),
    )?;
    let receipt = author(
        platform_sources,
        AuthoringObject::Assembly,
        root,
        artifact_root,
        environment,
    )?;
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
