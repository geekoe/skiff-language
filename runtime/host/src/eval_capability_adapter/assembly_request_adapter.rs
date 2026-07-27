use skiff_runtime_activation::{ActivationContext, RuntimeActivation};
use skiff_runtime_eval::program_execution::{ProgramExecutionContext, ProgramExecutionInput};
use skiff_runtime_linked_program::{GatewayConfig, ServiceMeta};
use skiff_runtime_request::{
    RequestEnvelope, RuntimeHttpGatewayEvalAdapter, RuntimeHttpGatewayEvalExecutionInputParts,
    RuntimeOperation, RuntimeWebSocketConnectEvalAdapter,
    RuntimeWebSocketConnectEvalExecutionInputParts,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
};

use super::*;

pub(crate) struct RuntimeHttpGatewayEvalAdapterInput {
    pub(crate) runtime_id: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) execution_image: Arc<skiff_runtime_linked_program::AssemblyExecutionImage>,
    pub(crate) header: RuntimeAssemblyRequestStartFrameHeader,
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
    pub(crate) http_response_max_bytes: usize,
}

pub(crate) fn http_gateway_eval_adapter(
    input: RuntimeHttpGatewayEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeHttpGatewayEvalAdapter>> {
    let config = crate::config_view::RuntimeConfigView::from_activation_literals(
        &input.activation.owned_bindings().config_literals,
    )?;
    let package_configs = package_config_views(
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
    extra.insert(
        "caller".to_string(),
        serde_json::to_value(&input.header.caller)?,
    );
    if let Some(client_session) = &input.header.client_session {
        extra.insert(
            "clientSession".to_string(),
            serde_json::to_value(client_session)?,
        );
    }
    if let Some(deadline) = &input.header.deadline {
        extra.insert("deadline".to_string(), serde_json::to_value(deadline)?);
    }
    extra.insert(
        "trace".to_string(),
        serde_json::to_value(&input.header.trace)?,
    );
    let request = RequestEnvelope {
        request_id: input.header.request_id.clone(),
        mode: input.header.mode.clone(),
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
        test_effects_enabled: input.header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra,
    };
    let operation = RuntimeOperation {
        operation_abi_id: None,
        operation: input.gateway_entry_key.clone(),
        target: input.gateway_entry_key,
        mode: input.header.mode,
        parameters: Vec::new(),
        service_protocol_identity: Some(input.service_protocol_identity),
        extra: Default::default(),
    };
    Ok(Arc::new(RuntimeHttpGatewayEvalAdapterImpl {
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
        http_response_max_bytes: input.http_response_max_bytes,
        request,
        operation,
    }))
}

pub(crate) struct RuntimeWebSocketConnectEvalAdapterInput {
    pub(crate) runtime_id: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) execution_image: Arc<skiff_runtime_linked_program::AssemblyExecutionImage>,
    pub(crate) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
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
    pub(crate) http_response_max_bytes: usize,
}

pub(crate) fn websocket_connect_eval_adapter(
    input: RuntimeWebSocketConnectEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeWebSocketConnectEvalAdapter>> {
    let config = crate::config_view::RuntimeConfigView::from_activation_literals(
        &input.activation.owned_bindings().config_literals,
    )?;
    let package_configs = package_config_views(
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
    extra.insert(
        "caller".to_string(),
        serde_json::to_value(&input.header.caller)?,
    );
    if let Some(client_session) = &input.header.client_session {
        extra.insert(
            "clientSession".to_string(),
            serde_json::to_value(client_session)?,
        );
    }
    if let Some(deadline) = &input.header.deadline {
        extra.insert("deadline".to_string(), serde_json::to_value(deadline)?);
    }
    extra.insert(
        "trace".to_string(),
        serde_json::to_value(&input.header.trace)?,
    );
    let request = RequestEnvelope {
        request_id: input.header.request_id.clone(),
        mode: input.header.mode.clone(),
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
        test_effects_enabled: input.header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra,
    };
    let operation = RuntimeOperation {
        operation_abi_id: None,
        operation: input.gateway_entry_key.clone(),
        target: input.gateway_entry_key,
        mode: input.header.mode,
        parameters: Vec::new(),
        service_protocol_identity: Some(input.service_protocol_identity),
        extra: Default::default(),
    };
    Ok(Arc::new(RuntimeHttpGatewayEvalAdapterImpl {
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
        http_response_max_bytes: input.http_response_max_bytes,
        request,
        operation,
    }))
}

struct RuntimeHttpGatewayEvalAdapterImpl {
    runtime_id: String,
    activation: Arc<ActivationContext>,
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
    http_response_max_bytes: usize,
    request: RequestEnvelope,
    operation: RuntimeOperation,
}

impl RuntimeHttpGatewayEvalAdapterImpl {
    fn program_execution_context<'a>(
        &'a self,
        execution: skiff_runtime_request::ExecutionControl<'a>,
        cancellation: CancellationToken,
        request_heap_limits: skiff_runtime_model::request_heap::RequestHeapLimits,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let request = &self.request;
        let operation = &self.operation;
        debug_assert!(Arc::ptr_eq(
            &self.activation,
            eval_target.activation_context()
        ));
        let execution = execution_control(execution);
        let db = self.db_source.context_for_request(
            self.activation.activation_id().as_str(),
            &request.request_id,
        );
        let file = file_source(self.file_source.clone()).context_for_request(db.clone());
        let effects = effects(effect_dispatch_context_from_request(
            request,
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
            request,
            operation,
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
        .with_websocket_capability_rebinder(websocket_rebinder(self.router_sender.as_ref()))
        .with_runtime_assembly_target(eval_target.clone())
    }
}

impl RuntimeHttpGatewayEvalAdapter for RuntimeHttpGatewayEvalAdapterImpl {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeHttpGatewayEvalExecutionInputParts<'a>,
        _request_context: skiff_runtime_request::RequestPayloadContext<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeHttpGatewayEvalExecutionInputParts {
            header: _,
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

impl RuntimeWebSocketConnectEvalAdapter for RuntimeHttpGatewayEvalAdapterImpl {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketConnectEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeWebSocketConnectEvalExecutionInputParts {
            header: _,
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

pub(super) fn package_config_views(
    image: &skiff_runtime_linked_program::AssemblyExecutionImage,
    literals: &[skiff_artifact_model::ConfigLiteralBinding],
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    if image.code_slots().len() != image.shared_packages().code_slots().len() {
        anyhow::bail!("active execution image package code-slot vectors are misaligned");
    }

    let mut requirements_by_slot = Vec::with_capacity(image.shared_packages().code_slots().len());
    for (slot, package) in image.shared_packages().code_slots().iter().enumerate() {
        if package.code_slot().index() != slot {
            anyhow::bail!(
                "active execution image package slot mismatch: expected {slot}, got {}",
                package.code_slot().index()
            );
        }
        requirements_by_slot.push(package.artifact().runtime_requirements.config.as_slice());
    }
    package_config_views_from_requirements(&requirements_by_slot, literals)
}

fn package_config_views_from_requirements(
    requirements_by_slot: &[&[skiff_artifact_model::PackageConfigRequirement]],
    literals: &[skiff_artifact_model::ConfigLiteralBinding],
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    use std::collections::BTreeSet;

    let mut known_paths = BTreeSet::new();
    let mut views = Vec::with_capacity(requirements_by_slot.len());
    for requirements in requirements_by_slot {
        let required_paths = requirements
            .iter()
            .map(|requirement| requirement.path.as_str())
            .collect::<BTreeSet<_>>();
        known_paths.extend(required_paths.iter().map(|path| (*path).to_string()));
        let scoped = literals
            .iter()
            .filter(|literal| required_paths.contains(literal.path.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let shape = skiff_artifact_model::config_shape_from_package_requirements(requirements)?;
        views.push(
            crate::config_view::RuntimeConfigView::from_activation_literals_with_shape(
                &scoped, shape,
            )?,
        );
    }
    if let Some(unknown) = literals
        .iter()
        .find(|literal| !known_paths.contains(&literal.path))
    {
        anyhow::bail!(
            "activation config literal {} is not required by an exact active package slot",
            unknown.path
        );
    }
    Ok(views)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_artifact_model::{ConfigLiteralBinding, MetadataValue, PackageConfigRequirement};

    use super::package_config_views_from_requirements;

    fn requirement(path: &str, value_type: &str, required: bool) -> PackageConfigRequirement {
        PackageConfigRequirement {
            path: path.to_string(),
            value_type: value_type.to_string(),
            required,
        }
    }

    fn literal(path: &str, value: MetadataValue) -> ConfigLiteralBinding {
        ConfigLiteralBinding {
            path: path.to_string(),
            value,
        }
    }

    #[test]
    fn activation_literals_are_projected_to_exact_package_slots() {
        let own = [
            requirement("cookieName", "string", true),
            requirement("maxAgeSeconds", "number", true),
        ];
        let dependency = [requirement("dependency.token", "string", true)];
        let views = package_config_views_from_requirements(
            &[&own, &dependency],
            &[
                literal("cookieName", MetadataValue::String("sid".into())),
                literal("maxAgeSeconds", MetadataValue::Number(3600.into())),
                literal(
                    "dependency.token",
                    MetadataValue::String("dependency-value".into()),
                ),
            ],
        )
        .unwrap();

        assert_eq!(views.len(), 2);
        assert_eq!(
            views[0].resolved_config_value(),
            &json!({"cookieName": "sid", "maxAgeSeconds": 3600})
        );
        assert_eq!(
            views[1].resolved_config_value(),
            &json!({"dependency": {"token": "dependency-value"}})
        );
        assert!(views[0].resolved_config_value().get("dependency").is_none());
        assert!(views[1].resolved_config_value().get("cookieName").is_none());
    }

    #[test]
    fn package_config_projection_fails_closed() {
        let own = [requirement("cookieName", "string", true)];

        let missing = package_config_views_from_requirements(&[&own], &[]).unwrap_err();
        assert!(missing
            .to_string()
            .contains("cookieName required value is missing"));

        let wrong_type = package_config_views_from_requirements(
            &[&own],
            &[literal("cookieName", MetadataValue::Number(1.into()))],
        )
        .unwrap_err();
        assert!(wrong_type
            .to_string()
            .contains("cookieName must be a string"));

        let unknown = package_config_views_from_requirements(
            &[&own],
            &[
                literal("cookieName", MetadataValue::String("sid".into())),
                literal("retired.key", MetadataValue::String("stale".into())),
            ],
        )
        .unwrap_err();
        assert!(unknown
            .to_string()
            .contains("retired.key is not required by an exact active package slot"));

        let duplicate = package_config_views_from_requirements(
            &[&own],
            &[
                literal("cookieName", MetadataValue::String("sid".into())),
                literal("cookieName", MetadataValue::String("other".into())),
            ],
        )
        .unwrap_err();
        assert!(duplicate.to_string().contains("cookieName is duplicated"));
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
