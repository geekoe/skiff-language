use skiff_runtime_activation::{ActivationContext, RuntimeActivation};
use skiff_runtime_eval::program_execution::{ProgramExecutionContext, ProgramExecutionInput};
use skiff_runtime_linked_program::{GatewayConfig, ServiceMeta};
use skiff_runtime_request::{RequestEnvelope, RuntimeOperation};

use super::*;

pub(crate) struct RuntimeAssemblyEvalAdapterContextInput {
    pub(crate) runtime_id: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) execution_image: Arc<skiff_runtime_linked_program::AssemblyExecutionImage>,
    pub(crate) gateway_entry_key: String,
    pub(crate) service_protocol_identity: String,
    pub(crate) ingress_selector: skiff_artifact_model::IngressSelector,
    pub(crate) db_source: concrete::DbCapabilitySource,
    pub(crate) file_source: concrete::FileCapabilitySource,
    pub(crate) http_options: concrete::HttpRuntimeOptions,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    pub(crate) spawn_workers: Arc<crate::host::spawn_worker::SpawnWorkerRegistry>,
    pub(crate) telemetry_context: Option<RequestTelemetryContext>,
    pub(crate) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(crate) connection_requests: Arc<ConnectionRequestRegistry>,
    pub(crate) router_session: ConnectionRequestSession,
    pub(crate) http_response_max_bytes: usize,
    pub(crate) test_http_entries: concrete::TestHttpEntryRegistry,
}

pub(super) struct RuntimeAssemblyRequestMetadata {
    pub(super) request_id: String,
    pub(super) mode: String,
    pub(super) caller: serde_json::Value,
    pub(super) client_session: Option<serde_json::Value>,
    pub(super) deadline: Option<serde_json::Value>,
    pub(super) trace: serde_json::Value,
    pub(super) test_effects_enabled: bool,
    pub(super) test_ingress_url: Option<String>,
}

pub(super) struct RuntimeAssemblyExecutionContext {
    runtime_id: String,
    pub(super) activation: Arc<ActivationContext>,
    activation_identity: ActivationIdentityControl,
    config: crate::config_view::RuntimeConfigView,
    package_configs: Vec<crate::config_view::RuntimeConfigView>,
    runtime_activation: Arc<RuntimeActivation>,
    db_source: concrete::DbCapabilitySource,
    file_source: concrete::FileCapabilitySource,
    http_options: concrete::HttpRuntimeOptions,
    outbound_requests: Arc<OutboundRequestRegistry>,
    actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    spawn_workers: Arc<crate::host::spawn_worker::SpawnWorkerRegistry>,
    telemetry_context: Option<RequestTelemetryContext>,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    connection_requests: Arc<ConnectionRequestRegistry>,
    router_session: ConnectionRequestSession,
    http_response_max_bytes: usize,
    pub(super) test_http_entries: concrete::TestHttpEntryRegistry,
    pub(super) test_ingress_url: Option<String>,
    pub(super) request: RequestEnvelope,
    operation: RuntimeOperation,
}

impl RuntimeAssemblyExecutionContext {
    pub(super) fn new(
        input: RuntimeAssemblyEvalAdapterContextInput,
        metadata: RuntimeAssemblyRequestMetadata,
    ) -> anyhow::Result<Self> {
        let config = crate::config_view::RuntimeConfigView::from_activation_literals(
            &input.activation.owned_bindings().config_literals,
        )?;
        let package_configs = super::assembly_request_adapter::package_config_views(
            input.execution_image.as_ref(),
            &input.activation.owned_bindings().config_literals,
        )?;
        let deployment = &input.activation.identity().deployment;
        let activation_identity = activation_identity_control(input.activation.as_ref());
        let runtime_activation = Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: deployment.service_id.clone(),
                display_name: None,
                metadata: Default::default(),
            },
            version: deployment.contract_version.clone(),
            package_configs: package_configs
                .iter()
                .map(|config| config.resolved_config_value().clone())
                .collect(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        });
        let mut extra = serde_json::Map::new();
        extra.insert("caller".to_string(), metadata.caller);
        if let Some(client_session) = metadata.client_session {
            extra.insert("clientSession".to_string(), client_session);
        }
        if let Some(deadline) = metadata.deadline {
            extra.insert("deadline".to_string(), deadline);
        }
        extra.insert("trace".to_string(), metadata.trace);
        let request = RequestEnvelope {
            request_id: metadata.request_id,
            mode: metadata.mode.clone(),
            target: input.gateway_entry_key.clone(),
            operation_abi_id: None,
            selector: None,
            service_id: Some(deployment.service_id.clone()),
            build_id: input
                .activation
                .implementation_package_build_id()
                .as_str()
                .to_string(),
            service_protocol_identity: input.service_protocol_identity.clone(),
            contract_identity: None,
            activation_identity: Some(input.activation.activation_id().as_str().to_string()),
            ingress_selector: Some(input.ingress_selector),
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: metadata.test_effects_enabled,
            test_effect_doubles: Default::default(),
            payload_bytes: Vec::new(),
            extra,
        };
        let operation = RuntimeOperation {
            operation_abi_id: None,
            operation: input.gateway_entry_key.clone(),
            target: input.gateway_entry_key,
            mode: metadata.mode,
            parameters: Vec::new(),
            service_protocol_identity: Some(input.service_protocol_identity),
            extra: Default::default(),
        };
        Ok(Self {
            runtime_id: input.runtime_id,
            activation: input.activation,
            activation_identity,
            config,
            package_configs,
            runtime_activation,
            db_source: input.db_source,
            file_source: input.file_source,
            http_options: input.http_options,
            outbound_requests: input.outbound_requests,
            actor_method_outbound: input.actor_method_outbound,
            spawn_workers: input.spawn_workers,
            telemetry_context: input.telemetry_context,
            router_sender: input.router_sender,
            connection_requests: input.connection_requests,
            router_session: input.router_session,
            http_response_max_bytes: input.http_response_max_bytes,
            test_http_entries: input.test_http_entries,
            test_ingress_url: metadata.test_ingress_url,
            request,
            operation,
        })
    }

    pub(super) fn program_execution_context<'a>(
        &'a self,
        execution: skiff_runtime_request::ExecutionControl<'a>,
        cancellation: CancellationToken,
        request_heap_limits: skiff_runtime_model::request_heap::RequestHeapLimits,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        debug_assert!(Arc::ptr_eq(
            &self.activation,
            eval_target.activation_context()
        ));
        let execution = execution_control(execution);
        let db = self.db_source.context_for_request(
            self.activation.activation_id().as_str(),
            &self.request.request_id,
        );
        let file = file_source(self.file_source.clone()).context_for_request(db.clone());
        let effects = effects(
            effect_dispatch_context_from_request(
                &self.request,
                self.http_response_max_bytes,
                execution.cancellation_token(),
                self.telemetry_context.clone(),
                self.http_options.clone(),
            )
            .with_test_http_self_ingress(
                self.test_http_entries.self_ingress_for_execution(
                    self.activation.activation_id().as_str(),
                    self.request.test_effects_enabled,
                ),
            ),
        );
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
            &self.spawn_workers,
            cancellation.clone(),
        );
        let stream_runtime = interpreter.stream_runtime.clone();
        let test_effect_doubles = interpreter.test_effect_double_context();
        ProgramExecutionContext::new(ProgramExecutionInput {
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
        .with_websocket_capability_rebinder(websocket_rebinder_for_runtime_request(
            self.router_sender.as_ref(),
            Arc::clone(&self.connection_requests),
            self.router_session.clone(),
        ))
        .with_runtime_assembly_target(eval_target.clone())
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
