use super::*;

#[derive(Clone)]
pub(super) struct RuntimeOwnedActorParts {
    pub(super) runtime_id: String,
    pub(super) service_id: String,
    pub(super) service_version: String,
    pub(super) request_id: String,
    pub(super) request_target: String,
    pub(super) request_build_id: String,
    pub(super) request_service_protocol_identity: String,
    pub(super) operation_service_protocol_identity: Option<String>,
    pub(super) activation_identity: Option<ActivationIdentityControl>,
    pub(super) trace_id: Option<String>,
    pub(super) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    pub(super) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(super) spawn_workers: Arc<crate::host::spawn_worker::SpawnWorkerRegistry>,
    pub(super) cancellation: CancellationToken,
}

pub(super) fn actor<'a>(
    context: concrete::ActorCapabilityContext<'a>,
    owned: RuntimeOwnedActorParts,
) -> eval_capabilities::ActorCapabilityContext<'a> {
    eval_capabilities::ActorCapabilityContext::new(RuntimeActorCapabilityContext { context, owned })
}

#[derive(Clone)]
pub(super) struct RuntimeActorCapabilityContext<'a> {
    context: concrete::ActorCapabilityContext<'a>,
    owned: RuntimeOwnedActorParts,
}

impl capability_contract::ActorCapabilityApi for RuntimeActorCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedActorCapabilityContext {
        capability_contract::ActorCapabilityContext::new(RuntimeOwnedActorCapabilityContext(
            self.owned.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::ActorCapabilityContext<'_> {
        actor(self.context.clone(), self.owned.clone())
    }

    fn runtime_id(&self) -> &str {
        self.context.runtime_id()
    }
    fn service_id(&self) -> &str {
        self.context.service_id()
    }
    fn service_version(&self) -> &str {
        self.context.service_version()
    }
    fn request_id(&self) -> &str {
        self.context.request_id()
    }
    fn request_target(&self) -> &str {
        self.context.request_target()
    }
    fn request_build_id(&self) -> &str {
        self.context.request_build_id()
    }
    fn spawn_service_protocol_identity(&self) -> &str {
        self.context.spawn_service_protocol_identity()
    }
    fn request_service_protocol_identity(&self) -> &str {
        self.context.request_service_protocol_identity()
    }
    fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.context.operation_service_protocol_identity()
    }
    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        self.context.activation_identity()
    }
    fn trace_id(&self) -> Option<&str> {
        self.context.trace_id()
    }

    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .put(request, object_payload)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .find(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, bool> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .remove(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(submit_spawn_and_wake(
            self.context.clone(),
            self.owned.spawn_workers.clone(),
            request,
            args_payload,
        ))
    }
}

struct RuntimeOwnedActorCapabilityContext(RuntimeOwnedActorParts);

impl capability_contract::ActorCapabilityApi for RuntimeOwnedActorCapabilityContext {
    fn owned(&self) -> capability_contract::OwnedActorCapabilityContext {
        capability_contract::ActorCapabilityContext::new(RuntimeOwnedActorCapabilityContext(
            self.0.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::ActorCapabilityContext<'_> {
        let context = concrete::ActorCapabilityContext::from_parts(
            &self.0.runtime_id,
            &self.0.service_id,
            &self.0.service_version,
            &self.0.request_id,
            &self.0.request_target,
            &self.0.request_build_id,
            &self.0.request_service_protocol_identity,
            self.0.operation_service_protocol_identity.as_deref(),
            self.0.activation_identity.as_ref(),
            self.0.trace_id.as_deref(),
            self.0.router_sender.as_ref(),
            self.0.outbound_requests.as_ref(),
            self.0.cancellation.clone(),
        );
        actor(context, self.0.clone())
    }

    fn runtime_id(&self) -> &str {
        &self.0.runtime_id
    }
    fn service_id(&self) -> &str {
        &self.0.service_id
    }
    fn service_version(&self) -> &str {
        &self.0.service_version
    }
    fn request_id(&self) -> &str {
        &self.0.request_id
    }
    fn request_target(&self) -> &str {
        &self.0.request_target
    }
    fn request_build_id(&self) -> &str {
        &self.0.request_build_id
    }
    fn spawn_service_protocol_identity(&self) -> &str {
        self.0
            .operation_service_protocol_identity
            .as_deref()
            .unwrap_or(&self.0.request_service_protocol_identity)
    }
    fn request_service_protocol_identity(&self) -> &str {
        &self.0.request_service_protocol_identity
    }
    fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.0.operation_service_protocol_identity.as_deref()
    }
    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        self.0.activation_identity.as_ref()
    }
    fn trace_id(&self) -> Option<&str> {
        self.0.trace_id.as_deref()
    }

    fn put_actor<'a>(
        &'a self,
        request: ActorPutControlRequest,
        object_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .put(request, object_payload)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .find(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> capability_contract::CapabilityFuture<'a, bool> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .remove(request)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(submit_spawn_and_wake(
            concrete_actor_context_from_owned(&self.0),
            self.0.spawn_workers.clone(),
            request,
            args_payload,
        ))
    }
}

async fn submit_spawn_and_wake(
    context: concrete::ActorCapabilityContext<'_>,
    spawn_workers: Arc<crate::host::spawn_worker::SpawnWorkerRegistry>,
    request: SpawnSubmitControlRequest,
    args_payload: Vec<u8>,
) -> capability_contract::CapabilityResult<()> {
    let build_id = request.build_id.clone();
    concrete::ActorClient::new(context)
        .submit_spawn(request, args_payload)
        .await
        .map_err(capability_contract::CapabilityError::opaque)?;
    if let Some(build_id) = build_id {
        spawn_workers.wake_build(&build_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
    use skiff_runtime_transport::protocol::{
        SpawnSubmitResponseFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use tokio::time::{timeout, Duration};

    const BUILD_ID: &str =
        "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[tokio::test]
    async fn successful_spawn_submit_wakes_the_target_build() {
        let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
        let outbound_requests = Arc::new(OutboundRequestRegistry::default());
        let spawn_workers = Arc::new(crate::host::spawn_worker::SpawnWorkerRegistry::default());
        let registration = spawn_workers.registration_for_test();
        let wake = spawn_workers
            .wake_signal_for_test(&registration, BUILD_ID)
            .expect("test registration should exist");
        let activation_identity = spawn_submit_request().activation_identity;
        let context = concrete::ActorClientContext::from_parts(
            "runtime-test",
            "service-test",
            "v1",
            "request-test",
            "program.test",
            BUILD_ID,
            "protocol-test",
            Some("protocol-test"),
            Some(&activation_identity),
            None,
            Some(&router_sender),
            outbound_requests.as_ref(),
            CancellationToken::new(),
        );
        let submit =
            submit_spawn_and_wake(context, spawn_workers, spawn_submit_request(), Vec::new());
        tokio::pin!(submit);

        let rpc_id = tokio::select! {
            result = &mut submit => panic!("spawn submit completed before its response: {result:?}"),
            message = router_receiver.recv() => match message.expect("spawn.submit request should be sent") {
                concrete::RouterWriterMessage::Control(
                    capability_contract::OutboundControlMessage::SpawnSubmit { request, .. }
                ) => request.rpc_id,
                other => panic!("unexpected router message: {other:?}"),
            }
        };
        let response = SpawnSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.submit.response".to_string(),
            rpc_id: rpc_id.clone(),
            spawn_id: "spawn-test".to_string(),
            item_id: "item-test".to_string(),
            status: "submitted".to_string(),
        };
        outbound_requests
            .complete_for_test(&rpc_id)
            .expect("spawn submit response should be pending")
            .send(skiff_runtime_request::OutboundResponse::End {
                payload: serde_json::to_vec(&response).expect("response should serialize"),
            })
            .expect("spawn submit response should be delivered");

        submit.await.expect("spawn submit should succeed");
        timeout(Duration::from_millis(50), wake.notified())
            .await
            .expect("successful submit should preserve a build wake permit");
    }

    fn spawn_submit_request() -> SpawnSubmitControlRequest {
        SpawnSubmitControlRequest {
            rpc_id: String::new(),
            runtime_id: String::new(),
            target_kind: "function".to_string(),
            service_id: "service-test".to_string(),
            service_version: "v1".to_string(),
            service_protocol_identity: "protocol-test".to_string(),
            target: "function:program.test".to_string(),
            spawn_id: None,
            build_id: Some(BUILD_ID.to_string()),
            activation_identity: ActivationIdentityControl {
                assembly_identity: AssemblyIdentity::new(
                    "skiff-runtime-assembly-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                generation: 7,
                runtime_replica_id: "runtime-replica-7".to_string(),
                deployment_revision: DeploymentRevision::new("deployment-revision-7"),
            },
            caller_request_id: Some("request-test".to_string()),
            trace_id: None,
            caller_target: Some("program.test".to_string()),
            max_queue_wait_ms: None,
        }
    }
}
