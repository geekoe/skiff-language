use std::sync::Mutex;

use skiff_runtime_activation::ActivationContext;
use skiff_runtime_eval::program_execution::{ProgramExecutionContext, ProgramExecutionInput};
use skiff_runtime_request::{RequestEnvelope, RequestError, RequestResult, RuntimeOperation};

use super::*;

pub(crate) struct RuntimeAssemblyEvalAdapterContextInput {
    pub(crate) runtime_id: String,
    pub(crate) activation: Arc<ActivationContext>,
    pub(crate) execution_image: Arc<skiff_runtime_linked_program::AssemblyExecutionImage>,
    pub(crate) contexts: Arc<crate::loader::active_assembly_context::ActiveAssemblyContextSet>,
    pub(crate) config_views: Arc<crate::loader::config_snapshot::ActivationConfigViews>,
    pub(crate) execution_target: String,
    pub(crate) service_protocol_identity: String,
    pub(crate) ingress_selector: Option<skiff_artifact_model::IngressSelector>,
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
    pub(super) test_case_capability: Option<String>,
    pub(super) test_case_parent_request_id: Option<String>,
}

pub(super) struct RuntimeAssemblyExecutionContext {
    runtime_id: String,
    pub(super) activation: Arc<ActivationContext>,
    execution_image: Arc<skiff_runtime_linked_program::AssemblyExecutionImage>,
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
    pub(super) router_session: ConnectionRequestSession,
    http_response_max_bytes: usize,
    pub(super) test_http_entries: concrete::TestHttpEntryRegistry,
    pub(super) test_ingress_url: Option<String>,
    pub(super) test_case_capability: Option<String>,
    pub(super) test_case_parent_request_id: Option<String>,
    test_http_admission: Mutex<Option<concrete::TestHttpAdmittedContext>>,
    pub(super) request: RequestEnvelope,
    operation: RuntimeOperation,
}

impl RuntimeAssemblyExecutionContext {
    pub(super) fn new(
        input: RuntimeAssemblyEvalAdapterContextInput,
        metadata: RuntimeAssemblyRequestMetadata,
    ) -> anyhow::Result<Self> {
        let deployment = &input.activation.identity().deployment;
        let activation_identity = activation_identity_control(input.activation.as_ref());
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
            target: input.execution_target.clone(),
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
            ingress_selector: input.ingress_selector,
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: metadata.test_effects_enabled,
            test_effect_doubles: Default::default(),
            payload_bytes: Vec::new(),
            extra,
        };
        let operation = RuntimeOperation {
            operation_abi_id: None,
            operation: input.execution_target.clone(),
            target: input.execution_target,
            mode: metadata.mode,
            parameters: Vec::new(),
            service_protocol_identity: Some(input.service_protocol_identity),
            extra: Default::default(),
        };
        Ok(Self {
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
            test_http_entries: input.test_http_entries,
            test_ingress_url: metadata.test_ingress_url,
            test_case_capability: metadata.test_case_capability,
            test_case_parent_request_id: metadata.test_case_parent_request_id,
            test_http_admission: Mutex::new(None),
            request,
            operation,
        })
    }

    pub(super) fn admit_test_http_context(
        &self,
        context: concrete::TestHttpAdmittedContext,
    ) -> RequestResult<()> {
        if context.router_session() != &self.router_session {
            return Err(RequestError::Unsupported(
                "test HTTP execution authority belongs to another router session".to_string(),
            ));
        }
        if self
            .test_case_capability
            .as_deref()
            .is_some_and(|declared| declared != context.capability())
        {
            return Err(RequestError::Unsupported(
                "test HTTP execution authority does not match the declared capability".to_string(),
            ));
        }
        let mut admitted = self
            .test_http_admission
            .lock()
            .expect("runtime test HTTP admission lock poisoned");
        if admitted.is_some() {
            return Err(RequestError::Unsupported(
                "test HTTP execution authority was already admitted".to_string(),
            ));
        }
        *admitted = Some(context);
        Ok(())
    }

    fn test_http_admission(&self) -> Option<concrete::TestHttpAdmittedContext> {
        self.test_http_admission
            .lock()
            .expect("runtime test HTTP admission lock poisoned")
            .clone()
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
        let test_http_admission = self.test_http_admission();
        let effects = effects(
            effect_dispatch_context_from_request(
                &self.request,
                self.http_response_max_bytes,
                execution.cancellation_token(),
                self.telemetry_context.clone(),
                self.http_options.clone(),
            )
            .with_test_http_self_ingress(explicit_test_http_self_ingress(
                test_http_admission.as_ref(),
            )),
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
            test_http_admission
                .as_ref()
                .map(|context| context.router_session().clone())
                .unwrap_or_else(|| self.router_session.clone()),
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
            test_http_admission
                .as_ref()
                .map(concrete::TestHttpAdmittedContext::capability),
            SpawnCallerKind::Request,
            cancellation.clone(),
        );
        let stream_runtime = interpreter.stream_runtime.clone();
        let test_effect_doubles = interpreter.test_effect_double_context();
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
            time: eval_capabilities::TimeCapabilityContext::new(execution.clone()),
            websocket,
            effects: effects.clone(),
            http_client: effects.http_client_context(
                interpreter.http_options.clone(),
                stream_runtime,
                test_effect_doubles.clone(),
            ),
            test_effect_doubles,
            actor: actor.clone(),
            request,
            request_heap_limits,
        })
        .with_runtime_assembly_target(eval_target.clone());
        let rebinder =
            activation_execution_context_rebinder(RuntimeActivationExecutionContextRebinderInput {
                contexts: Arc::clone(&self.contexts),
                execution_image: Arc::clone(&self.execution_image),
                runtime_id: self.runtime_id.clone(),
                request: self.request.clone(),
                file_source: self.file_source.clone(),
                http_options: self.http_options.clone(),
                eval_http_options: interpreter.http_options.clone(),
                outbound_requests: Arc::clone(&self.outbound_requests),
                actor_method_outbound: Arc::clone(&self.actor_method_outbound),
                telemetry_context: self.telemetry_context.clone(),
                router_sender: self.router_sender.clone(),
                connection_requests: Arc::clone(&self.connection_requests),
                router_session: self.router_session.clone(),
                http_response_max_bytes: self.http_response_max_bytes,
                test_http_admission,
                stream_runtime: context.stream_runtime(),
                test_effect_doubles: context.test_effect_double_context(),
                cancellation: context.execution().cancellation_token(),
            });
        context.with_activation_execution_context_rebinder(rebinder)
    }
}

fn explicit_test_http_self_ingress(
    admission: Option<&concrete::TestHttpAdmittedContext>,
) -> Option<concrete::TestHttpSelfIngressContext> {
    admission.map(concrete::TestHttpAdmittedContext::self_ingress)
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef,
    };

    use crate::capability_context::TestHttpEntryRegistry;

    use super::explicit_test_http_self_ingress;

    #[test]
    fn production_request_id_collision_cannot_create_test_self_ingress_authority() {
        const ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:collision-test";
        let colliding_request_id = "request-id-already-present-in-test-registry";
        let registry = TestHttpEntryRegistry::default();
        let _root = registry
            .begin_root_case(
                "case-with-colliding-request-id",
                ROUTER_SESSION,
                colliding_request_id.to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                ServiceDeploymentRef {
                    service_id: "test.service".to_string(),
                    contract_version: "1.0.0".to_string(),
                    deployment_revision: DeploymentRevision::new("revision-1"),
                    deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                        "skiff-deployment-artifact-v4:sha256:{}",
                        "a".repeat(64)
                    )),
                },
            )
            .expect("test case should register");
        assert!(registry
            .self_ingress_for_request(ROUTER_SESSION, colliding_request_id)
            .is_some());

        // A production Eval has no admitted context. Its colliding request id alone must not
        // grant the test self-ingress authority registered above.
        assert!(explicit_test_http_self_ingress(None).is_none());
    }
}

pub(super) fn activation_identity_control(
    activation: &ActivationContext,
) -> ActivationIdentityControl {
    let identity = activation.identity();
    ActivationIdentityControl {
        assembly_identity: identity.assembly_identity.clone(),
        generation: identity.assembly_generation,
        runtime_replica_id: identity.runtime_replica_id.clone(),
        deployment_revision: identity.deployment.deployment_revision.clone(),
    }
}
