use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use async_recursion::async_recursion;
use skiff_artifact_model::{ContractOperationId, InstructionSourceSite};
use skiff_runtime_linked_program::{
    ConstAddr, ExecutableAddr, ExecutableKind, ExprRefIr, LinkedExecutable, LinkedExprIr,
    LinkedFileUnit, LinkedStmtIr, LinkedTypeRef,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{ErrorCorrelation, ExceptionStackFrame},
    type_plan::RuntimeTypePlan,
};

#[allow(unused_imports)]
pub use super::program_types::executable_type_param_names;
use super::{
    capabilities::{
        ActorCapabilityContext, ConfigCapabilityContext, DbCapabilityContext,
        EffectDispatchContext, ExecutionControl, FileCapabilityContext, FileCapabilitySource,
        FileSourceStreamContext, HttpClientCapabilityContext, OwnedActorCapabilityContext,
        OwnedConfigCapabilityContext, OwnedExecutionControl, OwnedWebsocketCapabilityContext,
        StreamRuntime, StreamRuntimeOwner, TelemetryCapabilityContext, TestEffectDoubleContext,
        TimeCapabilityContext, WebsocketCapabilityContext,
    },
    error::attach_source_frame,
    eval_context::{promote_call_site_error, EvalContext},
    flow_completion::FlowCompletionPolicy,
    invocation::{EvalInvocation, EvalProgramProjection},
    program_ir::{
        executable_has_explicit_self_binding, program_assembly_index, program_u32_to_usize,
        validate_program_call_arg_count,
    },
    program_types::call_type_substitutions,
    runtime_ops::runtime_carrier_for_plan,
    source_context::program_source_context_frame,
    type_projection::EvalTypeProjection,
    *,
};
use crate::assembly_execution::{RuntimeAssemblyExecutionProjection, RuntimeExecutionProjection};
use crate::{RuntimeAssemblyEvalSeamError, RuntimeAssemblyEvalTarget};

mod execution_scope;
#[cfg(test)]
mod execution_scope_tests;
mod tail_call;

#[allow(unused_imports)]
pub(crate) use execution_scope::{ExecutionCheckpoint, ExecutionCheckpointKind};

pub struct ProgramExecutionInput<'a> {
    pub execution: ExecutionControl<'a>,
    pub config: ConfigCapabilityContext<'a>,
    pub db: DbCapabilityContext,
    pub file: FileCapabilityContext,
    pub file_source_stream: FileSourceStreamContext<'a>,
    pub time: TimeCapabilityContext<'a>,
    pub websocket: WebsocketCapabilityContext<'a>,
    pub effects: EffectDispatchContext,
    pub http_client: HttpClientCapabilityContext,
    pub test_effect_doubles: TestEffectDoubleContext,
    pub actor: ActorCapabilityContext<'a>,
    pub spawn: ActorCapabilityContext<'a>,
    pub request_heap_limits: RequestHeapLimits,
}

/// The exact operation that caused execution to enter another activation owner.
///
/// This is execution metadata, not a routing selector. The Host receives it only after Eval has
/// resolved an exact target from the generation-pinned assembly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationExecutionOperation {
    ServiceCall { operation_id: ContractOperationId },
    CallbackMethod { method_abi_id: String },
}

impl ActivationExecutionOperation {
    pub fn service_call(operation_id: ContractOperationId) -> Self {
        Self::ServiceCall { operation_id }
    }

    pub fn callback_method(method_abi_id: impl Into<String>) -> Self {
        Self::CallbackMethod {
            method_abi_id: method_abi_id.into(),
        }
    }
}

/// Owned deployment-scoped capabilities for one exact activation owner.
///
/// Construction happens outside Eval from an immutable, generation-pinned Host context set. The
/// bundle is deliberately indivisible: a failed rebind cannot leave config, DB, file, actor,
/// spawn, WebSocket, telemetry or HTTP effects owned by different deployments.
pub struct OwnedActivationExecutionCapabilityBundle {
    config: OwnedConfigCapabilityContext,
    db: DbCapabilityContext,
    file_source: FileCapabilitySource,
    websocket: OwnedWebsocketCapabilityContext,
    effects: EffectDispatchContext,
    http_client: HttpClientCapabilityContext,
    actor: OwnedActorCapabilityContext,
    spawn: OwnedActorCapabilityContext,
}

impl OwnedActivationExecutionCapabilityBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: OwnedConfigCapabilityContext,
        db: DbCapabilityContext,
        file_source: FileCapabilitySource,
        websocket: OwnedWebsocketCapabilityContext,
        effects: EffectDispatchContext,
        http_client: HttpClientCapabilityContext,
        actor: OwnedActorCapabilityContext,
        spawn: OwnedActorCapabilityContext,
    ) -> Self {
        Self {
            config,
            db,
            file_source,
            websocket,
            effects,
            http_client,
            actor,
            spawn,
        }
    }
}

/// Host-provided, generation-pinned activation capability lookup.
///
/// Implementations must resolve only inside the immutable context set that owns `target`. They
/// must not consult a current/latest activation pointer.
pub trait ActivationExecutionContextRebinder: Send + Sync {
    fn rebind(
        &self,
        target: &RuntimeAssemblyEvalTarget,
        operation: &ActivationExecutionOperation,
    ) -> Result<OwnedActivationExecutionCapabilityBundle>;
}

pub struct ProgramExecutionContext<'a> {
    execution: OwnedExecutionControl,
    execution_clock: execution_scope::ExecutionClock,
    config: ConfigCapabilityContext<'a>,
    db: DbCapabilityContext,
    file: FileCapabilityContext,
    file_source_stream: FileSourceStreamContext<'a>,
    time: TimeCapabilityContext<'a>,
    websocket: WebsocketCapabilityContext<'a>,
    activation_execution_context_rebinder: Option<Arc<dyn ActivationExecutionContextRebinder>>,
    effects: EffectDispatchContext,
    http_client: HttpClientCapabilityContext,
    test_effect_doubles: TestEffectDoubleContext,
    actor: ActorCapabilityContext<'a>,
    spawn: ActorCapabilityContext<'a>,
    request_heap_limits: RequestHeapLimits,
    runtime_assembly_target: Option<RuntimeAssemblyEvalTarget>,
    actor_execution_frame: Option<crate::actor_executor::ActorExecutionFrame>,
    exception_trace_id: Option<String>,
    exception_error_sequence: Arc<AtomicU64>,
    local_call_stack: Vec<ExceptionStackFrame>,
    program_call_depth: usize,
    _stream_runtime_owner: Option<StreamRuntimeOwner>,
}

/// Keeps language-level recursion below the native stack boundary of the Tokio
/// worker polling the evaluator futures.
///
/// Skiff calls are currently represented by nested `async_recursion` futures.
/// The instruction budget is intentionally much larger than a safe native call
/// stack, so it cannot be the only recursion guard.
const MAX_PROGRAM_CALL_DEPTH: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct PreparedTailCall {
    pub(crate) caller: ExecutableAddr,
    pub(crate) target: ExecutableAddr,
    pub(crate) env: Env,
    pub(crate) return_plan: Option<RuntimeTypePlan>,
    pub(crate) tail_site: InstructionSourceSite,
}

#[derive(Clone, Debug)]
pub(crate) enum EvaluatorControl {
    Complete(Flow),
    TailCall(Box<PreparedTailCall>),
}

impl EvaluatorControl {
    pub(crate) fn into_flow(self, owner: &str) -> Result<Flow> {
        match self {
            Self::Complete(flow) => Ok(flow),
            Self::TailCall(_) => Err(RuntimeError::InvalidArtifact(format!(
                "{owner} leaked an internal tail-call frame outside the evaluator trampoline"
            ))),
        }
    }
}

impl From<Flow> for EvaluatorControl {
    fn from(flow: Flow) -> Self {
        Self::Complete(flow)
    }
}

#[cfg(test)]
mod tail_call_internal_control_layout_tests {
    use super::{EvaluatorControl, Flow, PreparedTailCall};

    fn baseline_public_flow_discriminant(flow: Flow) -> u8 {
        match flow {
            Flow::Continue => 0,
            Flow::Return(_) => 1,
            Flow::Break => 2,
            Flow::LoopContinue => 3,
            Flow::Parked => 4,
            Flow::ContinueConsumer => 5,
        }
    }

    #[test]
    fn tail_call_internal_control_layout_keeps_prepared_frame_private_and_boxed() {
        assert_eq!(baseline_public_flow_discriminant(Flow::Continue), 0);
        assert!(
            std::mem::size_of::<EvaluatorControl>() < std::mem::size_of::<PreparedTailCall>(),
            "crate-private evaluator control must keep the prepared environment behind an owning indirection"
        );
    }
}

fn validate_activation_execution_capability_bundle(
    receiver: &ProgramExecutionContext<'_>,
    target: &RuntimeAssemblyEvalTarget,
    bundle: &OwnedActivationExecutionCapabilityBundle,
) -> Result<()> {
    let expected_service = target
        .activation_context()
        .identity()
        .deployment
        .service_id
        .as_str();
    let expected_version = target
        .activation_context()
        .identity()
        .deployment
        .contract_version
        .as_str();
    if bundle.actor.service_id() != expected_service
        || bundle.spawn.service_id() != expected_service
        || bundle.websocket.service_id() != expected_service
        || bundle.actor.service_version() != expected_version
        || bundle.spawn.service_version() != expected_version
        || bundle.actor.request_build_id()
            != target
                .activation_context()
                .implementation_package_build_id()
                .as_str()
        || bundle.spawn.request_build_id()
            != target
                .activation_context()
                .implementation_package_build_id()
                .as_str()
    {
        return Err(RuntimeError::InvalidArtifact(
            "activation execution capability bundle does not match the exact target owner"
                .to_string(),
        ));
    }
    if bundle.actor.runtime_id() != receiver.actor.runtime_id()
        || bundle.spawn.runtime_id() != receiver.spawn.runtime_id()
        || bundle.actor.request_id() != receiver.actor.request_id()
        || bundle.spawn.request_id() != receiver.spawn.request_id()
        || bundle.actor.trace_id() != receiver.actor.trace_id()
        || bundle.spawn.trace_id() != receiver.spawn.trace_id()
    {
        return Err(RuntimeError::InvalidArtifact(
            "activation owner switch changed request-wide execution identity".to_string(),
        ));
    }
    let target_identity = target.activation_context().identity();
    let expected_activation_identity =
        skiff_runtime_capability_context::ActivationIdentityControl {
            assembly_identity: target_identity.assembly_identity.clone(),
            generation: target_identity.assembly_generation,
            runtime_replica_id: target_identity.runtime_replica_id.clone(),
            deployment_revision: target_identity.deployment.deployment_revision.clone(),
        };
    if bundle.actor.activation_identity() != Some(&expected_activation_identity)
        || bundle.spawn.activation_identity() != Some(&expected_activation_identity)
    {
        return Err(RuntimeError::InvalidArtifact(
            "activation execution capability bundle is not pinned to the exact target activation"
                .to_string(),
        ));
    }
    Ok(())
}

impl<'a> Clone for ProgramExecutionContext<'a> {
    fn clone(&self) -> Self {
        Self {
            execution: self.execution.clone(),
            execution_clock: self.execution_clock.clone(),
            config: self.config.clone(),
            db: self.db.clone(),
            file: self.file.clone(),
            file_source_stream: self.file_source_stream.clone(),
            time: self.time.clone(),
            websocket: self.websocket.clone(),
            activation_execution_context_rebinder: self
                .activation_execution_context_rebinder
                .clone(),
            effects: self.effects.clone(),
            http_client: self.http_client.clone(),
            test_effect_doubles: self.test_effect_doubles.clone(),
            actor: self.actor.clone(),
            spawn: self.spawn.clone(),
            request_heap_limits: self.request_heap_limits.clone(),
            runtime_assembly_target: self.runtime_assembly_target.clone(),
            actor_execution_frame: self.actor_execution_frame.clone(),
            exception_trace_id: self.exception_trace_id.clone(),
            exception_error_sequence: self.exception_error_sequence.clone(),
            local_call_stack: self.local_call_stack.clone(),
            program_call_depth: self.program_call_depth,
            _stream_runtime_owner: self._stream_runtime_owner.clone(),
        }
    }
}

impl<'a> ProgramExecutionContext<'a> {
    pub fn new(input: ProgramExecutionInput<'a>) -> Self {
        let exception_trace_id = input
            .actor
            .trace_id()
            .filter(|trace_id| !trace_id.trim().is_empty())
            .map(str::to_string);
        Self {
            execution: input.execution.owned(),
            execution_clock: execution_scope::ExecutionClock::production(),
            config: input.config,
            db: input.db,
            file: input.file,
            file_source_stream: input.file_source_stream,
            time: input.time,
            websocket: input.websocket,
            activation_execution_context_rebinder: None,
            effects: input.effects,
            http_client: input.http_client,
            test_effect_doubles: input.test_effect_doubles,
            actor: input.actor,
            spawn: input.spawn,
            request_heap_limits: input.request_heap_limits,
            runtime_assembly_target: None,
            actor_execution_frame: None,
            exception_trace_id,
            exception_error_sequence: Arc::new(AtomicU64::new(0)),
            local_call_stack: Vec::new(),
            program_call_depth: 0,
            _stream_runtime_owner: None,
        }
    }

    /// Pins canonical execution to an admitted assembly and explicit request generation.
    pub fn with_runtime_assembly_target(mut self, target: RuntimeAssemblyEvalTarget) -> Self {
        let request_generation = target.request_activation().generation();
        let stream_runtime = self.stream_runtime();
        if stream_runtime.request_scope_generation() != Some(request_generation) {
            let (stream_runtime, owner) = stream_runtime.request_scope(request_generation);
            self.file_source_stream =
                FileSourceStreamContext::new(stream_runtime.clone(), self.execution());
            self.http_client = self.http_client.with_stream_runtime(stream_runtime);
            self._stream_runtime_owner = Some(owner);
        }
        self.runtime_assembly_target = Some(target);
        self
    }

    pub fn with_activation_execution_context_rebinder(
        mut self,
        rebinder: Arc<dyn ActivationExecutionContextRebinder>,
    ) -> Self {
        self.activation_execution_context_rebinder = Some(rebinder);
        self
    }

    /// Changes the deployment-owned execution capabilities as one fail-closed unit.
    ///
    /// `target` must already belong to the exact admitted assembly generation. The rebinder is
    /// installed by the Host from that same generation-pinned context set; Eval never asks it to
    /// select an activation or consult a latest pointer.
    pub fn switch_activation_owner(
        mut self,
        target: RuntimeAssemblyEvalTarget,
        operation: ActivationExecutionOperation,
    ) -> Result<Self> {
        let receiver = self.runtime_assembly_target()?;
        if receiver.request_activation().generation() != target.request_activation().generation()
            || receiver.execution_image().assembly_identity()
                != target.execution_image().assembly_identity()
        {
            return Err(RuntimeError::InvalidArtifact(
                "activation owner switch crossed its admitted assembly generation".to_string(),
            ));
        }
        target.ensure_execution_ready()?;
        let rebinder = self
            .activation_execution_context_rebinder
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "activation owner switch requires an execution context rebinder".to_string(),
                )
            })?;
        let bundle = rebinder.rebind(&target, &operation)?;
        validate_activation_execution_capability_bundle(&self, &target, &bundle)?;

        let OwnedActivationExecutionCapabilityBundle {
            config,
            db,
            file_source,
            websocket,
            effects,
            http_client,
            actor,
            spawn,
        } = bundle;
        self.config = config;
        self.db = db;
        self.file = file_source.context_for_request(self.db.clone());
        self.websocket = websocket;
        self.effects = effects;
        self.http_client = http_client;
        self.actor = actor;
        self.spawn = spawn;
        self.actor_execution_frame = None;
        reset_provider_local_stack(&mut self.local_call_stack);
        self = self.with_runtime_assembly_target(target);
        Ok(self)
    }

    pub(crate) fn with_actor_execution_frame(
        mut self,
        frame: crate::actor_executor::ActorExecutionFrame,
    ) -> Self {
        self.actor_execution_frame = Some(frame);
        self
    }

    pub(crate) fn with_local_call_site(mut self, site: InstructionSourceSite) -> Self {
        self.local_call_stack
            .push(ExceptionStackFrame::Local { site });
        self
    }

    fn enter_program_call(mut self) -> Result<Self> {
        let requested_depth = self.program_call_depth.saturating_add(1);
        if requested_depth > MAX_PROGRAM_CALL_DEPTH {
            return Err(RuntimeError::ResourceLimitExceeded {
                resource: "programCallDepth".to_string(),
                reason: "program call depth exceeds the runtime safety limit".to_string(),
                limit: MAX_PROGRAM_CALL_DEPTH,
                current: self.program_call_depth,
                requested_delta: 1,
            });
        }
        self.program_call_depth = requested_depth;
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn with_program_call_depth_for_test(mut self, depth: usize) -> Self {
        self.program_call_depth = depth;
        self
    }

    #[cfg(test)]
    pub(crate) fn program_call_depth_for_test(&self) -> usize {
        self.program_call_depth
    }

    pub(crate) fn exception_stack_for_site(
        &self,
        site: InstructionSourceSite,
    ) -> Vec<ExceptionStackFrame> {
        let mut stack = self.local_call_stack.clone();
        stack.push(ExceptionStackFrame::Local { site });
        stack
    }

    pub(crate) fn next_exception_correlation(&self) -> Result<ErrorCorrelation> {
        let trace_id = self.exception_trace_id.clone().ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "request-local exception requires a non-empty request trace id".to_string(),
            )
        })?;
        let sequence = self
            .exception_error_sequence
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "request-local exception error-id sequence overflowed".to_string(),
                )
            })?;
        Ok(ErrorCorrelation {
            error_id: format!("{trace_id}:local-error:{sequence}"),
            trace_id,
        })
    }

    pub(crate) fn actor_execution_frame(
        &self,
    ) -> Option<&crate::actor_executor::ActorExecutionFrame> {
        self.actor_execution_frame.as_ref()
    }

    pub fn execution(&self) -> ExecutionControl<'static> {
        execution_scope::borrow_owned_execution_control(&self.execution)
    }

    pub fn config_context(&self) -> ConfigCapabilityContext<'a> {
        self.config.clone()
    }

    pub fn db_context(&self) -> DbCapabilityContext {
        self.db.clone()
    }

    pub fn file_context(&self) -> FileCapabilityContext {
        self.file.clone()
    }

    pub fn file_source_stream_context(&self) -> FileSourceStreamContext<'a> {
        self.file_source_stream.clone()
    }

    pub fn time_context(&self) -> TimeCapabilityContext<'a> {
        self.time.clone()
    }

    pub fn websocket_context(&self) -> WebsocketCapabilityContext<'a> {
        self.websocket.clone()
    }

    pub fn telemetry_context(&self) -> TelemetryCapabilityContext {
        self.effects.telemetry_context()
    }

    pub fn http_client_context(&self) -> HttpClientCapabilityContext {
        self.http_client.clone()
    }

    pub fn test_effect_double_context(&self) -> TestEffectDoubleContext {
        self.test_effect_doubles.clone()
    }

    pub fn actor_context(&self) -> ActorCapabilityContext<'a> {
        self.actor.clone()
    }

    pub fn spawn_context(&self) -> ActorCapabilityContext<'a> {
        self.spawn.clone()
    }

    pub fn request_heap(&self) -> RequestHeap {
        RequestHeap::new(self.request_heap_limits.clone())
    }

    pub fn request_heap_limits(&self) -> RequestHeapLimits {
        self.request_heap_limits.clone()
    }

    pub fn stream_runtime(&self) -> StreamRuntime {
        self.file_source_stream.stream_runtime_handle()
    }

    pub(crate) fn take_stream_runtime_owner(&mut self) -> Option<StreamRuntimeOwner> {
        self._stream_runtime_owner.take()
    }

    pub(crate) fn stream_runtime_owner(&self) -> Option<StreamRuntimeOwner> {
        self._stream_runtime_owner.clone()
    }

    pub fn runtime_assembly_target(
        &self,
    ) -> std::result::Result<&RuntimeAssemblyEvalTarget, RuntimeAssemblyEvalSeamError> {
        self.runtime_assembly_target
            .as_ref()
            .ok_or(RuntimeAssemblyEvalSeamError::MissingExecutionTarget)
    }

    pub(crate) fn runtime_assembly_target_if_present(&self) -> Option<&RuntimeAssemblyEvalTarget> {
        self.runtime_assembly_target.as_ref()
    }
}

pub(crate) fn reset_provider_local_stack(stack: &mut Vec<ExceptionStackFrame>) {
    stack.clear();
}

#[cfg(test)]
mod provider_service_stack_scope_tests {
    use super::*;
    use skiff_artifact_model::SyntheticInstructionSiteReason;

    #[test]
    fn provider_scope_clears_only_local_stack_and_keeps_shared_sequence() {
        let sequence = Arc::new(AtomicU64::new(7));
        let same_request_sequence = Arc::clone(&sequence);
        let mut stack = vec![ExceptionStackFrame::Local {
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        }];

        reset_provider_local_stack(&mut stack);

        assert!(stack.is_empty());
        assert!(Arc::ptr_eq(&sequence, &same_request_sequence));
        assert_eq!(same_request_sequence.fetch_add(1, Ordering::Relaxed), 7);
    }
}

/// Owned, `'static` mirror of every borrow held by [`ProgramExecutionContext`].
///
/// A `ProgramExecutionContext<'a>` borrows almost entirely from data that lives
/// for the whole request inside service-level and per-request `Arc`s; the
/// borrows are just convenient views. To run a stream producer in
/// its own `tokio::spawn`ed task (so native stack depth stays constant no matter
/// how deeply stream producers nest) the producer future must be `Send +
/// 'static`, which a borrowed context can never be. This struct holds owned/
/// `Arc` copies of that underlying data, and [`OwnedProgramExecutionContext::borrow`]
/// reconstructs a borrowed `ProgramExecutionContext<'_>` from it. Wrap it in an
/// `Arc` and clone the `Arc` into each spawned task.
///
/// The owned `actor` strings are shared by both the actor and spawn contexts —
/// they are identical at the construction site (`runner.rs`).
pub struct OwnedProgramExecutionContext {
    execution: OwnedExecutionControl,
    execution_clock: execution_scope::ExecutionClock,
    config: OwnedConfigCapabilityContext,
    db: DbCapabilityContext,
    file_source: FileCapabilitySource,
    stream_runtime: StreamRuntime,
    _stream_runtime_owner: Option<StreamRuntimeOwner>,
    websocket: OwnedWebsocketCapabilityContext,
    activation_execution_context_rebinder: Option<Arc<dyn ActivationExecutionContextRebinder>>,
    effects: EffectDispatchContext,
    http_client: HttpClientCapabilityContext,
    test_effect_doubles: TestEffectDoubleContext,
    actor: OwnedActorCapabilityContext,
    spawn: OwnedActorCapabilityContext,
    request_heap_limits: RequestHeapLimits,
    runtime_assembly_target: Option<RuntimeAssemblyEvalTarget>,
    exception_trace_id: Option<String>,
    exception_error_sequence: Arc<AtomicU64>,
    local_call_stack: Vec<ExceptionStackFrame>,
    program_call_depth: usize,
}

impl OwnedProgramExecutionContext {
    /// Captures owned copies of everything `context` borrows so the resulting
    /// value can outlive the original borrow scope (e.g. inside a spawned task).
    pub fn capture(context: &ProgramExecutionContext<'_>) -> Self {
        let execution = context.execution.clone();
        let actor = context.actor.clone();
        Self {
            execution,
            execution_clock: context.execution_clock.clone(),
            config: ConfigCapabilityContext::owned(&context.config),
            db: context.db.clone(),
            file_source: context.file.source(),
            stream_runtime: context.file_source_stream.stream_runtime_handle(),
            _stream_runtime_owner: context.stream_runtime_owner(),
            websocket: context.websocket.owned(),
            activation_execution_context_rebinder: context
                .activation_execution_context_rebinder
                .clone(),
            effects: context.effects.clone(),
            http_client: context.http_client.clone(),
            test_effect_doubles: context.test_effect_doubles.clone(),
            actor: actor.owned(),
            spawn: context.spawn.owned(),
            request_heap_limits: context.request_heap_limits.clone(),
            runtime_assembly_target: context.runtime_assembly_target.clone(),
            exception_trace_id: context.exception_trace_id.clone(),
            exception_error_sequence: context.exception_error_sequence.clone(),
            local_call_stack: context.local_call_stack.clone(),
            program_call_depth: context.program_call_depth,
        }
    }

    /// Reconstructs a borrowed execution context that views this owned data.
    pub fn borrow(&self) -> ProgramExecutionContext<'_> {
        let execution = execution_scope::borrow_owned_execution_control(&self.execution);
        let config = self.config.borrow();
        let file = self.file_source.context_for_request(self.db.clone());
        let file_source_stream =
            FileSourceStreamContext::new(self.stream_runtime.clone(), execution.clone());
        let time = TimeCapabilityContext::new(execution.clone());
        let websocket = self.websocket.borrow();
        let actor = self.actor.borrow();
        let spawn = self.spawn.borrow();
        let mut context = ProgramExecutionContext::new(ProgramExecutionInput {
            execution,
            config,
            db: self.db.clone(),
            file,
            file_source_stream,
            time,
            websocket,
            effects: self.effects.clone(),
            http_client: self.http_client.clone(),
            test_effect_doubles: self.test_effect_doubles.clone(),
            actor,
            spawn,
            request_heap_limits: self.request_heap_limits.clone(),
        });
        context.execution_clock = self.execution_clock.clone();
        context.exception_trace_id = self.exception_trace_id.clone();
        context.activation_execution_context_rebinder =
            self.activation_execution_context_rebinder.clone();
        context.exception_error_sequence = self.exception_error_sequence.clone();
        context.local_call_stack = self.local_call_stack.clone();
        context.program_call_depth = self.program_call_depth;
        let mut context = match &self.runtime_assembly_target {
            Some(target) => context.with_runtime_assembly_target(target.clone()),
            None => context,
        };
        context._stream_runtime_owner = self._stream_runtime_owner.clone();
        context
    }

    /// Reconstructs a context for work that starts at an independent scheduler
    /// boundary. The new task has a fresh native stack, so call-depth accounting
    /// must not inherit the parent task's active stack frames.
    pub(crate) fn borrow_for_scheduled_task(&self) -> ProgramExecutionContext<'_> {
        let mut context = self.borrow();
        context.program_call_depth = 0;
        context
    }
}

pub trait IntoProgramExecutionContext<'a> {
    fn into_program_execution_context(self) -> ProgramExecutionContext<'a>;
}

impl<'a> IntoProgramExecutionContext<'a> for ProgramExecutionContext<'a> {
    fn into_program_execution_context(self) -> ProgramExecutionContext<'a> {
        self
    }
}

pub struct ExecutableInvocation<'a> {
    program: EvalProgramProjection<'a>,
    pub addr: &'a ExecutableAddr,
    pub file: &'a LinkedFileUnit,
    pub executable: &'a LinkedExecutable,
    pub explicit_self_param: bool,
}

impl<'a> ExecutableInvocation<'a> {
    pub fn from_eval_invocation(invocation: EvalInvocation<'a>) -> Self {
        let executable_body = invocation.executable_body();
        Self {
            program: invocation.program_projection(),
            addr: invocation.addr(),
            file: executable_body.file(),
            executable: executable_body.executable(),
            explicit_self_param: executable_body.explicit_self_param(),
        }
    }

    pub fn resolve(interpreter: &'a Interpreter, addr: &'a ExecutableAddr) -> Result<Self> {
        let program = interpreter.program_projection()?;
        let resolved = program.resolve_executable(addr)?;
        Ok(Self {
            program,
            addr,
            file: resolved.file,
            executable: resolved.executable,
            explicit_self_param: executable_has_explicit_self_binding(resolved.executable),
        })
    }

    pub fn program_projection(&self) -> EvalProgramProjection<'a> {
        self.program
    }

    pub fn validate_arg_count(&self, arg_count: usize) -> Result<()> {
        let expected_args = if self.explicit_self_param {
            arg_count.saturating_add(1)
        } else {
            arg_count
        };
        validate_program_call_arg_count(self.executable, expected_args)
    }

    pub fn validate_raw_arg_count(&self, arg_count: usize) -> Result<()> {
        validate_program_call_arg_count(self.executable, arg_count)
    }

    fn accepts_separate_self_argument_without_self_param(&self, arg_count: usize) -> bool {
        matches!(self.executable.kind, ExecutableKind::ImplMethod)
            && self.executable.self_type.is_some()
            && !self.explicit_self_param
            && arg_count == self.executable.params.len() + 1
    }

    pub fn env(&self) -> Result<Env> {
        Env::for_program_executable(
            self.executable,
            Some(self.file.module_path.clone()),
            program_assembly_index(self.addr),
        )
    }

    pub fn env_for_call(
        &self,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        type_args: &BTreeMap<String, LinkedTypeRef>,
    ) -> Result<Env> {
        let mut env = self.env()?;
        env.inherit_stream_consumer_supervision_from(caller_env);
        env.stream_sink = caller_env.stream_sink.clone();
        env.current_stream_item_type = caller_env.current_stream_item_type.clone();
        env.response_stream_sink = caller_env.response_stream_sink.clone();
        env.type_substitutions = call_type_substitutions(
            self.program_projection().type_view(),
            caller_addr,
            &caller_env.type_substitutions,
            self.executable,
            type_args,
        );
        Ok(env)
    }

    pub fn declare_self(&self, env: &mut Env, self_value: RuntimeValueCarrier) -> Result<()> {
        if self.explicit_self_param {
            env.declare_program_parameter(self.executable, "self", self_value)?;
        } else {
            env.declare_program_self(self.executable, self_value)?;
        }
        Ok(())
    }

    pub fn declare_args(&self, env: &mut Env, args: &[RuntimeValueCarrier]) -> Result<()> {
        for (index, parameter) in self
            .executable
            .params
            .iter()
            .skip(usize::from(self.explicit_self_param))
            .enumerate()
        {
            env.declare_program_parameter(
                self.executable,
                &parameter.name,
                args.get(index)
                    .cloned()
                    .unwrap_or_else(|| RuntimeValue::Null.into()),
            )?;
        }
        Ok(())
    }

    pub async fn exec<'ctx>(
        &self,
        interpreter: &Interpreter,
        context: impl IntoProgramExecutionContext<'ctx> + Send,
        heap: &mut RequestHeap,
        env: &mut Env,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        interpreter
            .exec_program_executable(context, heap, env, self.addr, self.file, self.executable)
            .await
    }
}

struct AssemblyExecutableInvocation<'a> {
    projection: &'a RuntimeAssemblyExecutionProjection,
    addr: ExecutableAddr,
    file: &'a LinkedFileUnit,
    executable: &'a LinkedExecutable,
    explicit_self_param: bool,
}

impl<'a> AssemblyExecutableInvocation<'a> {
    fn resolve(
        projection: &'a RuntimeAssemblyExecutionProjection,
        addr: &ExecutableAddr,
    ) -> Result<Self> {
        let resolved = projection.resolve_nested_executable(addr)?;
        Ok(Self {
            projection,
            addr: resolved.addr,
            file: resolved.file.as_ref(),
            executable: resolved.executable,
            explicit_self_param: executable_has_explicit_self_binding(resolved.executable),
        })
    }

    fn validate_arg_count(&self, arg_count: usize) -> Result<()> {
        let expected_args = if self.explicit_self_param {
            arg_count.saturating_add(1)
        } else {
            arg_count
        };
        validate_program_call_arg_count(self.executable, expected_args)
    }

    fn validate_raw_arg_count(&self, arg_count: usize) -> Result<()> {
        validate_program_call_arg_count(self.executable, arg_count)
    }

    fn accepts_separate_self_argument_without_self_param(&self, arg_count: usize) -> bool {
        matches!(self.executable.kind, ExecutableKind::ImplMethod)
            && self.executable.self_type.is_some()
            && !self.explicit_self_param
            && arg_count == self.executable.params.len() + 1
    }

    fn env(&self) -> Result<Env> {
        Env::for_program_executable(
            self.executable,
            Some(self.file.module_path.clone()),
            program_assembly_index(&self.addr),
        )
    }

    fn env_for_call(
        &self,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        type_args: &BTreeMap<String, LinkedTypeRef>,
    ) -> Result<Env> {
        let mut env = self.env()?;
        env.inherit_stream_consumer_supervision_from(caller_env);
        env.stream_sink = caller_env.stream_sink.clone();
        env.current_stream_item_type = caller_env.current_stream_item_type.clone();
        env.response_stream_sink = caller_env.response_stream_sink.clone();
        env.type_substitutions = call_type_substitutions(
            self.projection.type_view(),
            caller_addr,
            &caller_env.type_substitutions,
            self.executable,
            type_args,
        );
        Ok(env)
    }

    fn declare_self(&self, env: &mut Env, self_value: RuntimeValueCarrier) -> Result<()> {
        if self.explicit_self_param {
            env.declare_program_parameter(self.executable, "self", self_value)?;
        } else {
            env.declare_program_self(self.executable, self_value)?;
        }
        Ok(())
    }

    fn declare_args(&self, env: &mut Env, args: &[RuntimeValueCarrier]) -> Result<()> {
        for (index, parameter) in self
            .executable
            .params
            .iter()
            .skip(usize::from(self.explicit_self_param))
            .enumerate()
        {
            env.declare_program_parameter(
                self.executable,
                &parameter.name,
                args.get(index)
                    .cloned()
                    .unwrap_or_else(|| RuntimeValue::Null.into()),
            )?;
        }
        Ok(())
    }
}

fn materialize_local_callable_return(
    projection: RuntimeExecutionProjection<'_>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
    env: &Env,
    value: RuntimeValueCarrier,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let Some(return_type) = executable.return_type.as_ref() else {
        return Ok(value);
    };
    let plan = EvalTypeProjection::from_execution_projection(projection)
        .plan_from_linked_nested_ref_with_substitutions(
            return_type,
            addr,
            &env.type_substitutions,
        )?;
    runtime_carrier_for_plan(
        value,
        &plan,
        &format!("local callable {} return", executable.symbol),
        heap,
    )
}

impl Interpreter {
    /// Executes a canonical assembly address through the same nested-invocation path used by
    /// package/service/callback lanes. The supplied context must already be pinned to a
    /// [`RuntimeAssemblyEvalTarget`]; absence is a structured error and never selects legacy.
    pub async fn execute_runtime_assembly_addr(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        addr: &ExecutableAddr,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        context.runtime_assembly_target()?;
        self.call_program_executable(
            context,
            heap,
            &Env::new(),
            addr,
            addr,
            &BTreeMap::new(),
            args,
        )
        .await
    }

    /// Executes an exact assembly callable while retaining the existing deferred stream-producer
    /// scheduler used by local calls. HTTP gateway server streams consume the returned handle
    /// through the shared stream cleanup path.
    pub async fn execute_runtime_assembly_addr_with_stream_defer(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        addr: &ExecutableAddr,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        context.runtime_assembly_target()?;
        self.call_program_executable_with_self(
            context,
            heap,
            &Env::new(),
            addr,
            addr,
            &BTreeMap::new(),
            RuntimeValue::Null,
            args,
        )
        .await
    }

    pub fn program_projection(&self) -> Result<EvalProgramProjection<'_>> {
        let program = self.program.as_ref().ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "assembly interpreter has no legacy RuntimeProgram projection".to_string(),
            )
        })?;
        Ok(EvalProgramProjection::new_with_resources(
            &program.service_id,
            &program.service_files,
            &program.packages,
            &program.service_resources,
            &program.spawn_routes,
            &program.link_overlay,
            &program.types,
        ))
    }

    pub async fn call_program_executable(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        self.call_program_executable_carriers(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            args.into_iter().map(Into::into).collect(),
        )
        .await
        .map(RuntimeValueCarrier::into_value)
    }

    #[async_recursion]
    pub(crate) async fn call_program_executable_carriers(
        &self,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        args: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.enter_program_call()?;
        context.execution().add_instruction_units(1)?;
        context.execution().poll_execution_budget()?;

        if let Some(projection) = context
            .runtime_assembly_target_if_present()
            .map(RuntimeAssemblyEvalTarget::execution_projection)
        {
            return self
                .call_assembly_executable(
                    &context,
                    projection,
                    heap,
                    caller_env,
                    caller_addr,
                    addr,
                    type_args,
                    args,
                )
                .await;
        }

        let invocation = ExecutableInvocation::resolve(self, addr)?;
        let has_separate_self_arg =
            invocation.accepts_separate_self_argument_without_self_param(args.len());
        if !has_separate_self_arg {
            invocation.validate_raw_arg_count(args.len())?;
        }

        let mut env = invocation.env_for_call(caller_env, caller_addr, type_args)?;
        let (self_value, args) = if invocation.explicit_self_param || has_separate_self_arg {
            let Some((self_value, args)) = args.split_first() else {
                return Err(RuntimeError::Decode(format!(
                    "callable {} missing self argument",
                    invocation.executable.symbol
                )));
            };
            (self_value.clone(), args)
        } else {
            (
                caller_env
                    .self_value()
                    .unwrap_or_else(|| RuntimeValue::Null.into()),
                args.as_slice(),
            )
        };
        invocation.declare_self(&mut env, self_value)?;
        invocation.declare_args(&mut env, args)?;

        let flow = invocation
            .exec(self, context.clone(), heap, &mut env)
            .await?;
        context.execution().poll_execution_budget()?;
        let value = if context.actor_execution_frame().is_some() {
            FlowCompletionPolicy::actor_callable_value(flow, &invocation.executable.symbol)
        } else {
            FlowCompletionPolicy::callable_value(flow, &invocation.executable.symbol)
        }?;
        materialize_local_callable_return(
            RuntimeExecutionProjection::Legacy(invocation.program_projection()),
            invocation.addr,
            invocation.executable,
            &env,
            value,
            heap,
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_assembly_executable(
        &self,
        context: &ProgramExecutionContext<'_>,
        projection: &RuntimeAssemblyExecutionProjection,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &BTreeMap<String, LinkedTypeRef>,
        args: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        let invocation = AssemblyExecutableInvocation::resolve(projection, addr)?;
        let has_separate_self_arg =
            invocation.accepts_separate_self_argument_without_self_param(args.len());
        if !has_separate_self_arg {
            invocation.validate_raw_arg_count(args.len())?;
        }
        let mut env = invocation.env_for_call(caller_env, caller_addr, type_args)?;
        let (self_value, args) = if invocation.explicit_self_param || has_separate_self_arg {
            let Some((self_value, args)) = args.split_first() else {
                return Err(RuntimeError::Decode(format!(
                    "callable {} missing self argument",
                    invocation.executable.symbol
                )));
            };
            (self_value.clone(), args)
        } else {
            (
                caller_env
                    .self_value()
                    .unwrap_or_else(|| RuntimeValue::Null.into()),
                args.as_slice(),
            )
        };
        invocation.declare_self(&mut env, self_value)?;
        invocation.declare_args(&mut env, args)?;
        let flow = self
            .exec_program_executable(
                context.clone(),
                heap,
                &mut env,
                &invocation.addr,
                invocation.file,
                invocation.executable,
            )
            .await?;
        context.execution().poll_execution_budget()?;
        let value = if context.actor_execution_frame().is_some() {
            FlowCompletionPolicy::actor_callable_value(flow, &invocation.executable.symbol)
        } else {
            FlowCompletionPolicy::callable_value(flow, &invocation.executable.symbol)
        }?;
        materialize_local_callable_return(
            RuntimeExecutionProjection::Assembly(projection.clone()),
            &invocation.addr,
            invocation.executable,
            &env,
            value,
            heap,
        )
    }

    pub async fn call_program_executable_with_self(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        self.call_program_executable_with_self_carriers(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            self_value.into(),
            args.into_iter().map(Into::into).collect(),
        )
        .await
        .map(RuntimeValueCarrier::into_value)
    }

    pub(crate) async fn call_program_executable_with_self_carriers(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValueCarrier,
        args: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        self.call_program_executable_with_self_inner(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            self_value,
            args,
            true,
        )
        .await
    }

    pub async fn call_program_executable_with_self_direct(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        self.call_program_executable_with_self_direct_carriers(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            self_value.into(),
            args.into_iter().map(Into::into).collect(),
        )
        .await
        .map(RuntimeValueCarrier::into_value)
    }

    pub(crate) async fn call_program_executable_with_self_direct_carriers(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValueCarrier,
        args: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        self.call_program_executable_with_self_inner(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            self_value,
            args,
            false,
        )
        .await
    }

    #[async_recursion]
    async fn call_program_executable_with_self_inner(
        &self,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValueCarrier,
        args: Vec<RuntimeValueCarrier>,
        allow_stream_defer: bool,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.enter_program_call()?;
        context.execution().add_instruction_units(1)?;
        context.execution().poll_execution_budget()?;

        if let Some(projection) = context
            .runtime_assembly_target_if_present()
            .map(RuntimeAssemblyEvalTarget::execution_projection)
        {
            let invocation = AssemblyExecutableInvocation::resolve(projection, addr)?;
            invocation.validate_arg_count(args.len())?;
            if allow_stream_defer {
                if let Some(value) = self
                    .prepare_deferred_stream_producer_from_values(
                        RuntimeExecutionProjection::Assembly(projection.clone()),
                        context.clone(),
                        heap,
                        caller_env,
                        caller_addr,
                        &invocation.addr,
                        invocation.executable,
                        type_args,
                        self_value.clone(),
                        args.clone(),
                    )
                    .await?
                {
                    context.execution().poll_execution_budget()?;
                    return Ok(value.into());
                }
            }
            let mut env = invocation.env_for_call(caller_env, caller_addr, type_args)?;
            invocation.declare_self(&mut env, self_value)?;
            invocation.declare_args(&mut env, &args)?;
            let flow = self
                .exec_program_executable(
                    context.clone(),
                    heap,
                    &mut env,
                    &invocation.addr,
                    invocation.file,
                    invocation.executable,
                )
                .await?;
            context.execution().poll_execution_budget()?;
            let value = FlowCompletionPolicy::callable_value(flow, &invocation.executable.symbol)?;
            return materialize_local_callable_return(
                RuntimeExecutionProjection::Assembly(projection.clone()),
                &invocation.addr,
                invocation.executable,
                &env,
                value,
                heap,
            );
        }

        let invocation = ExecutableInvocation::resolve(self, addr)?;
        invocation.validate_arg_count(args.len())?;

        if allow_stream_defer {
            if let Some(value) = self
                .prepare_deferred_stream_producer_from_values(
                    RuntimeExecutionProjection::Legacy(self.program_projection()?),
                    context.clone(),
                    heap,
                    caller_env,
                    caller_addr,
                    addr,
                    invocation.executable,
                    type_args,
                    self_value.clone(),
                    args.clone(),
                )
                .await?
            {
                context.execution().poll_execution_budget()?;
                return Ok(value.into());
            }
        }

        let mut env = invocation.env_for_call(caller_env, caller_addr, type_args)?;
        invocation.declare_self(&mut env, self_value)?;
        invocation.declare_args(&mut env, &args)?;

        let flow = invocation
            .exec(self, context.clone(), heap, &mut env)
            .await?;
        context.execution().poll_execution_budget()?;
        let value = FlowCompletionPolicy::callable_value(flow, &invocation.executable.symbol)?;
        materialize_local_callable_return(
            RuntimeExecutionProjection::Legacy(invocation.program_projection()),
            invocation.addr,
            invocation.executable,
            &env,
            value,
            heap,
        )
    }

    pub async fn exec_program_executable<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        let projection = RuntimeExecutionProjection::for_context(self, &context)?;
        let mut control = {
            let mut eval = EvalContext::new_callable_with_projection(
                self,
                projection.clone(),
                context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
            );
            let control = eval.exec_program_executable_control().await?;
            if let EvaluatorControl::TailCall(prepared) = &control {
                eval.account_tail_transfer(&prepared.tail_site)?;
            }
            control
        };

        loop {
            let EvaluatorControl::TailCall(prepared) = control else {
                return control.into_flow("evaluator trampoline");
            };
            let PreparedTailCall {
                caller,
                target: prepared_target,
                env: mut prepared_env,
                return_plan: _,
                tail_site,
            } = *prepared;
            let resolved = promote_call_site_error(
                &projection,
                &context,
                heap,
                &caller,
                projection.resolve_nested_executable(&prepared_target),
                &tail_site,
            )?;
            let target = resolved.addr.clone();
            control = {
                let mut eval = EvalContext::new_callable_with_projection(
                    self,
                    projection.clone(),
                    context.clone(),
                    heap,
                    &mut prepared_env,
                    &target,
                    resolved.file,
                    resolved.executable,
                );
                let control = eval.exec_tail_entry_control(&caller, &tail_site).await?;
                if let EvaluatorControl::TailCall(next) = &control {
                    eval.account_tail_transfer(&next.tail_site)?;
                }
                control
            };
        }
    }

    pub async fn eval_program_const<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        const_index: u32,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.into_program_execution_context();
        let const_index = program_u32_to_usize(const_index, "const ref")?;
        self.eval_program_const_in_file(context, heap, caller_env, addr, file, const_index)
            .await
    }

    pub async fn eval_program_const_addr<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        const_addr: &ConstAddr,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.into_program_execution_context();
        let projection = RuntimeExecutionProjection::for_context(self, &context)?;
        let resolved = projection.resolve_const(const_addr)?;
        let addr = ExecutableAddr {
            unit: const_addr.unit.clone(),
            file: const_addr.file.clone(),
            executable: 0,
        };
        self.eval_program_const_in_file(
            context,
            heap,
            caller_env,
            &addr,
            resolved.file,
            const_addr.const_index,
        )
        .await
    }

    async fn eval_program_const_in_file<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        const_index: usize,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.into_program_execution_context();
        let constant = file.constants.get(const_index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!("RuntimeProgram const {const_index} is missing"))
        })?;
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: constant.name.clone(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(constant.ty.clone()),
            self_type: None,
            slots: Default::default(),
            may_suspend: false,
            body: constant.body.clone(),
        };
        let mut env = Env::for_program_executable(
            &executable,
            Some(file.module_path.clone()),
            program_assembly_index(addr),
        )?;
        env.inherit_stream_consumer_supervision_from(caller_env);
        env.stream_sink = caller_env.stream_sink.clone();
        env.current_stream_item_type = caller_env.current_stream_item_type.clone();
        env.response_stream_sink = caller_env.response_stream_sink.clone();
        env.type_substitutions = caller_env.type_substitutions.clone();
        let projection = RuntimeExecutionProjection::for_context(self, &context)?;
        let flow = self
            .exec_program_executable(context, heap, &mut env, addr, file, &executable)
            .await?;
        let value = FlowCompletionPolicy::const_value(flow, &constant.name)?;
        materialize_local_callable_return(projection, addr, &executable, &env, value, heap)
    }

    pub async fn exec_program_block<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        label: &str,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)?
            .exec_program_block(label)
            .await
    }

    async fn exec_program_statement<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        statement: &LinkedStmtIr,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)?
            .exec_program_statement(statement)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn exec_program_for_in_body<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        item_slot: usize,
        body: &str,
        item_value: RuntimeValue,
    ) -> Result<Flow> {
        self.exec_program_for_in_body_carrier(
            context,
            heap,
            env,
            addr,
            file,
            executable,
            item_slot,
            body,
            item_value.into(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn exec_program_for_in_body_carrier<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        item_slot: usize,
        body: &str,
        item_value: RuntimeValueCarrier,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)?
            .exec_program_for_in_body(item_slot, body, item_value)
            .await
    }

    pub async fn eval_program_expr_ref<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        expr_ref: ExprRefIr,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)?
            .eval_program_expr_ref(expr_ref)
            .await
    }

    async fn eval_program_expr<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        expr: &LinkedExprIr,
    ) -> Result<RuntimeValueCarrier> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)?
            .eval_program_expr(expr)
            .await
    }

    pub fn eval_program_rethrow_slot(
        &self,
        env: &Env,
        exception_slot: usize,
        heap: &RequestHeap,
    ) -> Result<Flow> {
        let exception = env.get_slot(exception_slot)?;
        let exception = exceptions::request_exception_for_rethrow(&exception, heap)?;
        Err(RuntimeError::UserException(UserException::new(exception)))
    }

    pub fn attach_program_source_context(
        &self,
        error: RuntimeError,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        source_id: Option<u64>,
    ) -> RuntimeError {
        let Some(source_id) = source_id else {
            return error;
        };
        let frame = program_source_context_frame(addr, file, source_id);
        attach_source_frame(error, source_id, frame)
    }
}
