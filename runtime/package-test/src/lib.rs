//! Canonical runtime support for package and service tests.
//!
//! A package test is a normal immutable package build plus a separate
//! test-owned package build. Service tests enter through code-free contracts
//! and source-free deployments, and this crate hydrates and links one exact
//! deployment bytecode closure for the test entrypoint.

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayProtocolSurface,
    IngressSelector, ServiceDeploymentRef,
};
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionEntry, DeploymentExecutionImage, LinkLimits,
};
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
        let hydrated = DeploymentBytecodeLoader::new(self.resolver).load(deployment)?;
        let (entrypoints, ingress_by_id) = validate_entrypoints(&hydrated, entrypoints)?;
        let limits = package_test_link_limits();
        let image = Arc::new(link_deployment_execution_image(hydrated, &limits)?);
        for (id, entrypoint) in &entrypoints {
            image.http_gateway_entry(
                ingress_by_id
                    .get(id)
                    .expect("validated package-test ingress is retained"),
                &entrypoint.gateway_entry_identity,
            )?;
        }
        Ok(PackageTestRuntimeTemplate {
            image,
            entrypoints,
            ingress_by_id,
        })
    }
}

#[derive(Debug)]
pub struct PackageTestRuntimeTemplate {
    image: Arc<DeploymentExecutionImage>,
    entrypoints: BTreeMap<String, PackageTestEntrypoint>,
    ingress_by_id: BTreeMap<String, IngressSelector>,
}

impl PackageTestRuntimeTemplate {
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
                    "package-test entrypoint {entrypoint_id} is not part of the deployment"
                )
            })?;
        Ok(LoadedPackageTestRuntimeProgram {
            entry: self.image.http_gateway_entry(
                self.ingress_by_id
                    .get(entrypoint_id)
                    .expect("validated package-test ingress is retained"),
                &entrypoint.gateway_entry_identity,
            )?,
            entrypoint,
        })
    }
}

#[derive(Debug)]
pub struct LoadedPackageTestRuntimeProgram {
    entry: DeploymentExecutionEntry,
    entrypoint: PackageTestEntrypoint,
}

impl LoadedPackageTestRuntimeProgram {
    pub fn entry(&self) -> &DeploymentExecutionEntry {
        &self.entry
    }

    pub fn entrypoint(&self) -> &PackageTestEntrypoint {
        &self.entrypoint
    }
}

fn validate_entrypoints(
    hydrated: &HydratedDeploymentBytecode,
    entrypoints: impl IntoIterator<Item = PackageTestEntrypoint>,
) -> anyhow::Result<(
    BTreeMap<String, PackageTestEntrypoint>,
    BTreeMap<String, IngressSelector>,
)> {
    let mut validated = BTreeMap::new();
    let mut ingress_by_id = BTreeMap::new();
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
        if !matches!(
            declared.protocol_surface.protocol,
            GatewayProtocolSurface::Http(ref surface)
                if surface.dispatch_mode == GatewayDispatchMode::Unary
        ) {
            anyhow::bail!(
                "package-test entrypoint {} must reference an HTTP unary gateway entry",
                entrypoint.id
            );
        }
        let mut ingress = hydrated
            .deployment()
            .ingress
            .iter()
            .filter(|binding| binding.gateway_entry_key == entrypoint.gateway_entry_key);
        let selector = ingress.next().ok_or_else(|| {
            anyhow::anyhow!(
                "package-test entrypoint {} has no exact ingress binding",
                entrypoint.id
            )
        })?;
        if ingress.next().is_some() {
            anyhow::bail!(
                "package-test entrypoint {} has duplicate ingress bindings",
                entrypoint.id
            );
        }
        ingress_by_id.insert(entrypoint.id.clone(), selector.selector.clone());
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
    Ok((validated, ingress_by_id))
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
