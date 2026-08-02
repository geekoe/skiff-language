use super::*;
use crate::loader::active_assembly_context::ActiveAssemblyContextSet;
use skiff_runtime_activation::ActivationContext;
use skiff_runtime_eval::{
    program_execution::{
        ActivationExecutionContextRebinder, ActivationExecutionOperation,
        OwnedActivationExecutionCapabilityBundle,
    },
    RuntimeAssemblyEvalResolver,
};
use skiff_runtime_linked_program::AssemblyExecutionImage;

pub(crate) struct RuntimeActivationExecutionContextRebinderInput {
    pub(crate) contexts: Arc<ActiveAssemblyContextSet>,
    pub(crate) execution_image: Arc<AssemblyExecutionImage>,
    pub(crate) runtime_id: String,
    pub(crate) request: RequestEnvelope,
    pub(crate) file_source: concrete::FileCapabilitySource,
    pub(crate) http_options: concrete::HttpRuntimeOptions,
    pub(crate) eval_http_options: eval_capabilities::HttpRuntimeOptions,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    pub(crate) telemetry_context: Option<RequestTelemetryContext>,
    pub(crate) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(crate) connection_requests: Arc<ConnectionRequestRegistry>,
    pub(crate) router_session: ConnectionRequestSession,
    pub(crate) http_response_max_bytes: usize,
    pub(crate) test_http_admission: Option<concrete::TestHttpAdmittedContext>,
    pub(crate) stream_runtime: eval_capabilities::StreamRuntime,
    pub(crate) test_effect_doubles: eval_capabilities::TestEffectDoubleContext,
    pub(crate) cancellation: CancellationToken,
}

pub(crate) fn activation_execution_context_rebinder(
    input: RuntimeActivationExecutionContextRebinderInput,
) -> Arc<dyn ActivationExecutionContextRebinder> {
    Arc::new(RuntimeActivationExecutionContextRebinder { input })
}

struct RuntimeActivationExecutionContextRebinder {
    input: RuntimeActivationExecutionContextRebinderInput,
}

struct ProviderExecutionFacts {
    activation: Arc<ActivationContext>,
    service_protocol_identity: String,
    target: String,
}

impl RuntimeActivationExecutionContextRebinder {
    fn provider_facts(
        &self,
        target: &skiff_runtime_eval::RuntimeAssemblyEvalTarget,
        operation: &ActivationExecutionOperation,
    ) -> Result<ProviderExecutionFacts> {
        provider_facts_from_pinned(
            &self.input.contexts,
            &self.input.execution_image,
            target,
            operation,
        )
    }

    fn provider_telemetry(
        &self,
        facts: &ProviderExecutionFacts,
    ) -> Option<RequestTelemetryContext> {
        self.input.telemetry_context.clone().map(|mut telemetry| {
            let deployment = &facts.activation.identity().deployment;
            telemetry.service_id = Some(deployment.service_id.clone());
            telemetry.revision_id = Some(deployment.deployment_revision.as_str().to_string());
            telemetry.build_id = Some(
                facts
                    .activation
                    .implementation_package_build_id()
                    .as_str()
                    .to_string(),
            );
            telemetry.activation_identity =
                Some(facts.activation.activation_id().as_str().to_string());
            telemetry.target = Some(facts.target.clone());
            telemetry
        })
    }

    fn provider_request(&self, facts: &ProviderExecutionFacts) -> RequestEnvelope {
        let mut request = self.input.request.clone();
        request.target.clone_from(&facts.target);
        request.service_id = Some(facts.activation.identity().deployment.service_id.clone());
        request.build_id = facts
            .activation
            .implementation_package_build_id()
            .as_str()
            .to_string();
        request
            .service_protocol_identity
            .clone_from(&facts.service_protocol_identity);
        request.contract_identity = None;
        request.activation_identity = Some(facts.activation.activation_id().as_str().to_string());
        request.ingress_selector = None;
        request
    }

    fn provider_operation(&self, facts: &ProviderExecutionFacts) -> RuntimeOperation {
        RuntimeOperation {
            operation_abi_id: None,
            operation: facts.target.clone(),
            target: facts.target.clone(),
            mode: self.input.request.mode.clone(),
            parameters: Vec::new(),
            service_protocol_identity: Some(facts.service_protocol_identity.clone()),
            extra: Default::default(),
        }
    }
}

fn provider_facts_from_pinned(
    contexts: &Arc<ActiveAssemblyContextSet>,
    execution_image: &Arc<AssemblyExecutionImage>,
    target: &skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    operation: &ActivationExecutionOperation,
) -> Result<ProviderExecutionFacts> {
    if !Arc::ptr_eq(target.execution_image(), execution_image) {
        return Err(invalid_provider_owner(
            "provider execution target does not use the pinned request execution image",
        ));
    }
    let activation = target.activation_context();
    let deployment = &activation.identity().deployment;
    let admitted = contexts
        .activation_for_deployment(deployment)
        .ok_or_else(|| {
            invalid_provider_owner(
                "provider execution target has no activation in the pinned generation",
            )
        })?;
    if !Arc::ptr_eq(activation, &admitted)
        || activation.identity().assembly_generation != admitted.identity().assembly_generation
        || activation.identity().assembly_identity != admitted.identity().assembly_identity
    {
        return Err(invalid_provider_owner(
            "provider execution target is not the exact pinned activation owner",
        ));
    }
    let contract = contexts
        .contract_for_deployment(deployment)
        .ok_or_else(|| {
            invalid_provider_owner(
                "provider activation has no exact contract in the pinned generation",
            )
        })?;
    if contract.service_id != deployment.service_id
        || contract.contract_version != deployment.contract_version
    {
        return Err(invalid_provider_owner(
            "provider activation contract does not match its deployment owner",
        ));
    }
    let target = match operation {
        ActivationExecutionOperation::ServiceCall { operation_id } => {
            contexts
                .operation_target(activation.activation_id(), operation_id)
                .ok_or_else(|| {
                    invalid_provider_owner(
                        "provider service operation is absent from the pinned generation",
                    )
                })?;
            operation_id.as_str().to_string()
        }
        ActivationExecutionOperation::CallbackMethod { method_abi_id } => {
            if method_abi_id.trim().is_empty() {
                return Err(invalid_provider_owner(
                    "provider callback method ABI identity is empty",
                ));
            }
            method_abi_id.clone()
        }
    };
    Ok(ProviderExecutionFacts {
        activation: admitted,
        service_protocol_identity: contract.service_protocol_identity.as_str().to_string(),
        target,
    })
}

#[cfg(test)]
pub(crate) fn provider_execution_facts_for_test(
    contexts: &Arc<ActiveAssemblyContextSet>,
    execution_image: &Arc<AssemblyExecutionImage>,
    target: &skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    operation: ActivationExecutionOperation,
) -> Result<(Arc<ActivationContext>, String, String)> {
    let facts = provider_facts_from_pinned(contexts, execution_image, target, &operation)?;
    Ok((
        facts.activation,
        facts.service_protocol_identity,
        facts.target,
    ))
}

impl ActivationExecutionContextRebinder for RuntimeActivationExecutionContextRebinder {
    fn rebind(
        &self,
        target: &skiff_runtime_eval::RuntimeAssemblyEvalTarget,
        operation: &ActivationExecutionOperation,
    ) -> Result<OwnedActivationExecutionCapabilityBundle> {
        let facts = self.provider_facts(target, operation)?;
        let deployment = &facts.activation.identity().deployment;
        let config_views = self
            .input
            .contexts
            .config_views(deployment)
            .ok_or_else(|| {
                invalid_provider_owner(
                    "provider activation has no scoped config views in the pinned generation",
                )
            })?;
        let db_source = self
            .input
            .contexts
            .db_source(facts.activation.activation_id())
            .ok_or_else(|| {
                invalid_provider_owner(
                    "provider activation has no DB source in the pinned generation",
                )
            })?;
        let db = db_source.context_for_request(
            facts.activation.activation_id().as_str(),
            &self.input.request.request_id,
        );
        let config = capability_contract::ConfigCapabilityContext::owned(&config_context(
            concrete::ConfigCapabilityContext::new(config_views.service(), config_views.packages()),
        ));
        let websocket = websocket_from_runtime_request(
            deployment.service_id.as_str(),
            facts
                .activation
                .websocket_entry_id()
                .map(|entry| entry.as_str()),
            self.input.router_sender.as_ref(),
            Arc::clone(&self.input.connection_requests),
            self.input
                .test_http_admission
                .as_ref()
                .map(|context| context.router_session().clone())
                .unwrap_or_else(|| self.input.router_session.clone()),
        )
        .owned();
        let request = self.provider_request(&facts);
        let runtime_operation = self.provider_operation(&facts);
        let activation_identity =
            super::assembly_execution_context::activation_identity_control(&facts.activation);
        let (actor, request_context) = actor_from_request(
            self.input.runtime_id.as_str(),
            deployment.service_id.as_str(),
            deployment.contract_version.as_str(),
            &request,
            &runtime_operation,
            Some(&activation_identity),
            self.input.router_sender.as_ref(),
            &self.input.outbound_requests,
            &self.input.actor_method_outbound,
            self.input
                .test_http_admission
                .as_ref()
                .map(concrete::TestHttpAdmittedContext::capability),
            self.input.cancellation.clone(),
        );
        let effects = effects(
            effect_dispatch_context_from_request(
                &request,
                self.input.http_response_max_bytes,
                self.input.cancellation.clone(),
                self.provider_telemetry(&facts),
                self.input.http_options.clone(),
            )
            .with_test_http_self_ingress(
                self.input
                    .test_http_admission
                    .as_ref()
                    .map(concrete::TestHttpAdmittedContext::self_ingress),
            ),
        );
        let http_client = effects.http_client_context(
            self.input.eval_http_options.clone(),
            self.input.stream_runtime.clone(),
            self.input.test_effect_doubles.clone(),
        );
        Ok(OwnedActivationExecutionCapabilityBundle::new(
            config,
            db,
            file_source(self.input.file_source.clone()),
            websocket,
            effects,
            http_client,
            actor.owned(),
            request_context.owned(),
        ))
    }
}

fn invalid_provider_owner(detail: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidArtifact(detail.into())
}
