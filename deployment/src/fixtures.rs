use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_runtime_assembly_identity, assign_service_deployment_identity,
};
use skiff_artifact_model::{
    ActivationPolicy, ActivationTemplate, AssemblyIdentity, CanonicalPackageLinkPlan,
    ConfigLiteralBinding, ContractOperationId, DeploymentArtifactIdentity,
    DeploymentDiagnosticText, DeploymentIngressBinding, DeploymentOperationBinding,
    DeploymentPolicy, DeploymentRevision, IngressProtocol, IngressSelector, MetadataValue,
    PackageArtifactRef, PackageBuildId, PackageCallableId, PackageCodeSlot,
    PackageLocalAbiIdentity, ResourcePolicy, RuntimeAssembly, ServiceBindingTemplate,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentOperationInput, ServiceDeploymentRef,
    ServiceProtocolIdentity, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use crate::Result;

pub fn package_artifact_ref_fixture() -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: "example.provider".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("package-build"),
        package_local_abi_identity: PackageLocalAbiIdentity::new("package-abi"),
    }
}

pub fn service_contract_ref_fixture() -> ServiceContractRef {
    ServiceContractRef {
        service_id: "example.echo".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
    }
}

pub fn deployment_policy_fixture() -> DeploymentPolicy {
    DeploymentPolicy {
        timeout_ms: Some(5_000),
        resources: ResourcePolicy {
            cpu_millis: 100,
            memory_bytes: 1_048_576,
        },
        activation: ActivationPolicy {
            max_concurrency: 8,
            idle_timeout_ms: Some(30_000),
        },
        principal: "service:example.echo".to_string(),
    }
}

pub fn service_deployment_input_fixture() -> skiff_artifact_model::ServiceDeploymentInput {
    skiff_artifact_model::ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: service_contract_ref_fixture(),
        deployment_revision: DeploymentRevision::new("revision-1"),
        implementation: package_artifact_ref_fixture(),
        operation_bindings: vec![ServiceDeploymentOperationInput {
            contract_operation_id: ContractOperationId::new("operation.echo"),
            package_public_path: "echo".to_string(),
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        ingress: vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: "example.test".to_string(),
                method: Some("POST".to_string()),
                path: "/echo".to_string(),
            },
            contract_operation_id: ContractOperationId::new("operation.echo"),
        }],
        config_literals: vec![ConfigLiteralBinding {
            path: "message.prefix".to_string(),
            value: MetadataValue::String("hello".to_string()),
        }],
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: deployment_policy_fixture(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Echo deployment".to_string(),
            notes: BTreeMap::new(),
        },
    }
}

pub fn service_deployment_fixture() -> Result<ServiceDeployment> {
    let input = service_deployment_input_fixture();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: input.contract,
        deployment_revision: input.deployment_revision,
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: input.implementation,
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: ContractOperationId::new("operation.echo"),
            package_callable_id: PackageCallableId::new("callable.echo"),
        }],
        package_bindings: input.package_bindings,
        service_selectors: input.service_selectors,
        ingress: input.ingress,
        config_literals: input.config_literals,
        secret_refs: input.secret_refs,
        state_bindings: input.state_bindings,
        resource_bindings: input.resource_bindings,
        runtime_capability_bindings: input.runtime_capability_bindings,
        policy: input.policy,
        diagnostic_text: input.diagnostic_text,
    };
    assign_service_deployment_identity(&mut deployment)?;
    Ok(deployment)
}

pub fn empty_runtime_assembly_fixture() -> Result<RuntimeAssembly> {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    assign_runtime_assembly_identity(&mut assembly)?;
    Ok(assembly)
}

pub fn runtime_assembly_fixture() -> Result<RuntimeAssembly> {
    let deployment = service_deployment_fixture()?;
    let deployment_ref = ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    };
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: vec![deployment_ref.clone()],
        resolved_deployments: vec![deployment_ref.clone()],
        resolved_contracts: vec![deployment.contract.clone()],
        resolved_packages: vec![deployment.implementation.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: deployment.implementation.clone(),
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: vec![ServiceBindingTemplate {
            activation: deployment_ref.clone(),
            bindings: Vec::new(),
        }],
        activation_templates: vec![ActivationTemplate {
            deployment: deployment_ref.clone(),
            implementation_package_build_id: deployment.implementation.package_build_id.clone(),
            config_literals: deployment.config_literals.clone(),
            secret_refs: deployment.secret_refs.clone(),
            state_bindings: deployment.state_bindings.clone(),
            resource_bindings: deployment.resource_bindings.clone(),
            policy: deployment.policy.clone(),
        }],
        global_ingress: deployment
            .ingress
            .iter()
            .map(|ingress| (&deployment_ref, &deployment.contract, ingress).into())
            .collect(),
    };
    assign_runtime_assembly_identity(&mut assembly)?;
    Ok(assembly)
}
