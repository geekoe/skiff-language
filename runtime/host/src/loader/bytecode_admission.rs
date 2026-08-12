use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use skiff_artifact_model::{
    ContractOperationId, GatewayAdapterKind, GatewayAdapterSource, GatewayEntryIdentity,
    GatewayEntryKey, IngressSelector, ServiceDeploymentRef,
};
use skiff_runtime_bytecode_verifier::{verify, VerificationLimits, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{
    DeploymentImage, DeploymentImageCache, DeploymentImageError, DeploymentOwnerIdentity,
};
use skiff_runtime_linked_bytecode::LinkedGatewayCallableRole;
use skiff_runtime_linker::{link_deployment, BytecodeLinkError, LinkLimits};
use skiff_runtime_loader::{
    load_deployment_bytecode_from_store, DeploymentBytecodeContentResolver,
    FilesystemDeploymentBytecodeContentResolver,
};
use skiff_runtime_request::{
    BytecodeRequestTarget, BytecodeRequestTargetError, GatewayAdapterArg as RequestGatewayAdapterArg,
    GatewayAdapterSource as RequestGatewayAdapterSource, HttpAdapter, HttpAdapterCallable,
    HttpAdapterKind,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum BytecodeDeploymentLoadError {
    #[error("deployment bytecode resolver failed: {0}")]
    Resolver(String),
    #[error("deployment bytecode hydration failed: {0}")]
    Hydration(#[from] skiff_runtime_loader::DeploymentBytecodeHydrationError),
    #[error("deployment bytecode link failed: {0}")]
    Link(#[from] BytecodeLinkError),
    #[error("deployment bytecode verification failed: {0}")]
    Verification(#[from] skiff_runtime_bytecode_verifier::VerificationError),
    #[error("deployment image construction failed: {0}")]
    Image(#[from] DeploymentImageError),
}

/// Selects the exact verified code entry used for one admitted request.
#[derive(Debug, Clone)]
pub(crate) enum BytecodeRouteSelector {
    Operation,
    Gateway {
        ingress: IngressSelector,
        gateway_entry_identity: GatewayEntryIdentity,
        role: LinkedGatewayCallableRole,
    },
}

/// Host-owned verified bytecode deployment admission.
///
/// The cache is keyed by the exact deployment owner. A deployment whose
/// implementation package still lacks a bytecode record is explicitly a
/// legacy-assembly fallback rather than a cache miss.
#[derive(Clone)]
pub(crate) struct BytecodeDeploymentRegistry {
    cache: DeploymentImageCache<VerifiedLinkedBytecodeImage, BytecodeDeploymentLoadError>,
}

impl BytecodeDeploymentRegistry {
    pub(crate) fn new() -> Self {
        Self {
            cache: DeploymentImageCache::new(),
        }
    }

    pub(crate) async fn is_loaded_build_id(&self, build_id: &str) -> bool {
        self.cache
            .loaded_snapshot()
            .await
            .iter()
            .any(|image| image.owner().build_id().as_str() == build_id)
    }

    pub(crate) async fn route(
        &self,
        deployment: &ServiceDeploymentRef,
        artifact_root: &Path,
        selector: BytecodeRouteSelector,
    ) -> anyhow::Result<Option<BytecodeRoute>> {
        let Some(image) = self.get_or_load(deployment, artifact_root).await? else {
            return Ok(None);
        };
        Ok(Some(BytecodeRoute::new(
            image,
            deployment,
            artifact_root,
            selector,
        )?))
    }

    pub(crate) async fn get_or_load(
        &self,
        deployment: &ServiceDeploymentRef,
        artifact_root: &Path,
    ) -> anyhow::Result<Option<Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>>> {
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(artifact_root)?;
        let deployment_record = resolver.resolve_deployment(deployment)?;
        let implementation = resolver.resolve_package(&deployment_record.implementation)?;
        if implementation.bytecode.is_none() {
            return Ok(None);
        }

        let owner = DeploymentOwnerIdentity::new(deployment.clone());
        let reference = deployment.clone();
        let artifact_root = artifact_root.to_path_buf();
        let image = self
            .cache
            .get_or_load(owner, move |_attempt_id, _owner| {
                let reference = reference.clone();
                let artifact_root = artifact_root.clone();
                async move {
                    let resolver = FilesystemDeploymentBytecodeContentResolver::open(
                        &artifact_root,
                    )
                    .map_err(|error| BytecodeDeploymentLoadError::Resolver(error.to_string()))?;
                    let hydrated =
                        load_deployment_bytecode_from_store(resolver.store(), &reference)
                            .map_err(BytecodeDeploymentLoadError::Hydration)?;
                    let candidate = link_deployment(&hydrated, &production_link_limits())
                        .map_err(BytecodeDeploymentLoadError::Link)?;
                    let verified = Arc::new(
                        verify(hydrated, candidate, &production_verification_limits())
                            .map_err(BytecodeDeploymentLoadError::Verification)?,
                    );
                    let image = Arc::new(
                        DeploymentImage::try_new(verified)
                            .map_err(BytecodeDeploymentLoadError::Image)?,
                    );
                    Ok(image)
                }
            })
            .await
            .map_err(|error| anyhow::anyhow!("bytecode deployment load failed: {error}"))?;
        Ok(Some(image))
    }
}

impl Default for BytecodeDeploymentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Exact deployment image pinned to an operation or gateway entry.
#[derive(Debug, Clone)]
pub(crate) struct BytecodeRoute {
    image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
    deployment: ServiceDeploymentRef,
    target: BytecodeRouteTarget,
    artifact_root: PathBuf,
    build_id: String,
    service_protocol_identity: String,
}

#[derive(Debug, Clone)]
enum BytecodeRouteTarget {
    Operation(ContractOperationId),
    Gateway {
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
        role: LinkedGatewayCallableRole,
    },
}

impl BytecodeRoute {
    fn new(
        image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        deployment: &ServiceDeploymentRef,
        artifact_root: &Path,
        selector: BytecodeRouteSelector,
    ) -> anyhow::Result<Self> {
        if image.owner().deployment() != deployment {
            anyhow::bail!(
                "bytecode route deployment {} does not match loaded image owner {}",
                deployment.deployment_artifact_identity,
                image.owner().build_id()
            );
        }
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(artifact_root)?;
        let deployment_record = resolver.resolve_deployment(deployment)?;
        let target = match selector {
            BytecodeRouteSelector::Operation => BytecodeRouteTarget::Operation(
                deployment_record
                    .operation_bindings
                    .first()
                    .map(|binding| binding.contract_operation_id.clone())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "bytecode deployment {} has no operation binding",
                            deployment.deployment_artifact_identity
                        )
                    })?,
            ),
            BytecodeRouteSelector::Gateway {
                ingress,
                gateway_entry_identity,
                role,
            } => {
                let mut matching_bindings = deployment_record
                    .ingress
                    .iter()
                    .filter(|binding| binding.selector == ingress);
                let binding = matching_bindings.next().ok_or_else(|| {
                    anyhow::anyhow!(
                        "bytecode deployment {} has no ingress binding for {:?}",
                        deployment.deployment_artifact_identity,
                        ingress
                    )
                })?;
                if matching_bindings.next().is_some() {
                    anyhow::bail!(
                        "bytecode deployment {} has duplicate ingress bindings for {:?}",
                        deployment.deployment_artifact_identity,
                        ingress
                    );
                }
                let gateway_entry_key = binding.gateway_entry_key.clone();
                let entry = deployment_record
                    .gateway_entries
                    .get(&gateway_entry_key)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "bytecode deployment {} ingress references missing gateway entry {}",
                            deployment.deployment_artifact_identity,
                            gateway_entry_key
                        )
                    })?;
                if entry.gateway_entry_identity != gateway_entry_identity {
                    anyhow::bail!(
                        "bytecode deployment {} ingress entry {} does not match routed gateway identity {}",
                        deployment.deployment_artifact_identity,
                        gateway_entry_key,
                        gateway_entry_identity
                    );
                }
                BytecodeRouteTarget::Gateway {
                    gateway_entry_key,
                    gateway_entry_identity,
                    role,
                }
            }
        };
        let contract = resolver.resolve_contract(&deployment_record.contract)?;
        Ok(Self {
            image,
            deployment: deployment.clone(),
            target,
            artifact_root: artifact_root.to_path_buf(),
            build_id: deployment_record
                .implementation
                .package_build_id
                .as_str()
                .to_string(),
            service_protocol_identity: contract.service_protocol_identity.as_str().to_string(),
        })
    }

    pub(crate) fn deployment(&self) -> &ServiceDeploymentRef {
        &self.deployment
    }

    pub(crate) fn target_label(&self) -> String {
        match &self.target {
            BytecodeRouteTarget::Operation(operation_id) => operation_id.as_str().to_string(),
            BytecodeRouteTarget::Gateway {
                gateway_entry_identity,
                ..
            } => gateway_entry_identity.as_str().to_string(),
        }
    }

    pub(crate) fn build_id(&self) -> &str {
        &self.build_id
    }

    pub(crate) fn service_protocol_identity(&self) -> &str {
        &self.service_protocol_identity
    }

    pub(crate) fn http_adapter(&self) -> anyhow::Result<HttpAdapter> {
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(&self.artifact_root)?;
        let deployment_record = resolver.resolve_deployment(&self.deployment)?;
        let (key, entry) = match &self.target {
            BytecodeRouteTarget::Gateway {
                gateway_entry_key,
                ..
            } => deployment_record
                .gateway_entries
                .get(gateway_entry_key)
                .map(|entry| (gateway_entry_key.clone(), entry))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "bytecode deployment {} has no gateway entry {}",
                        self.deployment.deployment_artifact_identity,
                        gateway_entry_key
                    )
                })?,
            BytecodeRouteTarget::Operation(_) => {
                anyhow::bail!("HTTP adapter requires a gateway route")
            }
        };
        let kind = match entry.adapter_plan.kind {
            GatewayAdapterKind::RawHttp => HttpAdapterKind::RawHttp,
            GatewayAdapterKind::TypedJson => HttpAdapterKind::TypedJson,
            _ => {
                anyhow::bail!(
                    "gateway entry {key} has no HTTP adapter plan"
                )
            }
        };
        let adapter_args = entry
            .adapter_plan
            .args
            .iter()
            .map(|arg| {
                let source = match arg.source {
                    GatewayAdapterSource::HttpRequest => RequestGatewayAdapterSource::HttpRequest,
                    GatewayAdapterSource::HttpBody => RequestGatewayAdapterSource::HttpBody,
                    GatewayAdapterSource::HttpContext => RequestGatewayAdapterSource::HttpContext,
                    _ => {
                        anyhow::bail!(
                            "gateway entry {key} adapter arg {} is not an HTTP source",
                            arg.param
                        )
                    }
                };
                Ok(RequestGatewayAdapterArg {
                    param: arg.param.clone(),
                    source,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(HttpAdapter {
            kind,
            handler: HttpAdapterCallable::PackageFunction {
                package_id: self.deployment.service_id.clone(),
                symbol_path: key.as_str().to_string(),
            },
            guard: None,
            pre: None,
            adapter_args,
        })
    }

    pub(crate) fn request_target(&self) -> anyhow::Result<BytecodeRequestTarget> {
        let program = Arc::clone(self.image.program());
        let image = Arc::clone(&self.image);
        match &self.target {
            BytecodeRouteTarget::Operation(operation_id) => {
                let entry = program
                    .operation_entry(operation_id)
                    .map_err(|error| anyhow::anyhow!("bytecode operation lookup failed: {error}"))?;
                BytecodeRequestTarget::try_new(image, entry, operation_id.clone())
                    .map_err(bytecode_target_error)
            }
            BytecodeRouteTarget::Gateway {
                gateway_entry_key,
                role,
                ..
            } => {
                let entry = program
                    .gateway_entry(gateway_entry_key, *role)
                    .map_err(|error| anyhow::anyhow!("bytecode gateway lookup failed: {error}"))?;
                BytecodeRequestTarget::try_new_gateway(
                    image,
                    entry,
                    gateway_entry_key.clone(),
                    *role,
                )
                .map_err(bytecode_target_error)
            }
        }
    }
}

fn bytecode_target_error(error: BytecodeRequestTargetError) -> anyhow::Error {
    anyhow::anyhow!("bytecode request target failed closed: {error}")
}

fn production_link_limits() -> LinkLimits {
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

fn production_verification_limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: 100_000,
        max_total_instructions: 100_000_000,
        max_instructions_per_function: 1_000_000,
        max_frame_slots_per_function: 65_536,
        max_operand_depth: 65_536,
        max_control_flow_edges_per_function: 1_000_000,
        max_exception_regions_per_function: 1_000_000,
        max_switch_targets_per_function: 65_536,
        max_statement_events_per_pc: 100_000,
        max_statement_events_per_function: 1_000_000,
        max_total_statement_events: 10_000_000,
        max_source_map_entries_per_function: 1_000_000,
        max_image_table_entries: 1_000_000,
        max_arity: 256,
        max_callback_captures_per_callback: 4_096,
        max_type_nesting_depth: 64,
        max_value_lifecycle_nodes: 1_000_000,
        max_value_lifecycle_canonical_bytes: 64 * 1024 * 1024,
        max_constant_graph_edges: 1_000_000,
    }
}
