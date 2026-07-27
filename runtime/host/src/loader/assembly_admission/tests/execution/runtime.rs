use std::{collections::HashMap, sync::Arc};

use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    capabilities::{FileSourceStreamContext, TimeCapabilityContext},
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    Interpreter, RuntimeAssemblyEvalTarget,
};
use skiff_runtime_linked_program::{GatewayConfig, ServiceMeta};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_request::{
    execution_budget::{ExecutionBudget, ExecutionBudgetConfig},
    ExecutionControl, OutboundRequestRegistry, RequestEnvelope, RuntimeOperation,
};

use crate::{
    capability_context::{DbCapabilityContext, FileCapabilitySource},
    config_view::RuntimeConfigView,
    eval_capability_adapter,
    host::file_runtime::FileRuntime,
};

pub(super) struct TypedExecutionRuntime {
    request: RequestEnvelope,
    operation: RuntimeOperation,
    cancellation: CancellationToken,
    budget: Arc<ExecutionBudget>,
    config: RuntimeConfigView,
    package_configs: Vec<RuntimeConfigView>,
    activation: Arc<RuntimeActivation>,
    file_runtime: Arc<FileRuntime>,
    db_request_state: Arc<tokio::sync::Mutex<skiff_runtime_service_db::DbRequestState>>,
    heap_limits: RequestHeapLimits,
    outbound_requests: Arc<OutboundRequestRegistry>,
    actor_factory: eval_capability_adapter::TestActorCapabilityFactory,
}

impl TypedExecutionRuntime {
    pub(super) fn new(service_id: &str) -> Self {
        let cancellation = CancellationToken::new();
        let operation = RuntimeOperation {
            operation_abi_id: None,
            operation: "typedExecutionCheckpoint".to_string(),
            target: service_id.to_string(),
            mode: "unary".to_string(),
            parameters: Vec::new(),
            service_protocol_identity: None,
            extra: serde_json::Map::new(),
        };
        let request = RequestEnvelope {
            request_id: "phase-four-typed-execution".to_string(),
            mode: "unary".to_string(),
            target: service_id.to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: Some(service_id.to_string()),
            build_id: "assembly-checkpoint".to_string(),
            service_protocol_identity: String::new(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: None,
            http_adapter: None,
            binary_http: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::from_iter([(
                "trace".to_string(),
                serde_json::json!({
                    "traceId": "trace-phase-four-typed-execution",
                }),
            )]),
        };
        Self {
            request,
            operation,
            cancellation,
            budget: Arc::new(ExecutionBudget::disabled()),
            config: RuntimeConfigView::empty(),
            package_configs: Vec::new(),
            activation: Arc::new(RuntimeActivation {
                service: ServiceMeta {
                    id: service_id.to_string(),
                    display_name: None,
                    metadata: Default::default(),
                },
                version: "1.0.0".to_string(),
                package_configs: Vec::new(),
                service_dependencies: Vec::new(),
                timeout: Default::default(),
                operation_route_bindings: Vec::new(),
                db: Vec::new(),
                actors: Vec::new(),
                gateway: GatewayConfig::default(),
            }),
            file_runtime: Arc::new(FileRuntime::new(
                None,
                std::env::temp_dir().join("skiff-phase-four-typed-execution"),
            )),
            db_request_state: Arc::new(tokio::sync::Mutex::new(
                skiff_runtime_service_db::DbRequestState::default(),
            )),
            heap_limits: RequestHeapLimits::default(),
            outbound_requests: Arc::new(OutboundRequestRegistry::default()),
            actor_factory: eval_capability_adapter::TestActorCapabilityFactory::default(),
        }
    }

    pub(super) fn interpreter(&self) -> Interpreter {
        Interpreter::for_runtime_assembly(eval_capability_adapter::runtime_factory())
    }

    pub(super) fn with_deadline(mut self, deadline: std::time::Instant) -> Self {
        self.budget = Arc::new(ExecutionBudget::new(
            ExecutionBudgetConfig::runtime_default(),
            Some(deadline),
        ));
        self
    }

    pub(super) fn cancel_request(&self) {
        self.cancellation.cancel();
    }

    pub(super) fn context<'a>(
        &'a self,
        interpreter: &Interpreter,
        target: &RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let stream_runtime = interpreter.stream_runtime.clone();
        let concrete_execution = ExecutionControl::new(self.cancellation.clone(), &self.budget);
        let execution = eval_capability_adapter::execution_control(concrete_execution.clone());
        let db = eval_capability_adapter::db_context(DbCapabilityContext::from_handle(
            skiff_runtime_service_db::ServiceDbCapabilityHandle::with_state(
                None,
                Arc::clone(&self.db_request_state),
            ),
        ));
        let file = eval_capability_adapter::file_source(FileCapabilitySource::new(Arc::clone(
            &self.file_runtime,
        )))
        .context_for_request(db.clone());
        let effects = eval_capability_adapter::effects(
            eval_capability_adapter::effect_dispatch_context_from_request(
                &self.request,
                1_048_576,
                execution.cancellation_token(),
                None,
                skiff_runtime_capability_context::HttpRuntimeOptions::from_env(),
            ),
        );
        let actor = self.actor_factory.actor_from_request(
            "typed-execution-replica",
            self.activation.service.id.as_str(),
            self.activation.version.as_str(),
            &self.request,
            &self.operation,
            None,
            &self.outbound_requests,
            execution.cancellation_token(),
        );
        let outbound = eval_capability_adapter::outbound(
            eval_capability_adapter::outbound_service_context_from_request(
                &self.request,
                self.operation.target.as_str(),
                Arc::clone(&self.budget),
                execution.cancellation_token(),
                self.heap_limits.clone(),
                None,
                Arc::clone(&self.outbound_requests),
                &self.activation.service_dependencies,
                &self.activation.timeout,
            ),
        );
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: eval_capability_adapter::config_context(
                crate::capability_context::ConfigCapabilityContext::new(
                    &self.config,
                    &self.package_configs,
                ),
            ),
            db,
            file,
            file_source_stream: FileSourceStreamContext::new(
                stream_runtime.clone(),
                execution.clone(),
            ),
            time: TimeCapabilityContext::new(execution),
            websocket: eval_capability_adapter::websocket_from_request(
                self.activation.service.id.as_str(),
                None,
                None,
            ),
            effects: effects.clone(),
            http_client: effects.http_client_context(
                interpreter.http_options.clone(),
                stream_runtime,
                interpreter.test_effect_double_context(),
            ),
            test_effect_doubles: interpreter.test_effect_double_context(),
            runtime_activation: Arc::clone(&self.activation),
            actor: actor.clone(),
            spawn: actor,
            outbound,
            request_heap_limits: self.heap_limits.clone(),
        })
        .with_websocket_capability_rebinder(eval_capability_adapter::websocket_rebinder(None))
        .with_runtime_assembly_target(target.clone())
    }
}
