use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_identity::assign_runtime_assembly_identity;
use skiff_artifact_model::{
    ActivationTemplate, AssemblyIdentity, CanonicalPackageLinkPlan, GatewayIngressBinding,
    PackageArtifactRef, PackageBuildId, PackageCodeSlot, RuntimeAssembly, ServiceBindingTemplate,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_deployment::assembly::resolve_runtime_assembly;

use crate::{HydratedRuntimeAssembly, RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

/// Release-pointer lookup boundary for dependency closure resolution.
///
/// The human coordinate `(profile, service_id, version)` resolves to the exact
/// provider deployment currently published by the release pointer table
/// (`CanonicalArtifactStore::read_release_pointer`). `None` means the pointer
/// is not set; dependency resolution must fail closed.
pub trait DeploymentReleasePointerResolver {
    fn resolve_release_pointer(
        &self,
        profile: &str,
        service_id: &str,
        version: &str,
    ) -> anyhow::Result<Option<ServiceDeploymentRef>>;
}

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

/// Composes the canonical RuntimeAssembly that exactly represents one
/// self-contained deployment record (no cross-service dependencies), reusing
/// the whole-assembly load + link chain.
///
/// A deployment with cross-service dependencies is not independently
/// lazy-loadable: use [`compose_dependency_closure_assembly`] so its provider
/// deployments share the same assembly and the linker binding holds.
pub fn compose_deployment_assembly(
    reference: &ServiceDeploymentRef,
    deployment: &ServiceDeployment,
) -> anyhow::Result<RuntimeAssembly> {
    skiff_artifact_identity::validate_service_deployment_ref(reference, deployment)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("deployment content mismatches ref {reference:?}"))?;
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

/// Exact dependency closure of one deployment entry: the entry plus every
/// provider deployment reachable through `service_selectors` and package-level
/// `service_requirements`, resolved through the release pointer table.
#[derive(Debug, Default)]
struct DeploymentClosure {
    deployments: BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: BTreeSet<ServiceContractRef>,
    packages: BTreeSet<PackageArtifactRef>,
}

/// Composes the canonical assembly for the whole dependency closure of one
/// deployment entry under one profile. Every closure deployment shares one
/// assembly, so the linker's same-assembly provider binding holds without any
/// cross-assembly activation_id match.
///
/// Recursion is bounded by the content-addressed deployment reference set:
/// already resolved providers are skipped and a cycle fails closed.
pub fn compose_dependency_closure_assembly<R>(
    entry: &ServiceDeploymentRef,
    resolver: &R,
    profile: &str,
) -> anyhow::Result<RuntimeAssembly>
where
    R: RuntimeAssemblyContentResolver + DeploymentReleasePointerResolver + ?Sized,
{
    let closure = resolve_dependency_closure(entry, resolver, profile)?;
    let mut roots = closure.deployments.keys().cloned().collect::<Vec<_>>();
    roots.sort();
    let deployments = closure.deployments.values().cloned().collect::<Vec<_>>();
    let contracts = closure
        .contracts
        .iter()
        .map(|reference| resolver.resolve_contract(reference))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let packages = closure
        .packages
        .iter()
        .map(|reference| resolver.resolve_package(reference))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let deployment_values = deployments
        .iter()
        .map(|d| d.as_ref().clone())
        .collect::<Vec<_>>();
    let contract_values = contracts
        .iter()
        .map(|c| c.as_ref().clone())
        .collect::<Vec<_>>();
    let package_values = packages
        .iter()
        .map(|p| p.as_ref().clone())
        .collect::<Vec<_>>();
    resolve_runtime_assembly(
        &roots,
        &deployment_values,
        &contract_values,
        &package_values,
    )
    .map_err(anyhow::Error::from)
    .with_context(|| {
        format!(
            "failed to compose dependency closure assembly for entry {entry:?} under profile {profile:?}"
        )
    })
}

/// DFS over the deployment dependency graph. The content-addressed reference
/// set terminates recursion; a reference that is still on the current DFS path
/// is a cycle and fails closed.
fn resolve_dependency_closure<R>(
    entry: &ServiceDeploymentRef,
    resolver: &R,
    profile: &str,
) -> anyhow::Result<DeploymentClosure>
where
    R: RuntimeAssemblyContentResolver + DeploymentReleasePointerResolver + ?Sized,
{
    enum Frame {
        Enter(ServiceDeploymentRef),
        Leave(ServiceDeploymentRef),
    }
    let mut closure = DeploymentClosure::default();
    let mut resolved = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    let mut stack = vec![Frame::Enter(entry.clone())];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Leave(reference) => {
                visiting.remove(&reference);
                resolved.insert(reference);
            }
            Frame::Enter(reference) => {
                if resolved.contains(&reference) {
                    continue;
                }
                if !visiting.insert(reference.clone()) {
                    anyhow::bail!(
                        "dependency cycle detected while composing the lazy-load closure of {reference:?}"
                    );
                }
                let deployment = resolver
                    .resolve_deployment(&reference)
                    .with_context(|| format!("failed to resolve deployment {reference:?}"))?;
                skiff_artifact_identity::validate_service_deployment_ref(&reference, &deployment)
                    .map_err(anyhow::Error::from)
                    .with_context(|| format!("deployment content mismatches ref {reference:?}"))?;
                stack.push(Frame::Leave(reference.clone()));
                closure.contracts.insert(deployment.contract.clone());
                for selector in &deployment.service_selectors {
                    let provider = release_provider(resolver, profile, &selector.contract)?;
                    stack.push(Frame::Enter(provider));
                }
                for package_reference in
                    deployment_package_closure(&reference, &deployment)?.values()
                {
                    let package = resolver
                        .resolve_package(package_reference)
                        .with_context(|| {
                            format!("failed to resolve package {package_reference:?}")
                        })?;
                    closure.packages.insert(package_reference.clone());
                    for requirement in &package.service_requirements {
                        let contract = ServiceContractRef {
                            service_id: requirement
                                .contract_requirement
                                .service_id
                                .clone(),
                            contract_version: requirement
                                .contract_requirement
                                .contract_version
                                .clone(),
                            service_protocol_identity: requirement
                                .contract_requirement
                                .expected_protocol_identity
                                .clone(),
                        };
                        let provider = release_provider(resolver, profile, &contract)?;
                        stack.push(Frame::Enter(provider));
                    }
                }
                closure.deployments.insert(reference.clone(), deployment);
            }
        }
    }
    Ok(closure)
}

fn release_provider<R>(
    resolver: &R,
    profile: &str,
    contract: &ServiceContractRef,
) -> anyhow::Result<ServiceDeploymentRef>
where
    R: RuntimeAssemblyContentResolver + DeploymentReleasePointerResolver + ?Sized,
{
    resolver
        .resolve_release_pointer(profile, &contract.service_id, &contract.contract_version)
        .with_context(|| {
            format!(
                "failed to resolve the release pointer of provider {} {} under profile {profile:?}",
                contract.service_id, contract.contract_version
            )
        })?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no release pointer is set for provider {} {} under profile {profile:?}; \
                 the lazy-load dependency closure cannot be resolved",
                contract.service_id,
                contract.contract_version
            )
        })
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

    /// Resolve + compose + hydrate the whole dependency closure of one entry
    /// deployment under one profile. The closure (entry plus recursively
    /// resolved providers) is synthesized into one image before the canonical
    /// whole-assembly load + link chain runs.
    pub fn load_closure(
        &self,
        entry: &ServiceDeploymentRef,
        profile: &str,
    ) -> anyhow::Result<HydratedRuntimeAssembly>
    where
        R: DeploymentReleasePointerResolver,
    {
        let assembly = compose_dependency_closure_assembly(entry, self.resolver, profile)
            .with_context(|| format!("failed to compose dependency closure assembly {entry:?}"))?;
        RuntimeAssemblyLoader::new(self.resolver)
            .load(Arc::new(assembly))
            .with_context(|| format!("failed to load dependency closure assembly {entry:?}"))
    }
}

#[cfg(test)]
mod tests;
