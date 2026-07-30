use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{AssemblyIdentity, RuntimeAssembly, RuntimeAssemblyRef};

use crate::{
    framing::{canonical_ir_bytes, sha256_hex},
    ArtifactIdentityError, Result, ASSEMBLY_IDENTITY_PREFIX, ASSEMBLY_IDENTITY_SCHEMA_MARKER,
};

mod normalization;
mod validation;

/// Complete canonical preimage of `AssemblyIdentity`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssemblyIdentityProjection {
    schema: &'static str,
    roots: Value,
    resolved_deployments: Value,
    resolved_contracts: Value,
    resolved_packages: Value,
    package_link_plan: Value,
    service_binding_templates: Value,
    activation_templates: Value,
    gateway_ingress: Value,
}

pub use validation::validate_runtime_assembly_surface;

pub fn runtime_assembly_identity_projection(
    assembly: &RuntimeAssembly,
) -> Result<AssemblyIdentityProjection> {
    validate_runtime_assembly_surface(assembly)?;
    let mut projection = AssemblyIdentityProjection {
        schema: ASSEMBLY_IDENTITY_SCHEMA_MARKER,
        roots: identity_value(&assembly.roots)?,
        resolved_deployments: identity_value(&assembly.resolved_deployments)?,
        resolved_contracts: identity_value(&assembly.resolved_contracts)?,
        resolved_packages: identity_value(&assembly.resolved_packages)?,
        package_link_plan: identity_value(&assembly.package_link_plan)?,
        service_binding_templates: identity_value(&assembly.service_binding_templates)?,
        activation_templates: identity_value(&assembly.activation_templates)?,
        gateway_ingress: identity_value(&assembly.gateway_ingress)?,
    };
    normalization::normalize_projection(&mut projection);
    Ok(projection)
}

fn identity_value<T: Serialize>(value: &T) -> Result<Value> {
    crate::identity_labels::without_human_version_labels(
        value,
        ArtifactIdentityError::SerializeAssemblyIdentity,
    )
}

pub fn runtime_assembly_identity(assembly: &RuntimeAssembly) -> Result<AssemblyIdentity> {
    let projection = runtime_assembly_identity_projection(assembly)?;
    let bytes = canonical_ir_bytes(
        &projection,
        ArtifactIdentityError::SerializeAssemblyIdentity,
    )?;
    Ok(AssemblyIdentity::new(crate::framed_identity(
        ASSEMBLY_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn assign_runtime_assembly_identity(
    assembly: &mut RuntimeAssembly,
) -> Result<AssemblyIdentity> {
    let identity = runtime_assembly_identity(assembly)?;
    assembly.assembly_identity = identity.clone();
    validate_runtime_assembly_identity(assembly)?;
    Ok(identity)
}

pub fn validate_runtime_assembly_identity(assembly: &RuntimeAssembly) -> Result<()> {
    let computed = runtime_assembly_identity(assembly)?;
    if assembly.assembly_identity != computed {
        return Err(ArtifactIdentityError::AssemblyIdentityMismatch {
            declared: assembly.assembly_identity.to_string(),
            computed: computed.to_string(),
        });
    }
    Ok(())
}

pub fn runtime_assembly_ref(assembly: &RuntimeAssembly) -> Result<RuntimeAssemblyRef> {
    validate_runtime_assembly_identity(assembly)?;
    Ok(RuntimeAssemblyRef {
        assembly_identity: assembly.assembly_identity.clone(),
    })
}

fn invalid_assembly<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidRuntimeAssembly {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_artifact_model::{
        ActivationTemplate, AssemblyIdentity, CanonicalPackageLinkPlan, DeploymentArtifactIdentity,
        DeploymentPolicy, DeploymentRevision, GatewayEntryIdentity, GatewayEntryKey,
        GatewayIngressBinding, IngressProtocol, IngressSelector, PackageArtifactRef,
        PackageBuildId, PackageCodeSlot, PackageLocalAbiIdentity, ResourcePolicy, RuntimeAssembly,
        RuntimeAssemblyRef, ServiceBindingTemplate, ServiceDeploymentRef,
        GATEWAY_ENTRY_IDENTITY_PREFIX, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };

    use super::*;

    fn deployment(service: &str, revision: &str) -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: service.to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new(revision),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "deployment-{revision}"
            )),
        }
    }

    fn gateway_identity(fill: char) -> GatewayEntryIdentity {
        GatewayEntryIdentity::parse(format!(
            "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
            fill.to_string().repeat(64)
        ))
        .unwrap()
    }

    fn runtime_assembly_fixture() -> RuntimeAssembly {
        let deployment = deployment("example.gateway", "revision-1");
        let package = PackageArtifactRef {
            package_id: "example.gateway.package".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("package-build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("package-local-abi"),
        };
        let key = GatewayEntryKey::parse("create-user").unwrap();
        let binding = |path: &str| GatewayIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: path.to_string(),
            },
            deployment: deployment.clone(),
            gateway_entry_key: key.clone(),
            gateway_entry_identity: gateway_identity('a'),
        };
        let mut assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("unassigned"),
            roots: vec![deployment.clone()],
            resolved_deployments: vec![deployment.clone()],
            resolved_contracts: Vec::new(),
            resolved_packages: vec![package.clone()],
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot { package }],
                package_links: Vec::new(),
            },
            service_binding_templates: vec![ServiceBindingTemplate {
                activation: deployment.clone(),
                bindings: Vec::new(),
            }],
            activation_templates: vec![ActivationTemplate {
                deployment: deployment.clone(),
                implementation_package_build_id: PackageBuildId::new("package-build"),
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                policy: DeploymentPolicy {
                    timeout_ms: Some(1000),
                    resources: ResourcePolicy {
                        cpu_millis: 100,
                        memory_bytes: 1024,
                    },
                    principal: "gateway-test".to_string(),
                },
            }],
            gateway_ingress: vec![binding("/users"), binding("/users/alias")],
        };
        assign_runtime_assembly_identity(&mut assembly).unwrap();
        assembly
    }

    #[test]
    fn runtime_assembly_v3_identity_covers_gateway_binding_and_normalizes_order() {
        let assembly = runtime_assembly_fixture();
        let expected = runtime_assembly_identity(&assembly).unwrap();

        let mut reordered = assembly.clone();
        reordered.gateway_ingress.reverse();
        assert_eq!(runtime_assembly_identity(&reordered).unwrap(), expected);

        let mut selector = assembly.clone();
        selector.gateway_ingress[0].selector.path = "/changed".to_string();
        assert_ne!(runtime_assembly_identity(&selector).unwrap(), expected);

        let mut exact_deployment = assembly.clone();
        let changed = deployment("example.gateway", "revision-2");
        exact_deployment.roots[0] = changed.clone();
        exact_deployment.resolved_deployments[0] = changed.clone();
        exact_deployment.service_binding_templates[0].activation = changed.clone();
        exact_deployment.activation_templates[0].deployment = changed.clone();
        for binding in &mut exact_deployment.gateway_ingress {
            binding.deployment = changed.clone();
        }
        assert_ne!(
            runtime_assembly_identity(&exact_deployment).unwrap(),
            expected
        );

        let mut key = assembly.clone();
        key.gateway_ingress[0].gateway_entry_key = GatewayEntryKey::parse("other").unwrap();
        assert_ne!(runtime_assembly_identity(&key).unwrap(), expected);

        let mut identity = assembly;
        identity.gateway_ingress[0].gateway_entry_identity = gateway_identity('b');
        assert_ne!(runtime_assembly_identity(&identity).unwrap(), expected);
    }

    #[test]
    fn runtime_assembly_ingress_collision_is_scoped_to_exact_deployment() {
        let mut assembly = runtime_assembly_fixture();
        let second = deployment("example.other", "revision-1");
        assembly.roots.push(second.clone());
        assembly.resolved_deployments.push(second.clone());
        assembly
            .service_binding_templates
            .push(ServiceBindingTemplate {
                activation: second.clone(),
                bindings: Vec::new(),
            });
        let mut activation = assembly.activation_templates[0].clone();
        activation.deployment = second.clone();
        assembly.activation_templates.push(activation);
        let mut shared_selector = assembly.gateway_ingress[0].clone();
        shared_selector.deployment = second;
        shared_selector.gateway_entry_key = GatewayEntryKey::parse("other-create-user").unwrap();
        assembly.gateway_ingress.push(shared_selector);

        validate_runtime_assembly_surface(&assembly)
            .expect("different services may share the same service-local selector");

        let mut duplicate = assembly;
        duplicate
            .gateway_ingress
            .push(duplicate.gateway_ingress[0].clone());
        assert!(validate_runtime_assembly_surface(&duplicate).is_err());
    }

    #[test]
    fn runtime_assembly_rejects_multiple_deployments_for_one_service_coordinate() {
        let mut assembly = runtime_assembly_fixture();
        assembly
            .resolved_deployments
            .push(deployment("example.gateway", "revision-2"));
        assert!(validate_runtime_assembly_surface(&assembly).is_err());
    }

    #[test]
    fn runtime_assembly_old_schemas_and_identity_prefixes_are_rejected() {
        let assembly = runtime_assembly_fixture();
        for legacy in ["skiff-runtime-assembly-v1", "skiff-runtime-assembly-v2"] {
            let mut stale_schema = assembly.clone();
            stale_schema.schema_version = legacy.to_string();
            assert!(validate_runtime_assembly_surface(&stale_schema).is_err());

            let stale_ref = json!({
                "assemblyIdentity": format!("{legacy}:sha256:{}", "a".repeat(64))
            });
            assert!(serde_json::from_value::<RuntimeAssemblyRef>(stale_ref).is_err());
        }
        assert!(assembly
            .assembly_identity
            .as_str()
            .starts_with("skiff-runtime-assembly-v3:sha256:"));
    }
}
