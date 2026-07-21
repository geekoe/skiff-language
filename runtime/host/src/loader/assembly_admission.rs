use std::sync::{Arc, RwLock};

use anyhow::Context;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, AssemblyIdentity,
    BoundaryOperationDescriptor, ContractOperationId, GlobalIngressBinding, IngressSelector,
    OperationTargetRef, RuntimeAssembly, RuntimeAssemblyRef, ServiceContract, ServiceContractRef,
    ServiceDeploymentRef,
};
use skiff_runtime_activation::{ActivationContext, RequestActivationContext};
use skiff_runtime_eval::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};
use skiff_runtime_linker::{
    link_runtime_assembly, AssemblyLinkedCandidate, LinkedActivationTemplate,
};
use skiff_runtime_loader::{
    RuntimeAssemblyContentResolver, RuntimeAssemblyLoader, RuntimeAssemblyRecordResolver,
    ServiceContractStore,
};
use skiff_runtime_request::RuntimeAssemblyRequestTarget;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::active_assembly_context::ActiveAssemblyContextSet;
use crate::host::RuntimeHost;

mod provisioning;

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
    candidate: Arc<AssemblyLinkedCandidate>,
    contexts: Arc<ActiveAssemblyContextSet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssemblyTransition {
    environment: String,
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
    assembly: RuntimeAssemblyRef,
}

#[derive(Debug)]
struct StagedAssembly {
    transition: AssemblyTransition,
    prepared: PreparedAssembly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedAssembly {
    environment: String,
    generation: u64,
    assembly: RuntimeAssemblyRef,
}

/// One request-entry route pinned to the exact active generation used for lookup.
#[derive(Debug, Clone)]
pub(crate) struct ActiveAssemblyRoute {
    active: Arc<ActiveAssembly>,
    binding: GlobalIngressBinding,
    activation: Arc<ActivationContext>,
    descriptor: BoundaryOperationDescriptor,
    contract: Arc<ServiceContract>,
    provider_target: OperationTargetRef,
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

    pub(crate) fn operation_descriptor(&self) -> &BoundaryOperationDescriptor {
        &self.descriptor
    }

    pub(crate) fn provider_target(&self) -> &OperationTargetRef {
        &self.provider_target
    }

    pub(crate) fn request_target(&self) -> anyhow::Result<RuntimeAssemblyRequestTarget> {
        let request_activation = RequestActivationContext::begin(Arc::clone(&self.activation))?;
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::clone(&self.active.contexts) as _;
        let eval = RuntimeAssemblyEvalTarget::new(
            Arc::clone(self.execution_image()),
            request_activation,
            resolver,
        )?;
        let boundary = eval.resolve_ingress_target(
            &self.binding.contract,
            &self.binding.contract_operation_id,
            Arc::clone(&self.contract),
            &self.provider_target,
        )?;
        Ok(RuntimeAssemblyRequestTarget::new(eval, boundary)?)
    }

    pub(crate) fn binding(&self) -> &GlobalIngressBinding {
        &self.binding
    }

    pub(crate) fn context_set(&self) -> &Arc<ActiveAssemblyContextSet> {
        &self.active.contexts
    }

    pub(crate) fn execution_image(
        &self,
    ) -> &Arc<skiff_runtime_linked_program::AssemblyExecutionImage> {
        self.active.candidate.execution_image()
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

    pub(crate) fn ingress(&self, selector: &IngressSelector) -> Option<&GlobalIngressBinding> {
        self.candidate.ingress(selector)
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
    candidate: Option<AssemblyCandidateHealth>,
    last_outcome: Option<AssemblyAdmissionOutcome>,
}

/// The sole owner of candidate build serialization and the active whole-assembly pointer.
#[derive(Debug)]
pub(crate) struct AssemblyAdmissionController {
    runtime_replica_id: String,
    reload: Mutex<()>,
    state: RwLock<AssemblyAdmissionState>,
}

impl Default for AssemblyAdmissionController {
    fn default() -> Self {
        Self::new("runtime-replica")
    }
}

impl AssemblyAdmissionController {
    pub(crate) fn new(runtime_replica_id: impl Into<String>) -> Self {
        Self {
            runtime_replica_id: runtime_replica_id.into(),
            reload: Mutex::new(()),
            state: RwLock::new(AssemblyAdmissionState::default()),
        }
    }

    /// Executes the only production whole-assembly admission path.
    ///
    /// The reload permit spans typed hydration, linking, host validation and publication. The
    /// active pointer is changed only by `publish`, after all fallible candidate work completed.
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
            .build_started_candidate(generation, &identity, assembly, resolver)
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
    ) -> anyhow::Result<PreparedAssembly>
    where
        R: RuntimeAssemblyContentResolver + Sync + ?Sized,
    {
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
        ) {
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

    pub(crate) fn route(
        &self,
        selector: &IngressSelector,
    ) -> anyhow::Result<Option<ActiveAssemblyRoute>> {
        let Some(active) = self.active()? else {
            return Ok(None);
        };
        let Some(binding) = active.ingress(selector).cloned() else {
            return Ok(None);
        };
        let activation = active
            .contexts
            .activation_for_deployment(&binding.deployment)
            .ok_or_else(|| anyhow::anyhow!("active assembly ingress has no activation context"))?;
        let descriptor = active
            .operation_descriptor(&binding.contract, &binding.contract_operation_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("active assembly ingress has no descriptor"))?;
        let contract = active
            .contexts
            .contract(&binding.contract)
            .ok_or_else(|| anyhow::anyhow!("active assembly ingress has no canonical contract"))?;
        let provider_target = active
            .contexts
            .operation_target(activation.activation_id(), &binding.contract_operation_id)
            .ok_or_else(|| anyhow::anyhow!("active assembly ingress has no provider target"))?;
        Ok(Some(ActiveAssemblyRoute {
            active,
            binding,
            activation,
            descriptor,
            contract,
            provider_target,
        }))
    }

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
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        ensure_current_candidate(&state, generation, identity)?;
        state.candidate = None;
        state.last_outcome = Some(AssemblyAdmissionOutcome {
            generation,
            identity: identity.clone(),
            succeeded: false,
            stage,
            observed_at: OffsetDateTime::now_utc(),
            // Health deliberately retains a stage-only diagnostic. Resolver/linker errors may
            // contain secret-bearing deployment values and remain only in the returned error.
            error: Some(format!("whole-assembly {} failed", stage.as_str())),
        });
        Ok(())
    }

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
        state.last_outcome = Some(AssemblyAdmissionOutcome {
            generation,
            identity,
            succeeded: true,
            stage: AssemblyCandidateStage::Admit,
            observed_at: admitted_at,
            error: None,
        });
        Ok(active)
    }
}

impl RuntimeHost {
    /// Applies one router-coordinated activation transition through the exact
    /// production record resolver boundary.
    pub async fn apply_assembly_activation_control<R>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
    {
        self.assembly_admission
            .apply_activation_control(control, resolver)
            .await
    }

    /// Returns only the currently committed whole-assembly registration.
    pub fn active_assembly_registration(
        &self,
    ) -> anyhow::Result<Option<AssemblyActivationControl>> {
        self.assembly_admission.registration()
    }

    /// Builds and atomically admits one complete typed runtime assembly.
    pub async fn admit_runtime_assembly<R>(
        &self,
        assembly: impl Into<Arc<RuntimeAssembly>>,
        resolver: &R,
    ) -> anyhow::Result<AssemblyIdentity>
    where
        R: RuntimeAssemblyContentResolver + Sync + ?Sized,
    {
        self.assembly_admission
            .admit(assembly, resolver)
            .await
            .map(|active| active.identity().clone())
    }

    #[allow(dead_code)] // Phase 04 execution consumes an immutable active-generation snapshot.
    pub(crate) fn active_runtime_assembly(&self) -> anyhow::Result<Option<Arc<ActiveAssembly>>> {
        self.assembly_admission.active()
    }

    #[allow(dead_code)] // Control-plane health consumes this without owning admission state.
    pub(crate) fn runtime_assembly_admission_health(
        &self,
    ) -> anyhow::Result<AssemblyAdmissionHealth> {
        self.assembly_admission.health()
    }

    pub(crate) fn active_runtime_assembly_route(
        &self,
        selector: &IngressSelector,
    ) -> anyhow::Result<Option<ActiveAssemblyRoute>> {
        self.assembly_admission.route(selector)
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

fn validate_candidate(candidate: &AssemblyLinkedCandidate) -> anyhow::Result<()> {
    let assembly = candidate.assembly();
    if candidate.shared_image().assembly_identity() != &assembly.assembly_identity {
        anyhow::bail!("linked package image belongs to a different assembly");
    }
    if candidate.activations().len() != assembly.activation_templates.len() {
        anyhow::bail!("linked activation set does not exactly match RuntimeAssembly");
    }
    if candidate.ingress_bindings().len() != assembly.global_ingress.len() {
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
    }

    for source in &assembly.global_ingress {
        let binding = candidate
            .ingress(&source.selector)
            .ok_or_else(|| anyhow::anyhow!("linked ingress {:?} is missing", source.selector))?;
        if binding != source {
            anyhow::bail!("linked ingress {:?} changed its binding", source.selector);
        }
        if candidate.activation(&binding.deployment).is_none() {
            anyhow::bail!("linked ingress {:?} has no activation", source.selector);
        }
        if candidate
            .operation_descriptor(&binding.contract, &binding.contract_operation_id)
            .is_none()
        {
            anyhow::bail!(
                "linked ingress {:?} has no canonical operation descriptor",
                source.selector
            );
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
        && assembly.global_ingress.is_empty();
    if candidate.is_empty() != canonical_empty {
        anyhow::bail!("linked candidate empty state does not match RuntimeAssembly");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
