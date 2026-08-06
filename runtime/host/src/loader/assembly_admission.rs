use std::sync::{Arc, RwLock};

use anyhow::Context;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, AssemblyActivationServiceDb,
    AssemblyIdentity, BoundaryOperationDescriptor, ContractOperationId, GatewayAdapterKind,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayProtocolSurface, GatewayWebSocketRpcProfile, IngressProtocol, IngressSelector,
    RuntimeAssembly, RuntimeAssemblyRef, RuntimeConfigSnapshotRef, ServiceContractRef,
    ServiceDeploymentRef, ServiceIngressKey, WebSocketEntryId,
};
use skiff_runtime_activation::{ActivationContext, RequestActivationContext};
use skiff_runtime_eval::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};
use skiff_runtime_linker::{
    link_runtime_assembly, AssemblyLinkedCandidate, LinkedActivationTemplate, LinkedGatewayEntry,
};
use skiff_runtime_loader::{
    RuntimeAssemblyContentResolver, RuntimeAssemblyLoader, RuntimeAssemblyRecordResolver,
    ServiceContractStore,
};
use skiff_runtime_request::{
    cancellation::CancellationToken, RuntimeAssemblyHttpGatewayTarget,
    RuntimeAssemblyWebSocketConnectTarget, RuntimeAssemblyWebSocketJsonRpcPhysicalRoute,
    RuntimeAssemblyWebSocketJsonRpcTarget,
};
use skiff_runtime_transport::actor_owner::ActorOwnerRouteAuthorityFrameHeader;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::active_assembly_context::{admitted_websocket_entry, ActiveAssemblyContextSet};
use crate::capability_context::DbProviderSource;
use crate::host::RuntimeHost;

mod candidate;
mod loaded_deployments;
mod provisioning;
mod recovery;

use loaded_deployments::LoadedDeploymentRegistry;

/// Host-owned immutable assembly published after the complete candidate passes admission.
#[derive(Debug)]
pub(crate) struct ActiveAssembly {
    generation: u64,
    admitted_at: OffsetDateTime,
    candidate: Arc<AssemblyLinkedCandidate>,
    contexts: Arc<ActiveAssemblyContextSet>,
}

#[derive(Debug)]
struct PreparedAssembly {
    generation: u64,
    config_snapshot: RuntimeConfigSnapshotRef,
    candidate: Arc<AssemblyLinkedCandidate>,
    contexts: Arc<ActiveAssemblyContextSet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssemblyTransition {
    profile: String,
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
}

#[derive(Debug)]
struct StagedAssembly {
    transition: AssemblyTransition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedAssembly {
    profile: String,
    generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
}

/// One request-entry route pinned to the exact active generation used for lookup.
#[derive(Debug, Clone)]
pub(crate) struct ActiveAssemblyRoute {
    active: Arc<ActiveAssembly>,
    ingress_key: ServiceIngressKey,
    entry: Arc<LinkedGatewayEntry>,
    activation: Arc<ActivationContext>,
}

/// Immutable committed-assembly snapshot for one Actor owner execution.
#[derive(Debug, Clone)]
pub(crate) struct ActiveActorExecutionRoute {
    active: Arc<ActiveAssembly>,
    activation: Arc<ActivationContext>,
}

impl ActiveActorExecutionRoute {
    pub(crate) fn active_assembly(&self) -> &Arc<ActiveAssembly> {
        &self.active
    }

    pub(crate) fn activation(&self) -> &Arc<ActivationContext> {
        &self.activation
    }

    pub(crate) fn execution_image(
        &self,
    ) -> &Arc<skiff_runtime_linked_program::AssemblyExecutionImage> {
        self.active.candidate.execution_image()
    }

    pub(crate) fn context_set(&self) -> &Arc<ActiveAssemblyContextSet> {
        &self.active.contexts
    }

    pub(crate) fn service_protocol_identity(
        &self,
    ) -> anyhow::Result<&skiff_artifact_model::ServiceProtocolIdentity> {
        self.active
            .contexts
            .contract_for_deployment(&self.activation.identity().deployment)
            .map(|contract| &contract.service_protocol_identity)
            .ok_or_else(|| anyhow::anyhow!("Actor activation has no exact service contract"))
    }

    pub(crate) fn db_source(
        &self,
    ) -> anyhow::Result<skiff_runtime_capability_context::DbCapabilitySource> {
        self.active
            .contexts
            .db_source(self.activation.activation_id())
            .ok_or_else(|| anyhow::anyhow!("Actor activation has no DB capability source"))
    }

    pub(crate) fn config_views(
        &self,
    ) -> anyhow::Result<Arc<super::config_snapshot::ActivationConfigViews>> {
        self.active
            .contexts
            .config_views(&self.activation.identity().deployment)
            .ok_or_else(|| anyhow::anyhow!("Actor activation has no scoped config views"))
    }
}

impl ActiveAssemblyRoute {
    pub(crate) fn assembly_identity(&self) -> &AssemblyIdentity {
        self.active.identity()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.active.generation()
    }

    pub(crate) fn activation(&self) -> &Arc<ActivationContext> {
        &self.activation
    }

    pub(crate) fn selector(&self) -> &IngressSelector {
        &self.ingress_key.selector
    }

    pub(crate) fn deployment(&self) -> &ServiceDeploymentRef {
        &self.ingress_key.deployment
    }

    pub(crate) fn gateway_entry_key(&self) -> &GatewayEntryKey {
        self.entry.gateway_entry_key()
    }

    pub(crate) fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        self.entry.gateway_entry_identity()
    }

    pub(crate) fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        self.entry.protocol_surface()
    }

    pub(crate) fn service_protocol_identity(
        &self,
    ) -> &skiff_artifact_model::ServiceProtocolIdentity {
        &self
            .active
            .candidate
            .activation(self.entry.owner())
            .expect("admitted route owner activation remains pinned")
            .deployment()
            .contract
            .service_protocol_identity
    }

    pub(crate) fn request_target(&self) -> anyhow::Result<RuntimeAssemblyHttpGatewayTarget> {
        let request_activation = RequestActivationContext::begin(Arc::clone(&self.activation))?;
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::clone(&self.active.contexts) as _;
        let eval = RuntimeAssemblyEvalTarget::new(
            Arc::clone(self.execution_image()),
            request_activation,
            resolver,
        )?;
        Ok(RuntimeAssemblyHttpGatewayTarget::new(
            eval,
            Arc::clone(&self.entry),
        )?)
    }

    pub(crate) fn websocket_connect_target(
        &self,
    ) -> anyhow::Result<RuntimeAssemblyWebSocketConnectTarget> {
        let request_activation = RequestActivationContext::begin(Arc::clone(&self.activation))?;
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::clone(&self.active.contexts) as _;
        let eval = RuntimeAssemblyEvalTarget::new(
            Arc::clone(self.execution_image()),
            request_activation,
            resolver,
        )?;
        Ok(RuntimeAssemblyWebSocketConnectTarget::new(
            eval,
            self.ingress_key.selector.clone(),
            Arc::clone(&self.entry),
        )?)
    }

    pub(crate) fn has_websocket_jsonrpc_methods(&self) -> anyhow::Result<bool> {
        Ok(self.admitted_physical_websocket_entry()?.has_methods())
    }

    pub(crate) fn websocket_jsonrpc_method_route(
        &self,
        path: &str,
        method: &str,
        gateway_entry_identity: &GatewayEntryIdentity,
        profile: GatewayWebSocketRpcProfile,
        websocket_entry_id: &WebSocketEntryId,
    ) -> anyhow::Result<Self> {
        let admitted = self.admitted_physical_websocket_entry()?;
        if &admitted.websocket_entry_id != websocket_entry_id {
            anyhow::bail!(
                "WebSocket JSON-RPC physical entry identity does not match the generation pin"
            );
        }
        let sibling = admitted.method(method).ok_or_else(|| {
            anyhow::anyhow!("WebSocket generation pin has no admitted JSON-RPC method {method:?}")
        })?;
        if sibling.selector.path != path
            || sibling.selector.protocol != IngressProtocol::WebSocket
            || sibling.selector.method.as_deref() != Some(method)
            || &sibling.gateway_entry_identity != gateway_entry_identity
            || sibling.profile != profile
            || sibling.linked_entry.owner() != self.entry.owner()
            || sibling.linked_entry.gateway_entry_key() != &sibling.gateway_entry_key
            || sibling.linked_entry.gateway_entry_identity() != gateway_entry_identity
        {
            anyhow::bail!(
                "WebSocket JSON-RPC request does not exactly match its pinned sibling method"
            );
        }
        Ok(Self {
            active: Arc::clone(&self.active),
            ingress_key: ServiceIngressKey {
                deployment: sibling.linked_entry.owner().clone(),
                selector: sibling.selector.clone(),
            },
            entry: Arc::clone(&sibling.linked_entry),
            activation: Arc::clone(&self.activation),
        })
    }

    pub(crate) fn websocket_jsonrpc_target(
        &self,
        physical_route: &ActiveAssemblyRoute,
    ) -> anyhow::Result<RuntimeAssemblyWebSocketJsonRpcTarget> {
        if !Arc::ptr_eq(&self.active, &physical_route.active)
            || !Arc::ptr_eq(&self.activation, &physical_route.activation)
            || self.entry.owner() != physical_route.entry.owner()
        {
            anyhow::bail!(
                "WebSocket JSON-RPC method route does not share the pinned physical generation"
            );
        }
        let admitted = physical_route.admitted_physical_websocket_entry()?;
        let request_activation = RequestActivationContext::begin(Arc::clone(&self.activation))?;
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::clone(&self.active.contexts) as _;
        let eval = RuntimeAssemblyEvalTarget::new(
            Arc::clone(self.execution_image()),
            request_activation,
            resolver,
        )?;
        Ok(RuntimeAssemblyWebSocketJsonRpcTarget::new(
            eval,
            self.ingress_key.selector.clone(),
            RuntimeAssemblyWebSocketJsonRpcPhysicalRoute::new(
                admitted.selector.clone(),
                admitted.gateway_entry_key.clone(),
                admitted.gateway_entry_identity.clone(),
                admitted.websocket_entry_id.clone(),
            ),
            Arc::clone(&self.entry),
        )?)
    }

    fn admitted_physical_websocket_entry(
        &self,
    ) -> anyhow::Result<&super::active_assembly_context::AdmittedWebSocketEntry> {
        let admitted = self
            .active
            .contexts
            .websocket_entry(self.entry.owner())
            .ok_or_else(|| {
                anyhow::anyhow!("active route owner has no admitted physical WebSocket entry")
            })?;
        if admitted.selector != self.ingress_key.selector
            || admitted.gateway_entry_key != *self.entry.gateway_entry_key()
            || admitted.gateway_entry_identity != *self.entry.gateway_entry_identity()
            || !Arc::ptr_eq(&admitted.linked_entry, &self.entry)
        {
            anyhow::bail!("active route is not the exact admitted physical WebSocket entry");
        }
        Ok(admitted)
    }

    pub(crate) fn entry(&self) -> &Arc<LinkedGatewayEntry> {
        &self.entry
    }

    #[cfg(test)]
    pub(crate) fn selector_and_owner_key_share_entry(&self) -> bool {
        self.active
            .candidate
            .gateway_entry(self.entry.owner(), self.entry.gateway_entry_key())
            .is_some_and(|entry| Arc::ptr_eq(&self.entry, entry))
    }

    pub(crate) fn context_set(&self) -> &Arc<ActiveAssemblyContextSet> {
        &self.active.contexts
    }

    pub(crate) fn execution_image(
        &self,
    ) -> &Arc<skiff_runtime_linked_program::AssemblyExecutionImage> {
        self.active.candidate.execution_image()
    }

    pub(crate) fn db_source(
        &self,
    ) -> anyhow::Result<skiff_runtime_capability_context::DbCapabilitySource> {
        self.active
            .contexts
            .db_source(self.activation.activation_id())
            .ok_or_else(|| anyhow::anyhow!("active activation has no DB capability source"))
    }

    pub(crate) fn config_views(
        &self,
    ) -> anyhow::Result<Arc<super::config_snapshot::ActivationConfigViews>> {
        self.active
            .contexts
            .config_views(self.deployment())
            .ok_or_else(|| anyhow::anyhow!("active activation has no scoped config views"))
    }
}

impl ActiveAssembly {
    pub(crate) fn identity(&self) -> &AssemblyIdentity {
        &self.candidate.assembly().assembly_identity
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn candidate(&self) -> &Arc<AssemblyLinkedCandidate> {
        &self.candidate
    }

    pub(crate) fn contexts(&self) -> &Arc<ActiveAssemblyContextSet> {
        &self.contexts
    }

    pub(crate) fn contract_store(&self) -> &Arc<ServiceContractStore> {
        self.candidate.contract_store()
    }

    pub(crate) fn operation_descriptor(
        &self,
        contract: &ServiceContractRef,
        operation: &ContractOperationId,
    ) -> Option<&BoundaryOperationDescriptor> {
        self.candidate.operation_descriptor(contract, operation)
    }

    pub(crate) fn activation(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<&LinkedActivationTemplate> {
        self.candidate.activation(deployment)
    }

    pub(crate) fn ingress(&self, key: &ServiceIngressKey) -> Option<&Arc<LinkedGatewayEntry>> {
        self.candidate.ingress(key)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.candidate.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssemblyCandidateStage {
    Load,
    Link,
    Validate,
    Admit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssemblyCandidateHealth {
    pub(crate) generation: u64,
    pub(crate) identity: AssemblyIdentity,
    pub(crate) stage: AssemblyCandidateStage,
    pub(crate) started_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssemblyAdmissionOutcome {
    pub(crate) generation: u64,
    pub(crate) identity: AssemblyIdentity,
    pub(crate) succeeded: bool,
    pub(crate) stage: AssemblyCandidateStage,
    pub(crate) observed_at: OffsetDateTime,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct AssemblyAdmissionHealth {
    pub(crate) active_identity: Option<AssemblyIdentity>,
    pub(crate) active_generation: Option<u64>,
    pub(crate) active_admitted_at: Option<OffsetDateTime>,
    pub(crate) candidate: Option<AssemblyCandidateHealth>,
    pub(crate) last_outcome: Option<AssemblyAdmissionOutcome>,
}

#[derive(Debug, Default)]
struct AssemblyAdmissionState {
    next_generation: u64,
    active: Option<Arc<ActiveAssembly>>,
    committed: Option<CommittedAssembly>,
    staged: Option<StagedAssembly>,
    preparing: Option<AssemblyTransition>,
    candidate: Option<AssemblyCandidateHealth>,
    last_outcome: Option<AssemblyAdmissionOutcome>,
}

/// The sole owner of candidate build serialization and the active whole-assembly pointer.
#[derive(Debug)]
pub(crate) struct AssemblyAdmissionController {
    runtime_replica_id: String,
    db_provider: DbProviderSource,
    reload: Mutex<()>,
    state: RwLock<AssemblyAdmissionState>,
    /// Append-only set of deployments loaded under their buildId (M2 lazy-load
    /// registry). This is the routing authority: every request locates its
    /// deployment here, never through the whole-assembly active pointer.
    loaded: Arc<LoadedDeploymentRegistry>,
}

impl Default for AssemblyAdmissionController {
    fn default() -> Self {
        Self::new("runtime-replica", DbProviderSource::unavailable())
    }
}

impl AssemblyAdmissionController {
    pub(crate) fn new(
        runtime_replica_id: impl Into<String>,
        db_provider: DbProviderSource,
    ) -> Self {
        Self {
            runtime_replica_id: runtime_replica_id.into(),
            db_provider,
            reload: Mutex::new(()),
            state: RwLock::new(AssemblyAdmissionState::default()),
            loaded: Arc::new(LoadedDeploymentRegistry::default()),
        }
    }

    /// Test-only direct admission for focused loader/linker coverage. Production activation
    /// publishes only through durable committed recovery or Router prepare/commit.
    #[cfg(test)]
    pub(crate) async fn admit<R>(
        &self,
        assembly: impl Into<Arc<RuntimeAssembly>>,
        resolver: &R,
    ) -> anyhow::Result<Arc<ActiveAssembly>>
    where
        R: RuntimeAssemblyContentResolver + Sync + ?Sized,
    {
        let assembly = assembly.into();
        let identity = assembly.assembly_identity.clone();
        let _reload = self.reload.lock().await;
        let generation = self.begin_candidate(identity.clone())?;
        info!(
            event = "runtime.assembly_candidate_started",
            assembly_identity = %identity,
            generation
        );

        let prepared = self
            .build_started_candidate(
                generation, &identity, assembly, resolver, None, None, None, None,
            )
            .await?;
        let active = self.publish(generation, identity, prepared)?;
        info!(
            event = "runtime.assembly_admitted",
            assembly_identity = %active.identity(),
            generation = active.generation()
        );
        Ok(active)
    }

    async fn build_started_candidate<R>(
        &self,
        generation: u64,
        identity: &AssemblyIdentity,
        assembly: Arc<RuntimeAssembly>,
        resolver: &R,
        service_db: Option<&AssemblyActivationServiceDb>,
        profile: Option<&str>,
        config_snapshot_ref: Option<&RuntimeConfigSnapshotRef>,
        config_snapshot: Option<&skiff_runtime_config_snapshot::RuntimeConfigSnapshot>,
    ) -> anyhow::Result<PreparedAssembly>
    where
        R: RuntimeAssemblyContentResolver + Sync + ?Sized,
    {
        let config_snapshot_ref = match (config_snapshot_ref, config_snapshot) {
            (Some(reference), Some(snapshot)) if snapshot.snapshot_ref() == reference => {
                reference.clone()
            }
            (Some(_), Some(_)) => {
                anyhow::bail!("resolved RuntimeConfigSnapshot content mismatches exact ref")
            }
            #[cfg(test)]
            (None, None) => skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref(),
            _ => anyhow::bail!(
                "Runtime activation requires one exact RuntimeConfigSnapshot ref and record"
            ),
        };
        let hydrated = match RuntimeAssemblyLoader::new(resolver).load(assembly) {
            Ok(hydrated) => hydrated,
            Err(error) => {
                self.fail_candidate(generation, identity, AssemblyCandidateStage::Load)?;
                warn!(
                    event = "runtime.assembly_admission_failed",
                    assembly_identity = %identity,
                    generation,
                    stage = AssemblyCandidateStage::Load.as_str()
                );
                return Err(error).context("whole-assembly candidate load failed");
            }
        };

        self.advance_candidate(generation, AssemblyCandidateStage::Link)?;
        let candidate = match link_runtime_assembly(hydrated) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.fail_candidate(generation, identity, AssemblyCandidateStage::Link)?;
                warn!(
                    event = "runtime.assembly_admission_failed",
                    assembly_identity = %identity,
                    generation,
                    stage = AssemblyCandidateStage::Link.as_str()
                );
                return Err(error).context("whole-assembly candidate link failed");
            }
        };

        self.advance_candidate(generation, AssemblyCandidateStage::Validate)?;
        if let Err(error) = validate_candidate(&candidate) {
            self.fail_candidate(generation, identity, AssemblyCandidateStage::Validate)?;
            warn!(
                event = "runtime.assembly_admission_failed",
                assembly_identity = %identity,
                generation,
                stage = AssemblyCandidateStage::Validate.as_str()
            );
            return Err(error).context("whole-assembly candidate validation failed");
        }

        self.advance_candidate(generation, AssemblyCandidateStage::Admit)?;
        let candidate = Arc::new(candidate);
        let contexts = match ActiveAssemblyContextSet::from_candidate(
            &candidate,
            generation,
            &self.runtime_replica_id,
            &self.db_provider,
            service_db,
            profile,
            config_snapshot,
        )
        .await
        {
            Ok(contexts) => Arc::new(contexts),
            Err(error) => {
                self.fail_candidate(generation, identity, AssemblyCandidateStage::Admit)?;
                warn!(
                    event = "runtime.assembly_admission_failed",
                    assembly_identity = %identity,
                    generation,
                    stage = AssemblyCandidateStage::Admit.as_str()
                );
                return Err(error).context("whole-assembly activation context construction failed");
            }
        };
        Ok(PreparedAssembly {
            generation,
            config_snapshot: config_snapshot_ref,
            candidate,
            contexts,
        })
    }

    pub(crate) fn active(&self) -> anyhow::Result<Option<Arc<ActiveAssembly>>> {
        Ok(self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?
            .active
            .clone())
    }

    pub(crate) fn health(&self) -> anyhow::Result<AssemblyAdmissionHealth> {
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        Ok(AssemblyAdmissionHealth {
            active_identity: state
                .active
                .as_ref()
                .map(|active| active.identity().clone()),
            active_generation: state.active.as_ref().map(|active| active.generation),
            active_admitted_at: state.active.as_ref().map(|active| active.admitted_at),
            candidate: state.candidate.clone(),
            last_outcome: state.last_outcome.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn route(
        &self,
        key: &ServiceIngressKey,
    ) -> anyhow::Result<Option<ActiveAssemblyRoute>> {
        let build_id = key.deployment.deployment_artifact_identity.as_str();
        let Some(active) = self.loaded.lookup(build_id) else {
            return Ok(None);
        };
        self.route_from_active(active, key)
    }

    /// Ordered snapshot of every loaded buildId (capability advertisement).
    pub(crate) fn loaded_build_ids(&self) -> Vec<String> {
        self.loaded.loaded_build_ids()
    }

    /// Whether one exact buildId is already in the loaded registry.
    pub(crate) fn is_loaded(&self, build_id: &str) -> bool {
        self.loaded.lookup(build_id).is_some()
    }

    /// Gateway surfaces of every loaded deployment (dispatch-mode capability).
    pub(crate) fn loaded_gateway_surfaces(
        &self,
    ) -> Vec<skiff_artifact_model::GatewayEntryProtocolSurface> {
        let mut surfaces = Vec::new();
        for build_id in self.loaded_build_ids() {
            let Some(active) = self.loaded.lookup(&build_id) else {
                continue;
            };
            surfaces.extend(
                active
                    .candidate()
                    .gateway_entries()
                    .map(|(_, entry)| entry.protocol_surface().clone()),
            );
        }
        surfaces
    }

    /// Resolves a route from the loaded registry, lazy-loading the deployment
    /// under its per-buildId critical section when it is not loaded yet.
    ///
    /// Load failures (missing record, unreachable directory, invalid content)
    /// fast-fail every waiting request; nothing is registered on failure.
    pub(crate) async fn route_or_lazy_load<R, C>(
        &self,
        key: &ServiceIngressKey,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
        profile: &str,
    ) -> anyhow::Result<ActiveAssemblyRoute>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let active = self
            .deployment_image_or_lazy_load(
                &key.deployment,
                resolver,
                config_snapshot_resolver,
                service_db,
                profile,
            )
            .await?;
        self.route_from_active(active, key)?.ok_or_else(|| {
            anyhow::anyhow!(
                "loaded deployment {} has no admitted ingress matching {key:?}",
                key.deployment.deployment_artifact_identity
            )
        })
    }

    /// Resolves the loaded image for one exact deployment, lazy-loading it
    /// under its per-buildId critical section when absent.
    pub(crate) async fn deployment_image_or_lazy_load<R, C>(
        &self,
        deployment: &ServiceDeploymentRef,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
        profile: &str,
    ) -> anyhow::Result<Arc<ActiveAssembly>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let build_id = deployment.deployment_artifact_identity.as_str();
        self.loaded
            .load_or_wait(build_id, || {
                self.load_lazy_deployment(
                    deployment,
                    resolver,
                    config_snapshot_resolver,
                    service_db,
                    profile,
                )
            })
            .await
    }

    async fn load_lazy_deployment<R, C>(
        &self,
        deployment: &ServiceDeploymentRef,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
        profile: &str,
    ) -> anyhow::Result<Arc<ActiveAssembly>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let reference = deployment;
        info!(
            event = "runtime.deployment_lazy_load_started",
            build_id = %reference.deployment_artifact_identity,
            deployment = %format_args!("{reference:?}")
        );
        let hydrated =
            skiff_runtime_loader::DeploymentAssemblyLoader::new(resolver).load_ref(reference)?;
        let candidate = link_runtime_assembly(hydrated)
            .map_err(|error| error.context("lazy-load deployment link failed"))?;
        validate_candidate(&candidate)?;
        let candidate = Arc::new(candidate);
        let config_snapshot_ref = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?
            .committed
            .as_ref()
            .map(|committed| committed.config_snapshot.clone())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "lazy-load deployment {reference:?} requires a bootstrapped config snapshot"
                )
            })?;
        let config_snapshot = config_snapshot_resolver
            .resolve(&config_snapshot_ref)
            .map_err(|_| {
                anyhow::anyhow!(
                    "RuntimeConfigSnapshot {} resolution failed during lazy load",
                    config_snapshot_ref.snapshot_id
                )
            })?;
        if *config_snapshot.snapshot_ref() != config_snapshot_ref {
            anyhow::bail!(
                "RuntimeConfigSnapshot {} rejected: resolved content does not match the requested opaque id",
                config_snapshot_ref.snapshot_id
            );
        }
        if let Err(error) =
            super::config_snapshot::validate_snapshot_profile(&config_snapshot, profile)
        {
            anyhow::bail!("lazy-load deployment {reference:?} rejected: {error}");
        }
        let contexts = ActiveAssemblyContextSet::from_candidate(
            &candidate,
            0,
            &self.runtime_replica_id,
            &self.db_provider,
            service_db,
            Some(profile),
            Some(&config_snapshot),
        )
        .await
        .map_err(|error| {
            error.context("lazy-load deployment activation context construction failed")
        })?;
        let active = Arc::new(ActiveAssembly {
            generation: 0,
            admitted_at: OffsetDateTime::now_utc(),
            candidate,
            contexts: Arc::new(contexts),
        });
        info!(
            event = "runtime.deployment_lazy_loaded",
            build_id = %reference.deployment_artifact_identity,
            deployment = %format_args!("{reference:?}")
        );
        Ok(active)
    }

    fn route_from_active(
        &self,
        active: Arc<ActiveAssembly>,
        key: &ServiceIngressKey,
    ) -> anyhow::Result<Option<ActiveAssemblyRoute>> {
        let Some(entry) = active.ingress(key).cloned() else {
            return Ok(None);
        };
        if entry.owner() != &key.deployment {
            anyhow::bail!("active assembly ingress key and linked entry owner disagree");
        }
        let exact_entry = active
            .candidate
            .gateway_entry(entry.owner(), entry.gateway_entry_key())
            .ok_or_else(|| anyhow::anyhow!("active assembly gateway entry lookup is missing"))?;
        if !Arc::ptr_eq(&entry, exact_entry) {
            anyhow::bail!("active assembly selector and owner/key lookup disagree");
        }
        let activation = active
            .contexts
            .activation_for_deployment(entry.owner())
            .ok_or_else(|| anyhow::anyhow!("active assembly gateway has no activation context"))?;
        Ok(Some(ActiveAssemblyRoute {
            active,
            ingress_key: key.clone(),
            entry,
            activation,
        }))
    }

    /// Resolves the exact immutable route authority carried by an Actor owner
    /// invoke/control frame against the current active assembly only. The Host
    /// layer additionally consults route holds owned by live Actor executions;
    /// anything else fails closed so a G1 chain cannot be silently re-executed
    /// on a newer generation.
    pub(crate) fn actor_execution_route(
        &self,
        authority: &ActorOwnerRouteAuthorityFrameHeader,
        service_id: &str,
    ) -> anyhow::Result<Option<ActiveActorExecutionRoute>> {
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        let Some(active) = state.active.as_ref() else {
            return Ok(None);
        };
        if active.generation() != authority.assembly_generation
            || active.identity().as_str() != authority.assembly_identity
        {
            return Ok(None);
        }
        actor_route_from_active(Arc::clone(active), service_id)
    }

    #[cfg(test)]
    fn begin_candidate(&self, identity: AssemblyIdentity) -> anyhow::Result<u64> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        if let Some(candidate) = &state.candidate {
            anyhow::bail!(
                "assembly admission generation {} is already building",
                candidate.generation
            );
        }
        let generation = state
            .next_generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("assembly admission generation exhausted"))?;
        state.next_generation = generation;
        state.candidate = Some(AssemblyCandidateHealth {
            generation,
            identity,
            stage: AssemblyCandidateStage::Load,
            started_at: OffsetDateTime::now_utc(),
        });
        Ok(generation)
    }

    fn advance_candidate(
        &self,
        generation: u64,
        stage: AssemblyCandidateStage,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        let candidate = state.candidate.as_mut().ok_or_else(|| {
            anyhow::anyhow!("assembly admission generation {generation} has no candidate")
        })?;
        if candidate.generation != generation {
            anyhow::bail!(
                "assembly admission generation {generation} cannot advance candidate generation {}",
                candidate.generation
            );
        }
        if candidate.stage.next() != Some(stage) {
            anyhow::bail!(
                "assembly admission generation {generation} cannot advance from {} to {}",
                candidate.stage.as_str(),
                stage.as_str()
            );
        }
        candidate.stage = stage;
        Ok(())
    }

    fn fail_candidate(
        &self,
        generation: u64,
        identity: &AssemblyIdentity,
        stage: AssemblyCandidateStage,
    ) -> anyhow::Result<()> {
        self.fail_candidate_with_health_error(
            generation,
            identity,
            stage,
            format!("whole-assembly {} failed", stage.as_str()),
        )
    }

    fn fail_candidate_with_health_error(
        &self,
        generation: u64,
        identity: &AssemblyIdentity,
        stage: AssemblyCandidateStage,
        health_error: String,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        ensure_current_candidate(&state, generation, identity)?;
        state.candidate = None;
        state.preparing = None;
        state.last_outcome = Some(AssemblyAdmissionOutcome {
            generation,
            identity: identity.clone(),
            succeeded: false,
            stage,
            observed_at: OffsetDateTime::now_utc(),
            // Resolver/linker errors may contain secret-bearing deployment values. Health keeps
            // only a bounded, explicitly redacted category supplied by the caller.
            error: Some(health_error),
        });
        Ok(())
    }

    fn fail_candidate_config_snapshot_profile(
        &self,
        generation: u64,
        identity: &AssemblyIdentity,
        config_snapshot: &RuntimeConfigSnapshotRef,
    ) -> anyhow::Result<()> {
        self.fail_candidate_with_health_error(
            generation,
            identity,
            AssemblyCandidateStage::Load,
            format!(
                "RuntimeConfigSnapshot {} profile mismatch",
                config_snapshot.snapshot_id
            ),
        )
    }

    #[cfg(test)]
    fn publish(
        &self,
        generation: u64,
        identity: AssemblyIdentity,
        prepared: PreparedAssembly,
    ) -> anyhow::Result<Arc<ActiveAssembly>> {
        let admitted_at = OffsetDateTime::now_utc();
        let active = Arc::new(ActiveAssembly {
            generation,
            admitted_at,
            candidate: prepared.candidate,
            contexts: prepared.contexts,
        });
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        ensure_current_candidate(&state, generation, &identity)?;
        // Active, candidate and outcome are one state-lock transaction. Readers can observe
        // either the complete previous assembly or the complete new generation, never a mix.
        state.active = Some(Arc::clone(&active));
        state.candidate = None;
        state.preparing = None;
        state.last_outcome = Some(AssemblyAdmissionOutcome {
            generation,
            identity,
            succeeded: true,
            stage: AssemblyCandidateStage::Admit,
            observed_at: admitted_at,
            error: None,
        });
        for (deployment, _) in active.candidate().activations() {
            self.loaded.register(
                deployment.deployment_artifact_identity.as_str(),
                Arc::clone(&active),
            );
        }
        Ok(active)
    }
}

impl RuntimeHost {
    /// Applies one router-coordinated activation transition through the exact
    /// production record resolver boundary.
    pub async fn apply_assembly_activation_control<R, C>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
        config_snapshot_resolver: &C,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        self.apply_bootstrapped_assembly_activation_control(
            control,
            resolver,
            config_snapshot_resolver,
            None,
        )
        .await
    }

    /// Production activation with the connection bootstrap's fixed DB transport binding.
    pub async fn apply_bootstrapped_assembly_activation_control<R, C>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let profile = activation_control_profile(&control);
        match self.trusted_profile() {
            Some(frozen) if frozen == profile => {}
            Some(frozen) => anyhow::bail!(
                "assembly activation profile {profile} does not match Runtime frozen profile {frozen}"
            ),
            None => anyhow::bail!(
                "assembly activation profile {profile} requires a router bootstrap profile first"
            ),
        }
        self.assembly_admission
            .apply_activation_control(control, resolver, config_snapshot_resolver, service_db)
            .await
    }

    /// Production activation with a connection-scoped cancellation token for Prepare.
    ///
    /// Router Abort and session teardown signal this token before waiting for the
    /// serialized admission transition. Commit and Abort themselves are never
    /// cancellable through this path.
    pub(crate) async fn apply_cancellable_bootstrapped_assembly_activation_control<R, C>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
        cancellation: &CancellationToken,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let profile = activation_control_profile(&control);
        match self.trusted_profile() {
            Some(frozen) if frozen == profile => {}
            Some(frozen) => anyhow::bail!(
                "assembly activation profile {profile} does not match Runtime frozen profile {frozen}"
            ),
            None => anyhow::bail!(
                "assembly activation profile {profile} requires a router bootstrap profile first"
            ),
        }
        self.assembly_admission
            .apply_cancellable_activation_control(
                control,
                resolver,
                config_snapshot_resolver,
                service_db,
                cancellation,
            )
            .await
    }

    /// Returns only the currently committed whole-assembly registration.
    pub fn active_assembly_registration(
        &self,
    ) -> anyhow::Result<Option<AssemblyActivationControl>> {
        self.assembly_admission.registration()
    }

    #[allow(dead_code)] // Phase 04 execution consumes an immutable active-generation snapshot.
    pub(crate) fn active_runtime_assembly(&self) -> anyhow::Result<Option<Arc<ActiveAssembly>>> {
        self.assembly_admission.active()
    }

    #[cfg(test)]
    pub(crate) fn runtime_assembly_admission_health(
        &self,
    ) -> anyhow::Result<AssemblyAdmissionHealth> {
        self.assembly_admission.health()
    }

    #[cfg(test)]
    pub(crate) fn active_runtime_assembly_route(
        &self,
        key: &ServiceIngressKey,
    ) -> anyhow::Result<Option<ActiveAssemblyRoute>> {
        self.assembly_admission.route(key)
    }

    pub(crate) fn actor_execution_route(
        &self,
        authority: &ActorOwnerRouteAuthorityFrameHeader,
        service_id: &str,
    ) -> anyhow::Result<Option<ActiveActorExecutionRoute>> {
        if let Some(route) = self
            .assembly_admission
            .actor_execution_route(authority, service_id)?
        {
            return Ok(Some(route));
        }
        if let Some(active) = self
            .actor_route_holds
            .find(&authority.assembly_identity, authority.assembly_generation)
        {
            return actor_route_from_active(active, service_id);
        }
        anyhow::bail!(
            "Actor route authority {} generation {} is not retained",
            authority.assembly_identity,
            authority.assembly_generation
        );
    }
}

fn activation_control_profile(control: &AssemblyActivationControl) -> &str {
    match control {
        AssemblyActivationControl::Prepare { profile, .. }
        | AssemblyActivationControl::Prepared { profile, .. }
        | AssemblyActivationControl::Reject { profile, .. }
        | AssemblyActivationControl::Commit { profile, .. }
        | AssemblyActivationControl::Abort { profile, .. }
        | AssemblyActivationControl::Register { profile, .. } => profile,
    }
}

impl AssemblyCandidateStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Link => "link",
            Self::Validate => "validation",
            Self::Admit => "admission",
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Load => Some(Self::Link),
            Self::Link => Some(Self::Validate),
            Self::Validate => Some(Self::Admit),
            Self::Admit => None,
        }
    }

    #[cfg(test)]
    fn ordinal(self) -> usize {
        match self {
            Self::Load => 0,
            Self::Link => 1,
            Self::Validate => 2,
            Self::Admit => 3,
        }
    }
}

fn ensure_current_candidate(
    state: &AssemblyAdmissionState,
    generation: u64,
    identity: &AssemblyIdentity,
) -> anyhow::Result<()> {
    let candidate = state.candidate.as_ref().ok_or_else(|| {
        anyhow::anyhow!("assembly admission generation {generation} has no candidate")
    })?;
    if candidate.generation != generation || &candidate.identity != identity {
        anyhow::bail!(
            "assembly admission candidate changed while generation {generation} was building"
        );
    }
    Ok(())
}

/// The sole committed publication primitive shared by online commit and durable recovery.
/// Callers hold the admission state write lock, so active context and committed tuple become
/// visible atomically.
fn publish_committed_locked(
    state: &mut AssemblyAdmissionState,
    prepared: PreparedAssembly,
    committed: CommittedAssembly,
) -> anyhow::Result<Arc<ActiveAssembly>> {
    let admitted_at = OffsetDateTime::now_utc();
    let identity = prepared.candidate.assembly().assembly_identity.clone();
    if prepared.generation != committed.generation
        || identity != committed.assembly.assembly_identity
        || prepared.config_snapshot != committed.config_snapshot
    {
        anyhow::bail!(
            "prepared assembly/config snapshot does not match committed publication tuple"
        );
    }
    let active = Arc::new(ActiveAssembly {
        generation: prepared.generation,
        admitted_at,
        candidate: prepared.candidate,
        contexts: prepared.contexts,
    });
    state.next_generation = state.next_generation.max(committed.generation);
    state.active = Some(Arc::clone(&active));
    state.committed = Some(committed);
    state.staged = None;
    state.preparing = None;
    state.candidate = None;
    state.last_outcome = Some(AssemblyAdmissionOutcome {
        generation: active.generation(),
        identity,
        succeeded: true,
        stage: AssemblyCandidateStage::Admit,
        observed_at: admitted_at,
        error: None,
    });
    Ok(active)
}

fn actor_route_from_active(
    active: Arc<ActiveAssembly>,
    service_id: &str,
) -> anyhow::Result<Option<ActiveActorExecutionRoute>> {
    let deployments = active
        .candidate
        .activations()
        .filter(|(deployment, _)| deployment.service_id == service_id)
        .map(|(deployment, _)| deployment.clone())
        .collect::<Vec<_>>();
    let Some(deployment) = deployments.first() else {
        return Ok(None);
    };
    if deployments.len() != 1 {
        anyhow::bail!(
            "Actor service {service_id} has multiple deployments in generation {}; invocation is ambiguous",
            active.generation()
        );
    }
    let activation = active
        .contexts
        .activation_for_deployment(deployment)
        .ok_or_else(|| anyhow::anyhow!("Actor service activation context is missing"))?;
    drop(deployments);
    Ok(Some(ActiveActorExecutionRoute { active, activation }))
}

fn reject_reason_for_stage(stage: AssemblyCandidateStage) -> AssemblyActivationRejectReason {
    match stage {
        AssemblyCandidateStage::Load => AssemblyActivationRejectReason::Load,
        AssemblyCandidateStage::Link => AssemblyActivationRejectReason::Link,
        AssemblyCandidateStage::Validate | AssemblyCandidateStage::Admit => {
            AssemblyActivationRejectReason::Admission
        }
    }
}

fn validate_candidate(candidate: &AssemblyLinkedCandidate) -> anyhow::Result<()> {
    let assembly = candidate.assembly();
    if candidate.shared_image().assembly_identity() != &assembly.assembly_identity {
        anyhow::bail!("linked package image belongs to a different assembly");
    }
    if candidate.activations().len() != assembly.activation_templates.len() {
        anyhow::bail!("linked activation set does not exactly match RuntimeAssembly");
    }
    if candidate.ingress_bindings().len() != assembly.gateway_ingress.len() {
        anyhow::bail!("linked ingress set does not exactly match RuntimeAssembly");
    }

    for source in &assembly.activation_templates {
        let activation = candidate.activation(&source.deployment).ok_or_else(|| {
            anyhow::anyhow!("linked activation {:?} is missing", source.deployment)
        })?;
        if activation.source() != source {
            anyhow::bail!(
                "linked activation {:?} changed its template",
                source.deployment
            );
        }
        if candidate
            .contract_store()
            .contract(activation.contract())
            .is_none()
        {
            anyhow::bail!(
                "linked activation {:?} has no canonical contract",
                source.deployment
            );
        }
        for (operation_id, _) in activation.operations() {
            if candidate
                .operation_descriptor(activation.contract(), operation_id)
                .is_none()
            {
                anyhow::bail!(
                    "linked activation {:?} operation {} has no canonical descriptor",
                    source.deployment,
                    operation_id
                );
            }
        }
        admitted_websocket_entry(candidate, &source.deployment)?;
    }

    for source in &assembly.gateway_ingress {
        let entry = candidate
            .ingress(&source.service_ingress_key())
            .ok_or_else(|| anyhow::anyhow!("linked ingress {:?} is missing", source.selector))?;
        let exact_entry = candidate
            .gateway_entry(&source.deployment, &source.gateway_entry_key)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "linked ingress {:?} owner/key entry is missing",
                    source.selector
                )
            })?;
        if !Arc::ptr_eq(entry, exact_entry) {
            anyhow::bail!(
                "linked ingress {:?} selector and owner/key lookups disagree",
                source.selector
            );
        }
        if entry.owner() != &source.deployment
            || entry.gateway_entry_key() != &source.gateway_entry_key
            || entry.gateway_entry_identity() != &source.gateway_entry_identity
        {
            anyhow::bail!("linked ingress {:?} changed its binding", source.selector);
        }
        let activation = candidate.activation(entry.owner()).ok_or_else(|| {
            anyhow::anyhow!("linked ingress {:?} has no activation", source.selector)
        })?;
        let deployment_entry = activation
            .deployment()
            .gateway_entries
            .get(entry.gateway_entry_key())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "linked ingress {:?} deployment entry is missing",
                    source.selector
                )
            })?;
        if deployment_entry.gateway_entry_identity != *entry.gateway_entry_identity()
            || deployment_entry.protocol_surface != *entry.protocol_surface()
        {
            anyhow::bail!(
                "linked ingress {:?} deployment entry facts changed",
                source.selector
            );
        }
        let Some(deployment_binding) = activation
            .deployment()
            .ingress
            .iter()
            .find(|binding| binding.selector == source.selector)
        else {
            anyhow::bail!(
                "linked ingress {:?} deployment binding is missing",
                source.selector
            );
        };
        if deployment_binding.gateway_entry_key != *entry.gateway_entry_key() {
            anyhow::bail!(
                "linked ingress {:?} deployment binding key changed",
                source.selector
            );
        }
        match (source.selector.protocol, &entry.protocol_surface().protocol) {
            (IngressProtocol::Http, GatewayProtocolSurface::Http(http)) => {
                let mode_is_valid = matches!(
                    (http.adapter_kind, http.dispatch_mode),
                    (GatewayAdapterKind::TypedJson, GatewayDispatchMode::Unary)
                        | (GatewayAdapterKind::RawHttp, GatewayDispatchMode::Unary)
                        | (
                            GatewayAdapterKind::RawHttp,
                            GatewayDispatchMode::ServerStream
                        )
                );
                if !mode_is_valid {
                    anyhow::bail!(
                        "linked ingress {:?} has an unsupported HTTP adapter/mode",
                        source.selector
                    );
                }
            }
            (IngressProtocol::WebSocket, GatewayProtocolSurface::WebSocketConnect(_)) => {
                let admitted =
                    admitted_websocket_entry(candidate, entry.owner())?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "linked ingress {:?} has no admitted WebSocket activation entry",
                            source.selector
                        )
                    })?;
                if admitted.selector != source.selector
                    || admitted.gateway_entry_key != *entry.gateway_entry_key()
                    || admitted.gateway_entry_identity != *entry.gateway_entry_identity()
                    || entry.adapter_plan().kind != GatewayAdapterKind::WebSocketConnect
                {
                    anyhow::bail!(
                        "linked ingress {:?} does not exactly match its admitted WebSocket entry",
                        source.selector
                    );
                }
            }
            (IngressProtocol::WebSocket, GatewayProtocolSurface::WebSocketJsonRpc(surface)) => {
                let admitted =
                    admitted_websocket_entry(candidate, entry.owner())?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "linked ingress {:?} has no admitted physical WebSocket entry",
                            source.selector
                        )
                    })?;
                let method = source.selector.method.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "linked WebSocket JSON-RPC ingress {:?} has no method",
                        source.selector
                    )
                })?;
                let admitted_method = admitted.method(method).ok_or_else(|| {
                    anyhow::anyhow!(
                        "linked ingress {:?} is absent from its immutable method table",
                        source.selector
                    )
                })?;
                if admitted_method.selector != source.selector
                    || admitted_method.gateway_entry_key != *entry.gateway_entry_key()
                    || admitted_method.gateway_entry_identity != *entry.gateway_entry_identity()
                    || admitted_method.profile != surface.profile
                    || !Arc::ptr_eq(&admitted_method.linked_entry, entry)
                    || entry.adapter_plan().kind != GatewayAdapterKind::WebSocketJsonRpc
                {
                    anyhow::bail!(
                        "linked ingress {:?} does not exactly match its admitted WebSocket JSON-RPC sibling",
                        source.selector
                    );
                }
            }
            _ => {
                anyhow::bail!(
                    "linked ingress {:?} protocol and entry surface do not match",
                    source.selector
                );
            }
        }
        if candidate.activation(entry.owner()).is_none() {
            anyhow::bail!("linked ingress {:?} has no activation", source.selector);
        }
    }

    let canonical_empty = assembly.roots.is_empty()
        && assembly.resolved_deployments.is_empty()
        && assembly.resolved_contracts.is_empty()
        && assembly.resolved_packages.is_empty()
        && assembly.package_link_plan.code_slots.is_empty()
        && assembly.package_link_plan.package_links.is_empty()
        && assembly.service_binding_templates.is_empty()
        && assembly.activation_templates.is_empty()
        && assembly.gateway_ingress.is_empty();
    if candidate.is_empty() != canonical_empty {
        anyhow::bail!("linked candidate empty state does not match RuntimeAssembly");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
