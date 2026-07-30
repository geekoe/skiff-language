use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_artifact_model::IngressProtocol;
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory, program_execution::ProgramExecutionContext, Interpreter,
    RuntimeAssemblyEvalTarget, RuntimeWebSocketConnectRequest, RuntimeWebSocketConnectResult,
    RuntimeWebSocketNameValue,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;

use crate::{
    ExecutionBudget, ExecutionControl, RequestError, RequestResult,
    RuntimeAssemblyWebSocketConnectTarget, RuntimeWebSocketConnectIngress,
};

pub struct RuntimeWebSocketConnectExecutionInput {
    pub target: RuntimeAssemblyWebSocketConnectTarget,
    pub request: RuntimeWebSocketConnectIngress,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: RuntimeWebSocketConnectExecutionHandles,
}

pub struct RuntimeWebSocketConnectExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub eval_adapter: Arc<dyn RuntimeWebSocketConnectEvalAdapter>,
}

pub trait RuntimeWebSocketConnectEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketConnectEvalExecutionInputParts<'a>,
        interpreter: &'a Interpreter,
        eval_target: &'a RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a>;
}

pub struct RuntimeWebSocketConnectEvalExecutionInputParts<'a> {
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub cancelled: &'a AtomicBool,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: RequestHeapLimits,
}

pub async fn execute_runtime_websocket_connect(
    input: RuntimeWebSocketConnectExecutionInput,
) -> RequestResult<RuntimeWebSocketConnectResult> {
    let RuntimeWebSocketConnectExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    validate_request(&target, &request)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(RequestError::Cancelled);
    }
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    let interpreter = if request.test_effects_enabled {
        Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
            Default::default(),
            handles.eval_adapter.runtime_factory(),
        )
    } else {
        Interpreter::for_runtime_assembly(handles.eval_adapter.runtime_factory())
    };
    let context = handles.eval_adapter.execution_context(
        RuntimeWebSocketConnectEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        &interpreter,
        target.eval(),
    );
    let eval_request = eval_request(&request);
    let body_result = interpreter
        .execute_runtime_websocket_connect(context, &eval_request, &target)
        .await
        .map_err(RequestError::from);
    let finalization_result = interpreter.finalize_test_case().map_err(RequestError::from);
    match (body_result, finalization_result) {
        (Err(body_error), _) => Err(body_error),
        (Ok(_), Err(finalization_error)) => Err(finalization_error),
        (Ok(response), Ok(())) => Ok(response),
    }
}

fn validate_request(
    target: &RuntimeAssemblyWebSocketConnectTarget,
    request: &RuntimeWebSocketConnectIngress,
) -> RequestResult<()> {
    validate_request_facts(
        RuntimeWebSocketConnectRequestTargetFacts {
            gateway_entry_key: target.gateway_entry_key(),
            selector: target.selector(),
            assembly_identity: target.eval().execution_image().assembly_identity(),
            assembly_generation: target
                .eval()
                .activation_context()
                .identity()
                .assembly_generation,
            deployment: target.owner(),
            gateway_entry_identity: target.gateway_entry_identity(),
            websocket_entry_id: target.websocket_entry_id(),
        },
        request,
    )
}

struct RuntimeWebSocketConnectRequestTargetFacts<'a> {
    gateway_entry_key: &'a skiff_artifact_model::GatewayEntryKey,
    selector: &'a skiff_artifact_model::IngressSelector,
    assembly_identity: &'a skiff_artifact_model::AssemblyIdentity,
    assembly_generation: u64,
    deployment: &'a skiff_artifact_model::ServiceDeploymentRef,
    gateway_entry_identity: &'a skiff_artifact_model::GatewayEntryIdentity,
    websocket_entry_id: &'a skiff_artifact_model::WebSocketEntryId,
}

fn validate_request_facts(
    target: RuntimeWebSocketConnectRequestTargetFacts<'_>,
    request: &RuntimeWebSocketConnectIngress,
) -> RequestResult<()> {
    let selector = target.selector;
    if selector.protocol != IngressProtocol::WebSocket
        || selector.method.is_some()
        || selector.path != request.ingress_path
        || request.pin.assembly_identity != *target.assembly_identity
        || request.pin.assembly_generation != target.assembly_generation
        || &request.pin.deployment != target.deployment
        || request.pin.gateway_entry_identity != *target.gateway_entry_identity
        || request.connect_gateway_entry_identity != *target.gateway_entry_identity
        || request.websocket_entry_id != *target.websocket_entry_id
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key.as_str(),
            "WebSocket connect routing does not match the exact pinned activation entry",
        ));
    }
    Ok(())
}

fn eval_request(request: &RuntimeWebSocketConnectIngress) -> RuntimeWebSocketConnectRequest {
    RuntimeWebSocketConnectRequest {
        connection_id: request.connection_id.clone(),
        url: request.url.clone(),
        query: name_values(&request.query),
        headers: name_values(&request.headers),
        cookies: name_values(&request.cookies),
        version: request.version.clone(),
        websocket_entry_id: request.websocket_entry_id.clone(),
        gateway_entry_identity: request.connect_gateway_entry_identity.clone(),
    }
}

fn name_values(values: &[crate::HttpNameValue]) -> Vec<RuntimeWebSocketNameValue> {
    values
        .iter()
        .map(|value| RuntimeWebSocketNameValue {
            name: value.name.clone(),
            value: value.value.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity,
        GatewayEntryKey, IngressSelector, ServiceDeploymentRef, WebSocketEntryId,
        WEBSOCKET_GATEWAY_ENTRY_KEY,
    };
    struct Fixture {
        key: GatewayEntryKey,
        selector: IngressSelector,
        assembly: AssemblyIdentity,
        gateway_identity: GatewayEntryIdentity,
        deployment: ServiceDeploymentRef,
        websocket_entry_id: WebSocketEntryId,
        request: RuntimeWebSocketConnectIngress,
    }

    impl Fixture {
        fn new() -> Self {
            let assembly = AssemblyIdentity::new("assembly:websocket");
            let gateway_identity = GatewayEntryIdentity::parse(format!(
                "skiff-gateway-entry-v2:sha256:{}",
                "1".repeat(64)
            ))
            .unwrap();
            let websocket_entry_id = WebSocketEntryId::parse(format!(
                "skiff-websocket-entry-v1:sha256:{}",
                "2".repeat(64)
            ))
            .unwrap();
            let deployment = ServiceDeploymentRef {
                service_id: "service.websocket".to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("revision-1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                    "skiff-deployment-artifact-v4:sha256:{}",
                    "3".repeat(64)
                )),
            };
            let selector = IngressSelector {
                protocol: IngressProtocol::WebSocket,
                method: None,
                path: "/connect".to_string(),
            };
            let request = RuntimeWebSocketConnectIngress {
                request_id: "request-1".to_string(),
                pin: crate::RuntimeGatewayIngressPin {
                    assembly_identity: assembly.clone(),
                    assembly_generation: 7,
                    deployment: deployment.clone(),
                    gateway_entry_identity: gateway_identity.clone(),
                },
                ingress_path: selector.path.clone(),
                connection_id: "connection-1".to_string(),
                url: "wss://websocket.test/connect".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
                cookies: Vec::new(),
                version: None,
                websocket_entry_id: websocket_entry_id.clone(),
                connect_gateway_entry_identity: gateway_identity.clone(),
                test_effects_enabled: false,
            };
            Self {
                key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
                selector,
                assembly,
                gateway_identity,
                deployment,
                websocket_entry_id,
                request,
            }
        }

        fn facts(&self) -> RuntimeWebSocketConnectRequestTargetFacts<'_> {
            RuntimeWebSocketConnectRequestTargetFacts {
                gateway_entry_key: &self.key,
                selector: &self.selector,
                assembly_identity: &self.assembly,
                assembly_generation: 7,
                deployment: &self.deployment,
                gateway_entry_identity: &self.gateway_identity,
                websocket_entry_id: &self.websocket_entry_id,
            }
        }
    }

    #[test]
    fn websocket_connect_request_projection_matches_exact_activation_entry() {
        let fixture = Fixture::new();
        validate_request_facts(fixture.facts(), &fixture.request)
            .expect("exact request facts should validate");
    }

    #[test]
    fn websocket_connect_request_rejects_projected_activation_and_generation_mismatches() {
        let fixture = Fixture::new();
        let mut mutations = Vec::new();

        let mut wrong_routing_identity = fixture.request.clone();
        wrong_routing_identity.pin.gateway_entry_identity = GatewayEntryIdentity::parse(format!(
            "skiff-gateway-entry-v2:sha256:{}",
            "3".repeat(64)
        ))
        .unwrap();
        mutations.push(wrong_routing_identity);

        let mut wrong_connect_identity = fixture.request.clone();
        wrong_connect_identity.connect_gateway_entry_identity = GatewayEntryIdentity::parse(
            format!("skiff-gateway-entry-v2:sha256:{}", "4".repeat(64)),
        )
        .unwrap();
        mutations.push(wrong_connect_identity);

        let mut wrong_entry_id = fixture.request.clone();
        wrong_entry_id.websocket_entry_id = WebSocketEntryId::parse(format!(
            "skiff-websocket-entry-v1:sha256:{}",
            "5".repeat(64)
        ))
        .unwrap();
        mutations.push(wrong_entry_id);

        let mut wrong_assembly = fixture.request.clone();
        wrong_assembly.pin.assembly_identity = AssemblyIdentity::new("assembly:other");
        mutations.push(wrong_assembly);

        let mut stale_generation = fixture.request.clone();
        stale_generation.pin.assembly_generation = 6;
        mutations.push(stale_generation);

        let mut wrong_deployment = fixture.request.clone();
        wrong_deployment.pin.deployment.service_id = "service.other".to_string();
        mutations.push(wrong_deployment);

        for mutation in mutations {
            assert!(validate_request_facts(fixture.facts(), &mutation).is_err());
        }
    }
}
