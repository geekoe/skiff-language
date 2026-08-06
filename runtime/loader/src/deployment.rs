use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_identity::assign_runtime_assembly_identity;
use skiff_artifact_model::{
    ActivationTemplate, AssemblyIdentity, CanonicalPackageLinkPlan, GatewayIngressBinding,
    PackageArtifactRef, PackageBuildId, PackageCodeSlot, RuntimeAssembly, ServiceBindingTemplate,
    ServiceDeployment, ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};

use crate::{HydratedRuntimeAssembly, RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

/// Exact reachable package closure of one deployment (implementation + bindings).
fn deployment_package_closure(
    reference: &ServiceDeploymentRef,
    deployment: &ServiceDeployment,
) -> anyhow::Result<BTreeMap<PackageBuildId, PackageArtifactRef>> {
    let mut bindings = BTreeMap::new();
    for binding in &deployment.package_bindings {
        if bindings.insert(binding.key.clone(), &binding.package).is_some() {
            anyhow::bail!(
                "deployment {reference:?} repeats package binding {:?}",
                binding.key
            );
        }
    }
    let mut closure = BTreeMap::new();
    let mut pending = vec![deployment.implementation.clone()];
    while let Some(package) = pending.pop() {
        let build_id = &package.package_build_id;
        if closure.insert(build_id.clone(), package.clone()).is_some() {
            continue;
        }
        for (key, provider) in &bindings {
            if &key.caller_package_build_id == build_id {
                pending.push((*provider).clone());
            }
        }
    }
    let mut used = BTreeSet::new();
    for build_id in closure.keys() {
        for (key, _) in &bindings {
            if &key.caller_package_build_id == build_id {
                used.insert(key.clone());
            }
        }
    }
    if used != bindings.keys().cloned().collect::<BTreeSet<_>>() {
        anyhow::bail!(
            "deployment {reference:?} contains a package binding outside its reachable closure"
        );
    }
    Ok(closure)
}

/// Composes the minimal canonical RuntimeAssembly that exactly represents one
/// deployment record, reusing the whole-assembly load + link chain.
///
/// A deployment is only independently lazy-loadable when it has no
/// cross-service dependencies: `service_selectors` must be empty and the
/// reachable package closure must not declare service requirements (those
/// would require profile-level provider deployment resolution).
pub fn compose_deployment_assembly(
    reference: &ServiceDeploymentRef,
    deployment: &ServiceDeployment,
) -> anyhow::Result<RuntimeAssembly> {
    skiff_artifact_identity::validate_service_deployment_ref(reference, deployment)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("deployment content mismatches ref {reference:?}"))?;
    if !deployment.service_selectors.is_empty() {
        anyhow::bail!(
            "deployment {reference:?} has cross-service dependencies and cannot be lazily linked; \
             lazy-load deployments require an empty service selector set"
        );
    }
    let closure = deployment_package_closure(reference, deployment)?;
    let mut code_slots = closure
        .values()
        .map(|package| PackageCodeSlot {
            package: package.clone(),
        })
        .collect::<Vec<_>>();
    code_slots.sort_by(|left, right| {
        left.package
            .package_build_id
            .cmp(&right.package.package_build_id)
    });
    let resolved_packages = code_slots
        .iter()
        .map(|slot| slot.package.clone())
        .collect::<Vec<_>>();
    let package_links = deployment.package_bindings.clone();

    let gateway_ingress = deployment
        .ingress
        .iter()
        .map(|binding| {
            let entry = deployment
                .gateway_entries
                .get(&binding.gateway_entry_key)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "deployment {reference:?} ingress {:?} has no gateway entry {}",
                        binding.selector,
                        binding.gateway_entry_key
                    )
                })?;
            Ok(GatewayIngressBinding {
                selector: binding.selector.clone(),
                deployment: reference.clone(),
                gateway_entry_key: binding.gateway_entry_key.clone(),
                gateway_entry_identity: entry.gateway_entry_identity.clone(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: vec![reference.clone()],
        resolved_deployments: vec![reference.clone()],
        resolved_contracts: vec![deployment.contract.clone()],
        resolved_packages,
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots,
            package_links,
        },
        service_binding_templates: vec![ServiceBindingTemplate {
            activation: reference.clone(),
            bindings: Vec::new(),
        }],
        activation_templates: vec![ActivationTemplate {
            deployment: reference.clone(),
            implementation_package_build_id: deployment.implementation.package_build_id.clone(),
        }],
        gateway_ingress,
    };
    assign_runtime_assembly_identity(&mut assembly)
        .map_err(anyhow::Error::from)
        .context("failed to assign the composed deployment assembly identity")?;
    Ok(assembly)
}

/// Loads and hydrates one exact deployment record through the canonical
/// whole-assembly loader by composing the minimal deployment assembly.
pub struct DeploymentAssemblyLoader<'a, R: ?Sized> {
    resolver: &'a R,
}

impl<'a, R> DeploymentAssemblyLoader<'a, R>
where
    R: RuntimeAssemblyContentResolver + ?Sized,
{
    pub fn new(resolver: &'a R) -> Self {
        Self { resolver }
    }

    /// Resolve + compose + hydrate one exact immutable deployment reference.
    pub fn load_ref(&self, reference: &ServiceDeploymentRef) -> anyhow::Result<HydratedRuntimeAssembly> {
        let deployment = self
            .resolver
            .resolve_deployment(reference)
            .with_context(|| format!("failed to resolve deployment {reference:?}"))?;
        skiff_artifact_identity::validate_service_deployment_ref(reference, &deployment)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("deployment content mismatches ref {reference:?}"))?;
        let assembly = compose_deployment_assembly(reference, &deployment)
            .with_context(|| format!("failed to compose deployment assembly {reference:?}"))?;
        RuntimeAssemblyLoader::new(self.resolver)
            .load(Arc::new(assembly))
            .with_context(|| format!("failed to load deployment assembly {reference:?}"))
    }
}

#[cfg(test)]
mod tests;
