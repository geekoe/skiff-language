//! Canonical runtime support for package and service tests.
//!
//! A package test is a normal immutable package build plus a separate
//! test-owned package build. Service tests enter through code-free contracts
//! and source-free deployments, and this crate hydrates and links one exact
//! deployment bytecode closure for the test entrypoint.

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayProtocolSurface,
    ServiceDeploymentRef,
};
use skiff_runtime_linked_bytecode::{
    LinkedBytecodeCandidate, LinkedGatewayCallable, LinkedGatewayEntry,
};
use skiff_runtime_linker::{link_deployment, LinkLimits};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader, HydratedDeploymentBytecode,
};

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
    R: DeploymentBytecodeContentResolver + ?Sized,
{
    pub fn new(resolver: &'a R) -> Self {
        Self { resolver }
    }

    pub fn load(
        &self,
        deployment: &ServiceDeploymentRef,
        entrypoints: impl IntoIterator<Item = PackageTestEntrypoint>,
    ) -> anyhow::Result<PackageTestRuntimeTemplate> {
        let hydrated = Arc::new(DeploymentBytecodeLoader::new(self.resolver).load(deployment)?);
        let candidate = Arc::new(link_deployment(&hydrated, &package_test_link_limits())?);
        let entrypoints = validate_entrypoints(&hydrated, &candidate, entrypoints)?;
        Ok(PackageTestRuntimeTemplate {
            hydrated,
            candidate,
            entrypoints,
        })
    }
}

#[derive(Debug)]
pub struct PackageTestRuntimeTemplate {
    hydrated: Arc<HydratedDeploymentBytecode>,
    candidate: Arc<LinkedBytecodeCandidate>,
    entrypoints: BTreeMap<String, PackageTestEntrypoint>,
}

impl PackageTestRuntimeTemplate {
    pub fn hydrated(&self) -> &Arc<HydratedDeploymentBytecode> {
        &self.hydrated
    }

    pub fn candidate(&self) -> &Arc<LinkedBytecodeCandidate> {
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
                anyhow::anyhow!("package-test entrypoint {entrypoint_id} is not part of the deployment")
            })?;
        Ok(LoadedPackageTestRuntimeProgram {
            hydrated: Arc::clone(&self.hydrated),
            candidate: Arc::clone(&self.candidate),
            entrypoint,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LoadedPackageTestRuntimeProgram {
    hydrated: Arc<HydratedDeploymentBytecode>,
    candidate: Arc<LinkedBytecodeCandidate>,
    entrypoint: PackageTestEntrypoint,
}

impl LoadedPackageTestRuntimeProgram {
    pub fn hydrated(&self) -> &Arc<HydratedDeploymentBytecode> {
        &self.hydrated
    }

    pub fn candidate(&self) -> &Arc<LinkedBytecodeCandidate> {
        &self.candidate
    }

    pub fn entrypoint(&self) -> &PackageTestEntrypoint {
        &self.entrypoint
    }

    pub fn handler(&self) -> Option<&LinkedGatewayCallable> {
        linked_entry(&self.candidate, &self.entrypoint).and_then(LinkedGatewayEntry::handler)
    }
}

fn validate_entrypoints(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    entrypoints: impl IntoIterator<Item = PackageTestEntrypoint>,
) -> anyhow::Result<BTreeMap<String, PackageTestEntrypoint>> {
    let mut validated = BTreeMap::new();
    for entrypoint in entrypoints {
        if entrypoint.id.trim().is_empty() {
            anyhow::bail!("package-test entrypoint id must not be empty");
        }
        if entrypoint.deployment != *hydrated.reference() {
            anyhow::bail!(
                "package-test entrypoint {} deployment is not part of the package-test deployment",
                entrypoint.id
            );
        }
        let declared = hydrated
            .deployment()
            .gateway_entries
            .get(&entrypoint.gateway_entry_key)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "package-test entrypoint {} gateway entry is missing from the deployment",
                    entrypoint.id
                )
            })?;
        if declared.gateway_entry_identity != entrypoint.gateway_entry_identity {
            anyhow::bail!(
                "package-test entrypoint {} gateway entry identity does not match",
                entrypoint.id
            );
        }
        let linked_entry = linked_entry(candidate, &entrypoint).ok_or_else(|| {
            anyhow::anyhow!(
                "package-test entrypoint {} gateway entry is missing from the linked candidate",
                entrypoint.id
            )
        })?;
        if linked_entry.handler().is_none() {
            anyhow::bail!(
                "package-test entrypoint {} has no linked gateway handler",
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

fn linked_entry<'a>(
    candidate: &'a LinkedBytecodeCandidate,
    entrypoint: &PackageTestEntrypoint,
) -> Option<&'a LinkedGatewayEntry> {
    candidate.gateway_entries().iter().find(|entry| {
        entry.gateway_entry_key() == &entrypoint.gateway_entry_key
            && entry.gateway_entry_identity() == &entrypoint.gateway_entry_identity
    })
}

fn package_test_link_limits() -> LinkLimits {
    LinkLimits {
        max_packages: 256,
        max_root_specializations: 100_000,
        max_specializations: 1_000_000,
        max_code_words_per_function: 1_000_000,
        max_total_code_words: 100_000_000,
        max_relocations_per_function: 100_000,
        max_total_relocations: 10_000_000,
        max_image_table_entries: 1_000_000,
        max_total_image_table_entries: 10_000_000,
        max_total_function_table_entries: 10_000_000,
        max_type_nesting_depth: 64,
        max_expanded_type_nodes: 1_000_000,
        max_expanded_type_bytes: 64 * 1024 * 1024,
        max_constant_graph_nodes: 1_000_000,
        max_constant_graph_edges: 1_000_000,
    }
}
