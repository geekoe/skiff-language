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
mod tests;
