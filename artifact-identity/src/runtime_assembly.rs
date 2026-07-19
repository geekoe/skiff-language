use serde::Serialize;
use skiff_artifact_model::{
    ActivationTemplate, AssemblyIdentity, CanonicalPackageLinkPlan, GlobalIngressBinding,
    PackageArtifactRef, RuntimeAssembly, ServiceBindingTemplate, ServiceContractRef,
    ServiceDeploymentRef,
};

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
    roots: Vec<ServiceDeploymentRef>,
    resolved_deployments: Vec<ServiceDeploymentRef>,
    resolved_contracts: Vec<ServiceContractRef>,
    resolved_packages: Vec<PackageArtifactRef>,
    package_link_plan: CanonicalPackageLinkPlan,
    service_binding_templates: Vec<ServiceBindingTemplate>,
    activation_templates: Vec<ActivationTemplate>,
    global_ingress: Vec<GlobalIngressBinding>,
}

pub use validation::validate_runtime_assembly_surface;

pub fn runtime_assembly_identity_projection(
    assembly: &RuntimeAssembly,
) -> Result<AssemblyIdentityProjection> {
    validate_runtime_assembly_surface(assembly)?;
    let mut projection = AssemblyIdentityProjection {
        schema: ASSEMBLY_IDENTITY_SCHEMA_MARKER,
        roots: assembly.roots.clone(),
        resolved_deployments: assembly.resolved_deployments.clone(),
        resolved_contracts: assembly.resolved_contracts.clone(),
        resolved_packages: assembly.resolved_packages.clone(),
        package_link_plan: assembly.package_link_plan.clone(),
        service_binding_templates: assembly.service_binding_templates.clone(),
        activation_templates: assembly.activation_templates.clone(),
        global_ingress: assembly.global_ingress.clone(),
    };
    normalization::normalize_projection(&mut projection);
    Ok(projection)
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

fn invalid_assembly<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidRuntimeAssembly {
        message: message.into(),
    })
}
