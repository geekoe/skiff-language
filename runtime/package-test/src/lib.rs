//! Canonical runtime support for package and service tests.
//!
//! A package test is a normal immutable package build plus a separate test-owned
//! package build. Service tests enter through code-free contracts and source-free
//! deployments. Both paths load one typed `RuntimeAssembly`; no synthetic service
//! program or publication aggregate exists here.

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayProtocolSurface,
    IngressSelector, OperationTargetRef, RuntimeAssembly, ServiceDeploymentRef,
};
use skiff_runtime_linker::{link_runtime_assembly, AssemblyLinkedCandidate};
use skiff_runtime_loader::{RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTestEntrypoint {
    pub id: String,
    pub deployment: ServiceDeploymentRef,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

pub struct PackageTestRuntimeBuilder<'a, R: ?Sized> {
    resolver: &'a R,
}

impl<'a, R> PackageTestRuntimeBuilder<'a, R>
where
    R: RuntimeAssemblyContentResolver + ?Sized,
{
    pub fn new(resolver: &'a R) -> Self {
        Self { resolver }
    }

    pub fn load_template(
        &self,
        assembly: impl Into<Arc<RuntimeAssembly>>,
        entrypoints: impl IntoIterator<Item = PackageTestEntrypoint>,
    ) -> anyhow::Result<PackageTestRuntimeTemplate> {
        let hydrated = RuntimeAssemblyLoader::new(self.resolver).load(assembly)?;
        let candidate = Arc::new(link_runtime_assembly(hydrated)?);
        let entrypoints = validate_entrypoints(&candidate, entrypoints)?;
        Ok(PackageTestRuntimeTemplate {
            candidate,
            entrypoints,
        })
    }
}

#[derive(Debug)]
pub struct PackageTestRuntimeTemplate {
    candidate: Arc<AssemblyLinkedCandidate>,
    entrypoints: BTreeMap<String, PackageTestEntrypoint>,
}

impl PackageTestRuntimeTemplate {
    pub fn candidate(&self) -> &Arc<AssemblyLinkedCandidate> {
        &self.candidate
    }

    pub fn entrypoints(&self) -> impl ExactSizeIterator<Item = (&str, &PackageTestEntrypoint)> {
        self.entrypoints
            .iter()
            .map(|(id, entrypoint)| (id.as_str(), entrypoint))
    }

    pub fn load(&self, entrypoint_id: &str) -> anyhow::Result<LoadedPackageTestRuntimeProgram> {
        let entrypoint = self
            .entrypoints
            .get(entrypoint_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package-test entrypoint {entrypoint_id} is not part of the assembly"
                )
            })?;
        Ok(LoadedPackageTestRuntimeProgram {
            candidate: Arc::clone(&self.candidate),
            entrypoint,
        })
    }

    pub fn ingress_entrypoint(
        &self,
        selector: &IngressSelector,
    ) -> anyhow::Result<LoadedPackageTestRuntimeProgram> {
        let binding = self.candidate.ingress(selector).ok_or_else(|| {
            anyhow::anyhow!("canonical test assembly has no ingress selector {selector:?}")
        })?;
        let entrypoint = self
            .entrypoints
            .values()
            .find(|entrypoint| {
                entrypoint.deployment == *binding.owner()
                    && entrypoint.gateway_entry_key == *binding.gateway_entry_key()
                    && entrypoint.gateway_entry_identity == *binding.gateway_entry_identity()
            })
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("ingress selector {selector:?} has no test-owned entrypoint")
            })?;
        Ok(LoadedPackageTestRuntimeProgram {
            candidate: Arc::clone(&self.candidate),
            entrypoint,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LoadedPackageTestRuntimeProgram {
    candidate: Arc<AssemblyLinkedCandidate>,
    entrypoint: PackageTestEntrypoint,
}

impl LoadedPackageTestRuntimeProgram {
    pub fn candidate(&self) -> &Arc<AssemblyLinkedCandidate> {
        &self.candidate
    }

    pub fn entrypoint(&self) -> &PackageTestEntrypoint {
        &self.entrypoint
    }

    pub fn handler_target(&self) -> anyhow::Result<&OperationTargetRef> {
        self.candidate
            .gateway_entry(
                &self.entrypoint.deployment,
                &self.entrypoint.gateway_entry_key,
            )
            .filter(|entry| {
                entry.gateway_entry_identity() == &self.entrypoint.gateway_entry_identity
            })
            .map(|entry| entry.handler().target())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package-test entrypoint {} has no exact linked gateway handler target",
                    self.entrypoint.id
                )
            })
    }
}

fn validate_entrypoints(
    candidate: &AssemblyLinkedCandidate,
    entrypoints: impl IntoIterator<Item = PackageTestEntrypoint>,
) -> anyhow::Result<BTreeMap<String, PackageTestEntrypoint>> {
    let mut validated = BTreeMap::new();
    for entrypoint in entrypoints {
        if entrypoint.id.trim().is_empty() {
            anyhow::bail!("package-test entrypoint id must not be empty");
        }
        candidate
            .activation(&entrypoint.deployment)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package-test entrypoint {} deployment is not in RuntimeAssembly",
                    entrypoint.id
                )
            })?;
        let linked_entry = candidate
            .gateway_entry(&entrypoint.deployment, &entrypoint.gateway_entry_key)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package-test entrypoint {} gateway entry is missing",
                    entrypoint.id
                )
            })?;
        if linked_entry.gateway_entry_identity() != &entrypoint.gateway_entry_identity {
            anyhow::bail!(
                "package-test entrypoint {} gateway entry identity does not match",
                entrypoint.id
            );
        }
        if linked_entry.owner() != &entrypoint.deployment {
            anyhow::bail!(
                "package-test entrypoint {} gateway entry owner does not match its deployment",
                entrypoint.id
            );
        }
        if !matches!(
            linked_entry.protocol_surface().protocol,
            GatewayProtocolSurface::Http(ref surface)
                if surface.dispatch_mode == GatewayDispatchMode::Unary
        ) {
            anyhow::bail!(
                "package-test entrypoint {} must reference an HTTP unary gateway entry",
                entrypoint.id
            );
        }
        if validated
            .insert(entrypoint.id.clone(), entrypoint)
            .is_some()
        {
            anyhow::bail!("duplicate package-test entrypoint id");
        }
    }
    if validated.is_empty() {
        anyhow::bail!("canonical package-test runtime requires at least one entrypoint");
    }
    Ok(validated)
}
