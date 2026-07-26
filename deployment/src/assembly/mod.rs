//! Deterministic runtime assembly resolution from frozen deployment artifacts.

use std::collections::BTreeSet;

use skiff_artifact_identity::assign_runtime_assembly_identity;
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, PackageArtifact, RuntimeAssembly, ServiceContract,
    ServiceDeployment, ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};

mod candidates;
mod error;
mod resolver;

use candidates::CandidateIndex;
use resolver::Resolver;

pub use error::{AssemblyResolutionError, AssemblyResult};

/// Resolves an exact local executable closure from immutable candidates.
///
/// Service bindings emitted by this first-version resolver are always
/// `InProcessBoundary`: a missing local provider is an error and never becomes
/// a router or remote fallback. Root order and candidate insertion order do not
/// affect the returned assembly or its identity.
pub fn resolve_runtime_assembly(
    roots: &[ServiceDeploymentRef],
    deployments: &[ServiceDeployment],
    contracts: &[ServiceContract],
    packages: &[PackageArtifact],
) -> AssemblyResult<RuntimeAssembly> {
    let normalized_roots = roots.iter().cloned().collect::<BTreeSet<_>>();
    if normalized_roots.is_empty() {
        return finish_assembly(empty_assembly());
    }

    let candidates = CandidateIndex::new(deployments, contracts, packages)?;
    for root in &normalized_roots {
        if !candidates.contains_deployment(root) {
            return Err(AssemblyResolutionError::MissingRoot(root.clone()));
        }
    }

    let mut resolver = Resolver::new(&candidates, normalized_roots.iter().cloned());
    resolver.resolve()?;
    finish_assembly(resolver.into_assembly(normalized_roots))
}

fn empty_assembly() -> RuntimeAssembly {
    RuntimeAssembly {
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
        gateway_ingress: Vec::new(),
    }
}

fn finish_assembly(mut assembly: RuntimeAssembly) -> AssemblyResult<RuntimeAssembly> {
    assign_runtime_assembly_identity(&mut assembly)?;
    Ok(assembly)
}

#[cfg(test)]
mod tests;
