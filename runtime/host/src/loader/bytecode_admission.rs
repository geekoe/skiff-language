use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use skiff_artifact_model::{
    ContractOperationId, GatewayAdapterKind, GatewayAdapterSource, GatewayEntryIdentity,
    GatewayEntryKey, IngressSelector, ServiceDeploymentRef,
};
use skiff_runtime_bytecode_verifier::VerificationLimits;
use skiff_runtime_deployment_image::{
    DeploymentImageCache, DeploymentLoadError, DeploymentLoadFailureReason, DeploymentOwnerIdentity,
};
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionEntry, DeploymentExecutionImage,
    DeploymentExecutionImageError, DeploymentExecutionLimits, LinkLimits,
};
use skiff_runtime_loader::{
    load_deployment_bytecode_from_store, DeploymentBytecodeContentResolver,
    FilesystemDeploymentBytecodeContentResolver,
};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEvent, BytecodeExecutionObserver, BytecodeGatewayCallableRole,
    BytecodeRouteEntrySelector, DeploymentImageSelected, RouteEntryPinned,
};
use skiff_runtime_request::{
    GatewayAdapterArg as RequestGatewayAdapterArg,
    GatewayAdapterSource as RequestGatewayAdapterSource, HttpAdapter, HttpAdapterCallable,
    HttpAdapterKind,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum BytecodeDeploymentLoadError {
    #[error("deployment bytecode resolver failed: {0}")]
    Resolver(String),
    #[error("deployment bytecode hydration failed: {0}")]
    Hydration(#[from] skiff_runtime_loader::DeploymentBytecodeHydrationError),
    #[error("deployment execution image construction failed: {0}")]
    ExecutionImage(#[from] DeploymentExecutionImageError),
    #[error("deployment implementation package has no bytecode record")]
    LegacyAssembly,
}

/// Selects the exact verified code entry used for one admitted request.
#[derive(Debug, Clone)]
pub(crate) enum BytecodeRouteSelector {
    Operation {
        contract_operation_id: ContractOperationId,
    },
    Gateway {
        ingress: IngressSelector,
        gateway_entry_identity: GatewayEntryIdentity,
    },
}

/// Host-owned verified bytecode deployment admission.
///
/// The cache is keyed by the exact deployment owner. A deployment whose
/// implementation package still lacks a bytecode record is explicitly a
/// legacy-assembly fallback rather than a cache miss.
#[derive(Clone)]
pub(crate) struct BytecodeDeploymentRegistry {
    cache: DeploymentImageCache<DeploymentExecutionImage, BytecodeDeploymentLoadError>,
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
        observer: &BytecodeExecutionObserver,
    ) -> anyhow::Result<Option<BytecodeRoute>> {
        let Some(image) = self.get_or_load(deployment, artifact_root).await? else {
            return Ok(None);
        };
        if image.owner().deployment() != deployment {
            anyhow::bail!(
                "bytecode route deployment {} does not match loaded image owner {}",
                deployment.deployment_artifact_identity,
                image.owner().build_id()
            );
        }
        let selected = BytecodeExecutionEvent::DeploymentImageSelected(DeploymentImageSelected {
            deployment: image.owner().deployment().clone(),
            deployment_build_id: image.owner().build_id().clone(),
        });
        Ok(Some(BytecodeRoute::new(
            image,
            deployment,
            selector,
            observer.clone(),
            selected,
        )?))
    }

    pub(crate) async fn get_or_load(
        &self,
        deployment: &ServiceDeploymentRef,
        artifact_root: &Path,
    ) -> anyhow::Result<Option<Arc<DeploymentExecutionImage>>> {
        let owner = DeploymentOwnerIdentity::new(deployment.clone());
        let reference = deployment.clone();
        let artifact_root = artifact_root.to_path_buf();
        let result = self
            .cache
            .get_or_load(owner, move |_attempt_id, _owner| {
                let reference = reference.clone();
                let artifact_root = artifact_root.clone();
                async move {
                    let resolver = FilesystemDeploymentBytecodeContentResolver::open(
                        &artifact_root,
                    )
                    .map_err(|error| BytecodeDeploymentLoadError::Resolver(error.to_string()))?;
                    let deployment_record =
                        resolver.resolve_deployment(&reference).map_err(|error| {
                            BytecodeDeploymentLoadError::Resolver(error.to_string())
                        })?;
                    let implementation = resolver
                        .resolve_package(&deployment_record.implementation)
                        .map_err(|error| {
                            BytecodeDeploymentLoadError::Resolver(error.to_string())
                        })?;
                    if implementation.bytecode.is_none() {
                        return Err(BytecodeDeploymentLoadError::LegacyAssembly);
                    }
                    let hydrated =
                        load_deployment_bytecode_from_store(resolver.store(), &reference)
                            .map_err(BytecodeDeploymentLoadError::Hydration)?;
                    let limits = DeploymentExecutionLimits::new(
                        production_link_limits(),
                        production_verification_limits(),
                    );
                    Ok(Arc::new(link_deployment_execution_image(
                        hydrated, &limits,
                    )?))
                }
            })
            .await;
        match result {
            Ok(image) => Ok(Some(image)),
            Err(error) if is_legacy_assembly(&error) => Ok(None),
            Err(error) => Err(anyhow::anyhow!("bytecode deployment load failed: {error}")),
        }
    }
}

impl Default for BytecodeDeploymentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn is_legacy_assembly(error: &DeploymentLoadError<BytecodeDeploymentLoadError>) -> bool {
    matches!(
        error,
        DeploymentLoadError::Attempt(failure)
            if matches!(
                failure.reason(),
                DeploymentLoadFailureReason::Provider { error }
                    if matches!(error.as_ref(), BytecodeDeploymentLoadError::LegacyAssembly)
            )
    )
}

/// Exact deployment image pinned to an operation or gateway entry.
#[derive(Debug)]
pub(crate) struct BytecodeRoute {
    image: Arc<DeploymentExecutionImage>,
    target: BytecodeRouteTarget,
    observer: BytecodeExecutionObserver,
    admission_observations: Mutex<AdmissionObservations>,
}

#[derive(Debug)]
struct AdmissionObservations {
    deployment: Option<BytecodeExecutionEvent>,
    entry: Option<BytecodeExecutionEvent>,
}

#[derive(Debug, Clone)]
enum BytecodeRouteTarget {
    Operation(ContractOperationId),
    Gateway {
        ingress: IngressSelector,
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
        adapter_plan: skiff_artifact_model::GatewayAdapterPlan,
    },
}

impl BytecodeRoute {
    fn new(
        image: Arc<DeploymentExecutionImage>,
        deployment: &ServiceDeploymentRef,
        selector: BytecodeRouteSelector,
        observer: BytecodeExecutionObserver,
        selected: BytecodeExecutionEvent,
    ) -> anyhow::Result<Self> {
        if image.owner().deployment() != deployment {
            anyhow::bail!(
                "bytecode route deployment {} does not match loaded image owner {}",
                deployment.deployment_artifact_identity,
                image.owner().build_id()
            );
        }
        let target = match selector {
            BytecodeRouteSelector::Operation {
                contract_operation_id,
            } => {
                image
                    .operation_entry(&contract_operation_id)
                    .map_err(|error| {
                        anyhow::anyhow!("bytecode operation lookup failed: {error}")
                    })?;
                BytecodeRouteTarget::Operation(contract_operation_id)
            }
            BytecodeRouteSelector::Gateway {
                ingress,
                gateway_entry_identity,
            } => {
                let mut matching_bindings = image
                    .ingress_bindings()
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
                image
                    .http_gateway_entry(&ingress, &gateway_entry_identity)
                    .map_err(|error| anyhow::anyhow!("bytecode gateway lookup failed: {error}"))?;
                let adapter_plan = image
                    .gateway_adapter_plan(&gateway_entry_key)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "bytecode deployment {} gateway entry {} has no verified adapter plan",
                            deployment.deployment_artifact_identity,
                            gateway_entry_key
                        )
                    })?;
                BytecodeRouteTarget::Gateway {
                    ingress,
                    gateway_entry_key,
                    gateway_entry_identity,
                    adapter_plan,
                }
            }
        };
        Ok(Self {
            image,
            target,
            observer,
            admission_observations: Mutex::new(AdmissionObservations {
                deployment: Some(selected),
                entry: None,
            }),
        })
    }

    pub(crate) fn deployment(&self) -> &ServiceDeploymentRef {
        self.image.owner().deployment()
    }

    pub(crate) fn owner(&self) -> &DeploymentOwnerIdentity {
        self.image.owner()
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
        self.image.owner().build_id().as_str()
    }

    pub(crate) fn service_protocol_identity(&self) -> &str {
        self.image.service_protocol_identity().as_str()
    }

    pub(crate) fn http_adapter(&self) -> anyhow::Result<HttpAdapter> {
        let (key, adapter_plan) = match &self.target {
            BytecodeRouteTarget::Gateway {
                gateway_entry_key,
                adapter_plan,
                ..
            } => (gateway_entry_key, adapter_plan),
            BytecodeRouteTarget::Operation(_) => {
                anyhow::bail!("HTTP adapter requires a gateway route")
            }
        };
        let kind = match adapter_plan.kind {
            GatewayAdapterKind::RawHttp => HttpAdapterKind::RawHttp,
            GatewayAdapterKind::TypedJson => HttpAdapterKind::TypedJson,
            _ => anyhow::bail!("gateway entry {key} has no HTTP adapter plan"),
        };
        let adapter_args = adapter_plan
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
                package_id: self.deployment().service_id.clone(),
                symbol_path: key.as_str().to_string(),
            },
            guard: None,
            pre: None,
            adapter_args,
        })
    }

    pub(crate) fn execution_entry(&self) -> anyhow::Result<DeploymentExecutionEntry> {
        let target = match &self.target {
            BytecodeRouteTarget::Operation(operation_id) => self
                .image
                .operation_entry(operation_id)
                .map_err(|error| anyhow::anyhow!("bytecode operation lookup failed: {error}")),
            BytecodeRouteTarget::Gateway {
                ingress,
                gateway_entry_identity,
                ..
            } => self
                .image
                .http_gateway_entry(ingress, gateway_entry_identity)
                .map_err(|error| anyhow::anyhow!("bytecode gateway lookup failed: {error}")),
        }?;
        let (selector, gateway_key, gateway_identity, callable_role) = match &self.target {
            BytecodeRouteTarget::Operation(operation_id) => (
                BytecodeRouteEntrySelector::Operation(operation_id.clone()),
                None,
                None,
                None,
            ),
            BytecodeRouteTarget::Gateway {
                ingress,
                gateway_entry_key,
                gateway_entry_identity,
                ..
            } => (
                BytecodeRouteEntrySelector::Gateway(ingress.clone()),
                Some(gateway_entry_key.clone()),
                Some(gateway_entry_identity.clone()),
                Some(BytecodeGatewayCallableRole::Handler),
            ),
        };
        let entry = BytecodeExecutionEvent::RouteEntryPinned(RouteEntryPinned {
            image_owner: self.image.owner().deployment().clone(),
            selector,
            gateway_key,
            gateway_identity,
            callable_role,
            verified_function_index: target.function().get(),
        });
        self.admission_observations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry = Some(entry);
        Ok(target)
    }

    /// Publishes owner-minted admission facts only after the supervisor row is
    /// active. A route dropped on any pre-admission failure publishes nothing.
    pub(crate) fn publish_admission_observations(&self) {
        let observations = {
            let mut staged = self
                .admission_observations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let deployment = staged
                .deployment
                .take()
                .expect("admitted route retains its deployment observation");
            let entry = staged
                .entry
                .take()
                .expect("admitted route retains its entry observation");
            [deployment, entry]
        };
        for observation in observations {
            self.observer.observe(observation);
        }
    }
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
