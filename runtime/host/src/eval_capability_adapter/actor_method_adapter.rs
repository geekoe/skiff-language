use std::{collections::HashMap, sync::Arc};

use serde_json::{Map, Value};
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
/// `contexts` from the same immutable `ActiveAssembly` snapshot.
pub(crate) struct ActorMethodEvalExecutionInput {
    pub(crate) runtime_id: String,
    pub(crate) invocation_id: String,
    pub(crate) trace_id: Option<String>,
    pub(crate) service_protocol_identity: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) execution_image: Arc<AssemblyExecutionImage>,
    pub(crate) config_views: Arc<crate::loader::config_snapshot::ActivationConfigViews>,
    pub(crate) contexts: Arc<crate::loader::active_assembly_context::ActiveAssemblyContextSet>,
    pub(crate) db_source: concrete::DbCapabilitySource,
    pub(crate) file_source: concrete::FileCapabilitySource,
    pub(crate) http_options: concrete::HttpRuntimeOptions,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    pub(crate) telemetry_context: Option<RequestTelemetryContext>,
    pub(crate) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(crate) connection_requests: Arc<ConnectionRequestRegistry>,
    pub(crate) router_session: ConnectionRequestSession,
    pub(crate) http_response_max_bytes: usize,
    pub(crate) cancellation: CancellationToken,
    pub(crate) execution_budget: Arc<ExecutionBudget>,
    pub(crate) request_heap_limits: RequestHeapLimits,
    pub(crate) test_http_entries: concrete::TestHttpEntryRegistry,
}

/// Owned backing for the borrowed eval context consumed by
/// `skiff_runtime_eval::actor_executor::ActorMethodExecutor`.
pub(crate) struct ActorMethodEvalExecution {
    interpreter: Interpreter,
    runtime_id: String,
    activation: Arc<ActivationContext>,
    execution_image: Arc<AssemblyExecutionImage>,
    contexts: Arc<crate::loader::active_assembly_context::ActiveAssemblyContextSet>,
    activation_identity: ActivationIdentityControl,
    config_views: Arc<crate::loader::config_snapshot::ActivationConfigViews>,
    db_source: concrete::DbCapabilitySource,
    file_source: concrete::FileCapabilitySource,
    http_options: concrete::HttpRuntimeOptions,
    outbound_requests: Arc<OutboundRequestRegistry>,
    actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    telemetry_context: Option<RequestTelemetryContext>,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    connection_requests: Arc<ConnectionRequestRegistry>,
    router_session: ConnectionRequestSession,
    http_response_max_bytes: usize,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    request_heap_limits: RequestHeapLimits,
    test_http_entries: concrete::TestHttpEntryRegistry,
    request: RequestEnvelope,
    operation: RuntimeOperation,
}

impl ActorMethodEvalExecution {
    pub(crate) fn new(input: ActorMethodEvalExecutionInput) -> anyhow::Result<Self> {
        if input.invocation_id.trim().is_empty() {
            anyhow::bail!("Actor method invocation id must be non-empty");
        }
        let deployment = &input.activation.identity().deployment;
        let activation_identity = super::assembly_execution_context::activation_identity_control(
            input.activation.as_ref(),
        );
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
            extra: request_extra_with_trace_id(input.trace_id.as_deref()),
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
            contexts: input.contexts,
            activation_identity,
            config_views: input.config_views,
            db_source: input.db_source,
            file_source: input.file_source,
            http_options: input.http_options,
            outbound_requests: input.outbound_requests,
            actor_method_outbound: input.actor_method_outbound,
            telemetry_context: input.telemetry_context,
            router_sender: input.router_sender,
            connection_requests: input.connection_requests,
            router_session: input.router_session,
            http_response_max_bytes: input.http_response_max_bytes,
            cancellation: input.cancellation,
            execution_budget: input.execution_budget,
            request_heap_limits: input.request_heap_limits,
            test_http_entries: input.test_http_entries,
            request,
            operation,
        })
    }

    pub(crate) fn interpreter(&self) -> &Interpreter {
        &self.interpreter
    }

    pub(crate) fn context(&self) -> anyhow::Result<ProgramExecutionContext<'_>> {
        let request_activation = RequestActivationContext::begin(Arc::clone(&self.activation))?;
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::clone(&self.contexts) as _;
        let target = RuntimeAssemblyEvalTarget::new(
            Arc::clone(&self.execution_image),
            request_activation,
            resolver,
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
        let websocket = websocket_from_runtime_request(
            service_id,
            websocket_entry_id,
            self.router_sender.as_ref(),
            Arc::clone(&self.connection_requests),
            self.router_session.clone(),
        );
        let (actor, request) = actor_from_request(
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
        let context = ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: config_context(concrete::ConfigCapabilityContext::new(
                self.config_views.service(),
                self.config_views.packages(),
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
            request,
            request_heap_limits: self.request_heap_limits.clone(),
        })
        .with_runtime_assembly_target(target);
        let rebinder =
            activation_execution_context_rebinder(RuntimeActivationExecutionContextRebinderInput {
                contexts: Arc::clone(&self.contexts),
                execution_image: Arc::clone(&self.execution_image),
                runtime_id: self.runtime_id.clone(),
                request: self.request.clone(),
                file_source: self.file_source.clone(),
                http_options: self.http_options.clone(),
                eval_http_options: self.interpreter.http_options.clone(),
                outbound_requests: Arc::clone(&self.outbound_requests),
                actor_method_outbound: Arc::clone(&self.actor_method_outbound),
                telemetry_context: self.telemetry_context.clone(),
                router_sender: self.router_sender.clone(),
                connection_requests: Arc::clone(&self.connection_requests),
                router_session: self.router_session.clone(),
                http_response_max_bytes: self.http_response_max_bytes,
                test_http_entries: self.test_http_entries.clone(),
                stream_runtime: context.stream_runtime(),
                test_effect_doubles: context.test_effect_double_context(),
                cancellation: context.execution().cancellation_token(),
            });
        Ok(context.with_activation_execution_context_rebinder(rebinder))
    }
}

pub(crate) fn request_extra_with_trace_id(
    trace_id: Option<&str>,
) -> serde_json::Map<String, Value> {
    let mut extra = Map::new();
    let Some(trace_id) = trace_id.filter(|trace_id| !trace_id.trim().is_empty()) else {
        return extra;
    };
    let mut trace = Map::new();
    trace.insert("traceId".to_string(), Value::String(trace_id.to_string()));
    extra.insert("trace".to_string(), Value::Object(trace));
    extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_extra_with_trace_id_builds_request_mapper_trace_shape() {
        let extra = request_extra_with_trace_id(Some("trace:spawn:1"));
        assert_eq!(
            extra["trace"]["traceId"],
            Value::String("trace:spawn:1".to_string())
        );
    }

    #[test]
    fn request_extra_without_trace_id_remains_empty() {
        assert!(request_extra_with_trace_id(None).is_empty());
        assert!(request_extra_with_trace_id(Some("  ")).is_empty());
    }
}
