use std::{collections::HashMap, sync::Arc};

use skiff_runtime_activation::{ActivationContext, RequestActivationContext};
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};
use skiff_runtime_linked_program::AssemblyExecutionImage;
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_request::{
    ExecutionBudget, OutboundRequestRegistry, RequestEnvelope, RuntimeOperation,
};
use tokio::sync::mpsc;

use super::*;
use crate::capability_context::actor_method_outbound::ActorMethodOutboundRegistry;

/// All host-owned state needed to execute an Actor method against one committed
/// assembly activation. Callers must obtain `activation`, `execution_image`, and
/// `resolver` from the same immutable `ActiveAssembly` snapshot.
pub(crate) struct ActorMethodEvalExecutionInput {
    pub(crate) runtime_id: String,
    pub(crate) invocation_id: String,
    pub(crate) service_protocol_identity: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) execution_image: Arc<AssemblyExecutionImage>,
    pub(crate) resolver: Arc<dyn RuntimeAssemblyEvalResolver>,
    pub(crate) db_source: concrete::DbCapabilitySource,
    pub(crate) file_source: concrete::FileCapabilitySource,
    pub(crate) http_options: concrete::HttpRuntimeOptions,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    pub(crate) telemetry_context: Option<RequestTelemetryContext>,
    pub(crate) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(crate) http_response_max_bytes: usize,
    pub(crate) cancellation: CancellationToken,
    pub(crate) execution_budget: Arc<ExecutionBudget>,
    pub(crate) request_heap_limits: RequestHeapLimits,
}

/// Owned backing for the borrowed eval context consumed by
/// `skiff_runtime_eval::actor_executor::ActorMethodExecutor`.
pub(crate) struct ActorMethodEvalExecution {
    interpreter: Interpreter,
    runtime_id: String,
    activation: Arc<ActivationContext>,
    execution_image: Arc<AssemblyExecutionImage>,
    resolver: Arc<dyn RuntimeAssemblyEvalResolver>,
    activation_identity: ActivationIdentityControl,
    config: crate::config_view::RuntimeConfigView,
    package_configs: Vec<crate::config_view::RuntimeConfigView>,
    db_source: concrete::DbCapabilitySource,
    file_source: concrete::FileCapabilitySource,
    http_options: concrete::HttpRuntimeOptions,
    outbound_requests: Arc<OutboundRequestRegistry>,
    actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    telemetry_context: Option<RequestTelemetryContext>,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    http_response_max_bytes: usize,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    request_heap_limits: RequestHeapLimits,
    request: RequestEnvelope,
    operation: RuntimeOperation,
}

impl ActorMethodEvalExecution {
    pub(crate) fn new(input: ActorMethodEvalExecutionInput) -> anyhow::Result<Self> {
        if input.invocation_id.trim().is_empty() {
            anyhow::bail!("Actor method invocation id must be non-empty");
        }
        let deployment = &input.activation.identity().deployment;
        let activation_identity = activation_identity_control(input.activation.as_ref());
        let config = crate::config_view::RuntimeConfigView::from_activation_literals(
            &input.activation.owned_bindings().config_literals,
        )?;
        let package_configs = super::assembly_request_adapter::package_config_views(
            input.execution_image.as_ref(),
            input.activation.implementation_package_build_id(),
            &input.activation.owned_bindings().config_literals,
        )?;
        let target = "actor.method".to_string();
        let request = RequestEnvelope {
            request_id: input.invocation_id,
            mode: "unary".to_string(),
            target: target.clone(),
            operation_abi_id: None,
            selector: None,
            service_id: Some(deployment.service_id.clone()),
            build_id: input
                .activation
                .implementation_package_build_id()
                .to_string(),
            service_protocol_identity: input.service_protocol_identity,
            contract_identity: None,
            activation_identity: Some(input.activation.activation_id().as_str().to_string()),
            ingress_selector: None,
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let operation = RuntimeOperation {
            operation_abi_id: None,
            operation: target.clone(),
            target,
            mode: "unary".to_string(),
            parameters: Vec::new(),
            service_protocol_identity: None,
            extra: serde_json::Map::new(),
        };
        Ok(Self {
            interpreter: Interpreter::for_runtime_assembly(runtime_factory()),
            runtime_id: input.runtime_id,
            activation: input.activation,
            execution_image: input.execution_image,
            resolver: input.resolver,
            activation_identity,
            config,
            package_configs,
            db_source: input.db_source,
            file_source: input.file_source,
            http_options: input.http_options,
            outbound_requests: input.outbound_requests,
            actor_method_outbound: input.actor_method_outbound,
            telemetry_context: input.telemetry_context,
            router_sender: input.router_sender,
            http_response_max_bytes: input.http_response_max_bytes,
            cancellation: input.cancellation,
            execution_budget: input.execution_budget,
            request_heap_limits: input.request_heap_limits,
            request,
            operation,
        })
    }

    pub(crate) fn interpreter(&self) -> &Interpreter {
        &self.interpreter
    }

    pub(crate) fn context(&self) -> anyhow::Result<ProgramExecutionContext<'_>> {
        let request_activation = RequestActivationContext::begin(Arc::clone(&self.activation))?;
        let target = RuntimeAssemblyEvalTarget::new(
            Arc::clone(&self.execution_image),
            request_activation,
            Arc::clone(&self.resolver),
        )?;
        let concrete_execution = skiff_runtime_request::ExecutionControl::new(
            self.cancellation.clone(),
            &self.execution_budget,
        );
        let execution = execution_control(concrete_execution);
        let db = self.db_source.context_for_request(
            self.activation.activation_id().as_str(),
            &self.request.request_id,
        );
        let file = file_source(self.file_source.clone()).context_for_request(db.clone());
        let effects = effects(effect_dispatch_context_from_request(
            &self.request,
            self.http_response_max_bytes,
            execution.cancellation_token(),
            self.telemetry_context.clone(),
            self.http_options.clone(),
        ));
        let service_id = self.activation.identity().deployment.service_id.as_str();
        let websocket_entry_id = self
            .activation
            .websocket_entry_id()
            .map(|entry| entry.as_str());
        let websocket =
            websocket_from_request(service_id, websocket_entry_id, self.router_sender.as_ref());
        let actor = actor_from_request(
            self.runtime_id.as_str(),
            service_id,
            self.activation
                .identity()
                .deployment
                .contract_version
                .as_str(),
            &self.request,
            &self.operation,
            Some(&self.activation_identity),
            self.router_sender.as_ref(),
            &self.outbound_requests,
            &self.actor_method_outbound,
            self.cancellation.clone(),
        );
        let stream_runtime = self.interpreter.stream_runtime.clone();
        let test_effect_doubles = self.interpreter.test_effect_double_context();
        Ok(ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: config_context(concrete::ConfigCapabilityContext::new(
                &self.config,
                &self.package_configs,
            )),
            db,
            file,
            file_source_stream: eval_capabilities::FileSourceStreamContext::new(
                stream_runtime.clone(),
                execution.clone(),
            ),
            time: eval_capabilities::TimeCapabilityContext::new(execution),
            websocket,
            effects: effects.clone(),
            http_client: effects.http_client_context(
                self.interpreter.http_options.clone(),
                stream_runtime,
                test_effect_doubles.clone(),
            ),
            test_effect_doubles,
            actor: actor.clone(),
            spawn: actor,
            request_heap_limits: self.request_heap_limits.clone(),
        })
        .with_websocket_capability_rebinder(websocket_rebinder(self.router_sender.as_ref()))
        .with_runtime_assembly_target(target))
    }
}

fn activation_identity_control(activation: &ActivationContext) -> ActivationIdentityControl {
    let identity = activation.identity();
    ActivationIdentityControl {
        assembly_identity: identity.assembly_identity.clone(),
        generation: identity.assembly_generation,
        runtime_replica_id: identity.runtime_replica_id.clone(),
        deployment_revision: identity.deployment.deployment_revision.clone(),
    }
}
