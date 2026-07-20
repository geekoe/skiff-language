use skiff_runtime_activation::{ActivationContext, RuntimeActivation};
use skiff_runtime_eval::program_execution::{ProgramExecutionContext, ProgramExecutionInput};
use skiff_runtime_linked_program::{GatewayConfig, ServiceMeta};
use skiff_runtime_request::{
    AssemblyRequestEvalAdapter, RequestEvalExecutionInputParts, RuntimeAssemblyRequestTarget,
};

use super::*;

pub(crate) struct RuntimeAssemblyRequestEvalAdapterInput {
    pub(crate) runtime_id: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) file_source: concrete::FileCapabilitySource,
    pub(crate) http_options: concrete::HttpRuntimeOptions,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) spawn_workers: Arc<crate::host::spawn_worker::SpawnWorkerRegistry>,
    pub(crate) telemetry_context: Option<RequestTelemetryContext>,
    pub(crate) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(crate) http_response_max_bytes: usize,
}

pub(crate) fn assembly_request_eval_adapter(
    input: RuntimeAssemblyRequestEvalAdapterInput,
) -> anyhow::Result<Arc<dyn AssemblyRequestEvalAdapter>> {
    let config = crate::config_view::RuntimeConfigView::from_activation_literals(
        &input.activation.owned_bindings().config_literals,
    )?;
    let deployment = &input.activation.identity().deployment;
    let runtime_activation = Arc::new(RuntimeActivation {
        service: ServiceMeta {
            id: deployment.service_id.clone(),
            display_name: None,
            metadata: Default::default(),
        },
        version: deployment.contract_version.clone(),
        package_configs: Vec::new(),
        service_dependencies: Vec::new(),
        timeout: Default::default(),
        operation_route_bindings: Vec::new(),
        db: Vec::new(),
        actors: Vec::new(),
        gateway: GatewayConfig::default(),
    });
    Ok(Arc::new(RuntimeAssemblyRequestEvalAdapter {
        runtime_id: input.runtime_id,
        activation: input.activation,
        config,
        runtime_activation,
        file_source: input.file_source,
        http_options: input.http_options,
        outbound_requests: input.outbound_requests,
        spawn_workers: input.spawn_workers,
        telemetry_context: input.telemetry_context,
        router_sender: input.router_sender,
        http_response_max_bytes: input.http_response_max_bytes,
    }))
}

struct RuntimeAssemblyRequestEvalAdapter {
    runtime_id: String,
    activation: Arc<ActivationContext>,
    config: crate::config_view::RuntimeConfigView,
    runtime_activation: Arc<RuntimeActivation>,
    file_source: concrete::FileCapabilitySource,
    http_options: concrete::HttpRuntimeOptions,
    outbound_requests: Arc<OutboundRequestRegistry>,
    spawn_workers: Arc<crate::host::spawn_worker::SpawnWorkerRegistry>,
    telemetry_context: Option<RequestTelemetryContext>,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    http_response_max_bytes: usize,
}

impl AssemblyRequestEvalAdapter for RuntimeAssemblyRequestEvalAdapter {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RequestEvalExecutionInputParts<'a>,
        _request_context: skiff_runtime_request::RequestPayloadContext<'a>,
        interpreter: &skiff_runtime_eval::Interpreter,
        target: &RuntimeAssemblyRequestTarget,
    ) -> ProgramExecutionContext<'a> {
        let RequestEvalExecutionInputParts {
            operation,
            request,
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        debug_assert!(Arc::ptr_eq(
            &self.activation,
            target.eval().activation_context()
        ));
        let execution = execution_control(execution);
        let db = concrete::DbCapabilityContext::unavailable();
        let file = file_source(self.file_source.clone()).context_for_request(db.clone());
        let effects = effects(effect_dispatch_context_from_request(
            request,
            self.http_response_max_bytes,
            execution.cancellation_token(),
            self.telemetry_context.clone(),
            self.http_options.clone(),
        ));
        let service_id = self.activation.identity().deployment.service_id.as_str();
        let websocket = websocket_from_request(
            service_id,
            request
                .extra
                .get("websocketEntryId")
                .and_then(Value::as_str),
            self.router_sender.as_ref(),
        );
        let actor = actor_from_request(
            self.runtime_id.as_str(),
            service_id,
            self.activation
                .identity()
                .deployment
                .contract_version
                .as_str(),
            request,
            operation,
            self.router_sender.as_ref(),
            &self.outbound_requests,
            &self.spawn_workers,
            cancellation.clone(),
        );
        let stream_runtime = interpreter.stream_runtime.clone();
        let test_effect_doubles = interpreter.test_effect_double_context();
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: config_context(concrete::ConfigCapabilityContext::new(&self.config, &[])),
            db,
            file,
            file_source_stream: eval_capabilities::FileSourceStreamContext::new(
                stream_runtime.clone(),
                execution.clone(),
            ),
            time: eval_capabilities::TimeCapabilityContext::new(execution.clone()),
            websocket,
            effects: effects.clone(),
            http_client: effects.http_client_context(
                interpreter.http_options.clone(),
                stream_runtime,
                test_effect_doubles.clone(),
            ),
            test_effect_doubles,
            runtime_activation: Arc::clone(&self.runtime_activation),
            actor: actor.clone(),
            spawn: actor,
            outbound: retired_assembly_outbound(cancellation, request_heap_limits.clone()),
            request_heap_limits,
        })
        .with_runtime_assembly_target(target.eval().clone())
    }
}
