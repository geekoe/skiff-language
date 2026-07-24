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
    pub(super) actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
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

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .get_or_create(request, bootstrap_payload)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn replace_actor<'a>(
        &'a self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(self.context.clone())
                .replace(request, bootstrap_payload)
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

    fn invoke_actor<'a>(
        &'a self,
        request: capability_contract::ActorInvocationRequest,
    ) -> capability_contract::CapabilityFuture<'a, capability_contract::ActorInvocationOutcome>
    {
        Box::pin(invoke_actor_method(self.owned.clone(), request))
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

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .get_or_create(request, bootstrap_payload)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn replace_actor<'a>(
        &'a self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                .replace(request, bootstrap_payload)
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

    fn invoke_actor<'a>(
        &'a self,
        request: capability_contract::ActorInvocationRequest,
    ) -> capability_contract::CapabilityFuture<'a, capability_contract::ActorInvocationOutcome>
    {
        Box::pin(invoke_actor_method(self.0.clone(), request))
    }
}

async fn invoke_actor_method(
    parts: RuntimeOwnedActorParts,
    request: capability_contract::ActorInvocationRequest,
) -> capability_contract::CapabilityResult<capability_contract::ActorInvocationOutcome> {
    use base64::Engine as _;
    use skiff_runtime_transport::actor_method::{
        encode_actor_method_frame, ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
        ActorMethodCancelFrameHeader, ActorMethodCancelReason, ActorMethodDeadlineFrameHeader,
        ActorMethodFrame, ActorMethodInvokeFrameHeader, ActorOwnerFileFrameHeader,
        ActorOwnerUnitFrameHeader, ACTOR_ARGUMENTS_ENCODING_V1,
    };
    use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
    use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

    let invocation_id = request.identity.invocation_id.clone();
    let cancellation_correlation = request.identity.cancellation_correlation.clone();
    let deadline_timeout_ms = request.deadline.timeout_ms;
    let sender = parts.router_sender.clone().ok_or_else(|| {
        capability_contract::CapabilityError::provider_unavailable(
            "actor.method.invoke",
            "router writer is not available",
        )
    })?;
    let epoch = request.actor_ref.epoch().ok_or_else(|| {
        capability_contract::CapabilityError::protocol(
            "actor.method.invoke",
            "Actor method invocation requires a pinned epoch",
        )
    })?;
    if epoch != request.identity.expected_epoch {
        return Err(capability_contract::CapabilityError::protocol(
            "actor.method.invoke",
            "Actor invocation expected epoch does not match its Actor reference",
        ));
    }
    let mut lease = parts
        .actor_method_outbound
        .register(
            invocation_id.clone(),
            cancellation_correlation.clone(),
            epoch,
            request.identity.requested_implementation_identity.clone(),
        )
        .map_err(|message| {
            capability_contract::CapabilityError::protocol("actor.method.invoke", message)
        })?;
    let owner = ActorDeclarationOwnerFrameHeader {
        unit: match request.declaration_owner.unit {
            capability_contract::ActorInvocationOwnerUnit::Service => {
                ActorOwnerUnitFrameHeader::Service
            }
            capability_contract::ActorInvocationOwnerUnit::Package(index) => {
                ActorOwnerUnitFrameHeader::Package(index)
            }
        },
        file: match request.declaration_owner.file {
            capability_contract::ActorInvocationOwnerFile::LoadedFileIndex(index) => {
                ActorOwnerFileFrameHeader::LoadedFileIndex(index)
            }
            capability_contract::ActorInvocationOwnerFile::FileIrIdentity(identity) => {
                ActorOwnerFileFrameHeader::FileIrIdentity(identity)
            }
        },
        actor_symbol: request.declaration_owner.actor_symbol,
    };
    let timeout_ms = i64::try_from(request.deadline.timeout_ms).map_err(|_| {
        capability_contract::CapabilityError::protocol(
            "actor.method.invoke",
            "Actor invocation timeout exceeds the supported range",
        )
    })?;
    let expires_at = (OffsetDateTime::now_utc() + Duration::milliseconds(timeout_ms))
        .format(&Rfc3339)
        .map_err(|error| {
            capability_contract::CapabilityError::protocol(
                "actor.method.invoke",
                format!("cannot format Actor invocation deadline: {error}"),
            )
        })?;
    let invoke = ActorMethodFrame::Invoke(
        ActorMethodInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.invoke".to_string(),
            invocation_id: invocation_id.clone(),
            actor_ref: ActorLogicalRefFrameHeader {
                service_id: request.actor_ref.service_id().to_string(),
                actor_type_identity: request.actor_ref.actor_type_identity().to_string(),
                actor_id_type_identity: request.actor_ref.actor_id_type_identity().to_string(),
                actor_id_encoding_version: request
                    .actor_ref
                    .actor_id_encoding_version()
                    .to_string(),
                canonical_actor_id_key_bytes_base64: base64::engine::general_purpose::STANDARD
                    .encode(request.actor_ref.canonical_actor_id_key_bytes()),
                actor_id_hash: request.actor_ref.actor_id_hash().to_string(),
                epoch,
            },
            declaration_owner: owner,
            actor_abi_identity: request.identity.actor_abi_identity,
            actor_implementation_identity: request.identity.requested_implementation_identity,
            method_identity: request.identity.method_identity,
            arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.to_string(),
            deadline: ActorMethodDeadlineFrameHeader {
                timeout_ms: request.deadline.timeout_ms,
                expires_at,
            },
            cancellation_correlation: cancellation_correlation.clone(),
        },
        request.arguments_payload,
    );
    let wire = encode_actor_method_frame(&invoke).map_err(|error| {
        capability_contract::CapabilityError::protocol(
            "actor.method.invoke",
            format!("cannot encode Actor invocation: {error}"),
        )
    })?;
    sender
        .send(concrete::RouterWriterMessage::Binary(wire))
        .map_err(|_| {
            capability_contract::CapabilityError::provider_unavailable(
                "actor.method.invoke",
                "router writer channel closed",
            )
        })?;

    tokio::select! {
        outcome = lease.receive() => match outcome {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(error)) => Err(capability_contract::CapabilityError::protocol(
                "actor.method.invoke",
                format!("Actor owner transport failure {}: {}", error.code, error.message),
            )),
            Err(_) => Err(capability_contract::CapabilityError::provider_unavailable(
                "actor.method.invoke",
                "Actor invocation response channel closed",
            )),
        },
        _ = parts.cancellation.wait_cancelled() => {
            let cancel = ActorMethodFrame::Cancel(ActorMethodCancelFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "actor.method.cancel".to_string(),
                invocation_id,
                cancellation_correlation,
                reason: ActorMethodCancelReason::Cancelled,
            });
            if let Ok(wire) = encode_actor_method_frame(&cancel) {
                let _ = sender.send(concrete::RouterWriterMessage::Binary(wire));
            }
            Ok(capability_contract::ActorInvocationOutcome::Cancelled(
                capability_contract::ActorInvocationCancellation::Cancelled,
            ))
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_millis(deadline_timeout_ms)) => {
            let cancel = ActorMethodFrame::Cancel(ActorMethodCancelFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "actor.method.cancel".to_string(),
                invocation_id,
                cancellation_correlation,
                reason: ActorMethodCancelReason::DeadlineExceeded,
            });
            if let Ok(wire) = encode_actor_method_frame(&cancel) {
                let _ = sender.send(concrete::RouterWriterMessage::Binary(wire));
            }
            Ok(capability_contract::ActorInvocationOutcome::Cancelled(
                capability_contract::ActorInvocationCancellation::DeadlineExceeded,
            ))
        }
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

    #[tokio::test]
    async fn rejected_spawn_submit_receipt_does_not_wake_the_target_build() {
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
            status: "queued".to_string(),
        };
        outbound_requests
            .complete_for_test(&rpc_id)
            .expect("spawn submit response should be pending")
            .send(skiff_runtime_request::OutboundResponse::End {
                payload: serde_json::to_vec(&response).expect("response should serialize"),
            })
            .expect("spawn submit response should be delivered");

        submit
            .await
            .expect_err("non-submitted receipt must fail through the adapter");
        assert!(
            timeout(Duration::from_millis(20), wake.notified())
                .await
                .is_err(),
            "failed submit receipt must not wake a spawn worker"
        );
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
