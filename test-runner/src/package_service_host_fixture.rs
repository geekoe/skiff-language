use std::{fs, path::Path};

use serde_json::{json, Value};
use skiff_artifact_model::{
    ActivationPolicy, ConfigLiteralBinding, DeploymentDiagnosticText, DeploymentPolicy,
    DeploymentRevision, MetadataValue, PackageArtifactRef, PackageBinding, PackageRequirementKey,
    ResourcePolicy, RuntimeAssemblyAuthoring, RuntimeAssemblyRef, ServiceContractRef,
    ServiceDeploymentInput, ServiceDeploymentOperationInput, ServiceDeploymentRef,
    ServiceRequirementKey, ServiceSelectorBinding, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};
use skiff_deployment::storage::CanonicalArtifactStore;

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
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
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
    fixture_root: &Path,
    work_root: &Path,
    artifact_root: &Path,
    environment: &str,
) -> anyhow::Result<PackageServiceHostFixtureReceipt> {
    if environment.trim().is_empty() {
        anyhow::bail!("host fixture environment must not be empty");
    }
    fs::create_dir_all(work_root)?;

    let payments_contract =
        publish_contract(&fixture_root.join("payments-contract"), artifact_root)?;
    let consumer_contract =
        publish_contract(&fixture_root.join("consumer-contract"), artifact_root)?;
    let helper_package = publish_package(&fixture_root.join("helper"), artifact_root)?;
    let provider_package = publish_package(&fixture_root.join("provider"), artifact_root)?;

    let store = CanonicalArtifactStore::open(artifact_root)?;
    let provider_deployment = publish_provider_deployment(
        work_root,
        artifact_root,
        &store,
        &payments_contract,
        &provider_package,
    )?;

    let consumer_package = publish_package(&fixture_root.join("consumer"), artifact_root)?;
    let consumer_deployment = publish_consumer_deployment(
        work_root,
        artifact_root,
        &store,
        &consumer_contract,
        &payments_contract,
        &consumer_package,
        &helper_package,
    )?;

    let base_assembly = publish_assembly(
        &work_root.join("base-assembly"),
        artifact_root,
        &RuntimeAssemblyAuthoring {
            environment: environment.to_string(),
            root_deployments: vec![consumer_deployment.clone()],
        },
    )?;

    Ok(PackageServiceHostFixtureReceipt {
        environment: environment.to_string(),
        payments_contract,
        consumer_contract,
        helper_package,
        provider_package,
        consumer_package,
        provider_deployment,
        consumer_deployment,
        base_assembly,
    })
}

fn publish_provider_deployment(
    work_root: &Path,
    artifact_root: &Path,
    store: &CanonicalArtifactStore,
    payments_contract: &ServiceContractRef,
    provider_package: &PackageArtifactRef,
) -> anyhow::Result<ServiceDeploymentRef> {
    let payments_operation = contract_operation(store, payments_contract, "echo")?;
    publish_deployment(
        &work_root.join("provider-deployment"),
        artifact_root,
        &ServiceDeploymentInput {
            schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
            contract: payments_contract.clone(),
            deployment_revision: DeploymentRevision::new("provider-r1"),
            implementation: provider_package.clone(),
            operation_bindings: vec![ServiceDeploymentOperationInput {
                contract_operation_id: payments_operation,
                package_public_path: "handle".to_string(),
            }],
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            ingress: Vec::new(),
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            runtime_capability_bindings: Vec::new(),
            policy: deployment_policy("service:provider"),
            diagnostic_text: deployment_diagnostic("Provider"),
        },
    )
}

fn publish_consumer_deployment(
    work_root: &Path,
    artifact_root: &Path,
    store: &CanonicalArtifactStore,
    consumer_contract: &ServiceContractRef,
    payments_contract: &ServiceContractRef,
    consumer_package: &PackageArtifactRef,
    helper_package: &PackageArtifactRef,
) -> anyhow::Result<ServiceDeploymentRef> {
    let consumer_artifact = store.read_package_artifact(consumer_package)?;
    let package_requirement = consumer_artifact
        .package_requirements
        .iter()
        .find(|requirement| requirement.alias == "helper")
        .ok_or_else(|| anyhow::anyhow!("consumer fixture omitted helper package requirement"))?;
    let service_requirement = consumer_artifact
        .service_requirements
        .iter()
        .find(|requirement| requirement.contract_requirement.alias == "payments")
        .ok_or_else(|| anyhow::anyhow!("consumer fixture omitted payments service requirement"))?;
    let consumer_operation = contract_operation(store, consumer_contract, "echo")?;
    publish_deployment(
        &work_root.join("consumer-deployment"),
        artifact_root,
        &ServiceDeploymentInput {
            schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
            contract: consumer_contract.clone(),
            deployment_revision: DeploymentRevision::new("consumer-r1"),
            implementation: consumer_package.clone(),
            operation_bindings: vec![ServiceDeploymentOperationInput {
                contract_operation_id: consumer_operation,
                package_public_path: "owner".to_string(),
            }],
            package_bindings: vec![PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: consumer_package.package_build_id.clone(),
                    package_requirement_alias: package_requirement.alias.clone(),
                },
                package: helper_package.clone(),
            }],
            service_selectors: vec![ServiceSelectorBinding {
                key: ServiceRequirementKey {
                    caller_package_build_id: consumer_package.package_build_id.clone(),
                    service_requirement_slot: service_requirement.service_binding_slot,
                },
                contract: payments_contract.clone(),
            }],
            ingress: Vec::new(),
            config_literals: vec![ConfigLiteralBinding {
                path: "app.token".to_string(),
                value: MetadataValue::String("owned-by-base".to_string()),
            }],
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            runtime_capability_bindings: Vec::new(),
            policy: deployment_policy("service:consumer"),
            diagnostic_text: deployment_diagnostic("Consumer"),
        },
    )
}

fn publish_contract(root: &Path, artifact_root: &Path) -> anyhow::Result<ServiceContractRef> {
    let receipt = author(AuthoringObject::Contract, root, artifact_root)?;
    Ok(serde_json::from_value(
        receipt["serviceContractReceipt"]["contract"].clone(),
    )?)
}

fn publish_package(root: &Path, artifact_root: &Path) -> anyhow::Result<PackageArtifactRef> {
    let receipt = author(AuthoringObject::Package, root, artifact_root)?;
    Ok(serde_json::from_value(
        receipt["packageArtifactReceipt"]["artifact"].clone(),
    )?)
}

fn publish_deployment(
    root: &Path,
    artifact_root: &Path,
    input: &ServiceDeploymentInput,
) -> anyhow::Result<ServiceDeploymentRef> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("deployment.yml"),
        format!("{}\n", serde_json::to_string_pretty(input)?),
    )?;
    let receipt = author(AuthoringObject::Deployment, root, artifact_root)?;
    Ok(serde_json::from_value(
        receipt["serviceDeploymentReceipt"]["deployment"].clone(),
    )?)
}

fn publish_assembly(
    root: &Path,
    artifact_root: &Path,
    input: &RuntimeAssemblyAuthoring,
) -> anyhow::Result<RuntimeAssemblyRef> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("assembly.yml"),
        format!("{}\n", serde_json::to_string_pretty(input)?),
    )?;
    let receipt = author(AuthoringObject::Assembly, root, artifact_root)?;
    Ok(serde_json::from_value(
        receipt["runtimeAssemblyReceipt"]["assembly"].clone(),
    )?)
}

fn author(object: AuthoringObject, root: &Path, artifact_root: &Path) -> anyhow::Result<Value> {
    build_authoring_object(object, root, artifact_root, true)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn contract_operation(
    store: &CanonicalArtifactStore,
    contract: &ServiceContractRef,
    stable_key: &str,
) -> anyhow::Result<skiff_artifact_model::ContractOperationId> {
    let contract = store.read_service_contract(contract)?;
    contract
        .operations
        .iter()
        .find(|(_, operation)| operation.stable_key == stable_key)
        .map(|(operation_id, _)| operation_id.clone())
        .ok_or_else(|| anyhow::anyhow!("contract omitted operation {stable_key}"))
}

fn deployment_policy(principal: &str) -> DeploymentPolicy {
    DeploymentPolicy {
        timeout_ms: 1_000,
        resources: ResourcePolicy {
            cpu_millis: 100,
            memory_bytes: 1_048_576,
        },
        activation: ActivationPolicy {
            max_concurrency: 1,
            idle_timeout_ms: None,
        },
        principal: principal.to_string(),
    }
}

fn deployment_diagnostic(display_name: &str) -> DeploymentDiagnosticText {
    DeploymentDiagnosticText {
        display_name: display_name.to_string(),
        notes: Default::default(),
    }
}
