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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(self.context.clone())
                    .get_or_create(request, bootstrap_payload)
                    .await,
            )
            .await
        })
    }

    fn replace_actor<'a>(
        &'a self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(self.context.clone())
                    .replace(request, bootstrap_payload)
                    .await,
            )
            .await
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(self.context.clone())
                    .find(request)
                    .await,
            )
            .await
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, bool> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(self.context.clone())
                    .remove(request)
                    .await,
            )
            .await
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(submit_spawn_and_wake(
            self.context.clone(),
            self.owned.spawn_workers.clone(),
            request,
            args_payload,
            execution_control,
        ))
    }

    fn invoke_actor<'a>(
        &'a self,
        request: capability_contract::ActorInvocationRequest,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, capability_contract::ActorInvocationOutcome>
    {
        Box::pin(invoke_actor_method(
            self.owned.clone(),
            request,
            execution_control,
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

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .get_or_create(request, bootstrap_payload)
                    .await,
            )
            .await
        })
    }

    fn replace_actor<'a>(
        &'a self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .replace(request, bootstrap_payload)
                    .await,
            )
            .await
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .find(request)
                    .await,
            )
            .await
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, bool> {
        Box::pin(async move {
            let _execution_control = execution_control;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .remove(request)
                    .await,
            )
            .await
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(submit_spawn_and_wake(
            concrete_actor_context_from_owned(&self.0),
            self.0.spawn_workers.clone(),
            request,
            args_payload,
            execution_control,
        ))
    }

    fn invoke_actor<'a>(
        &'a self,
        request: capability_contract::ActorInvocationRequest,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, capability_contract::ActorInvocationOutcome>
    {
        Box::pin(invoke_actor_method(
            self.0.clone(),
            request,
            execution_control,
        ))
    }
}

async fn invoke_actor_method(
    parts: RuntimeOwnedActorParts,
    request: capability_contract::ActorInvocationRequest,
    execution_control: capability_contract::OwnedExecutionControl,
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

    let _execution_control = execution_control;
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
        biased;
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
    execution_control: capability_contract::OwnedExecutionControl,
) -> capability_contract::CapabilityResult<()> {
    let _execution_control = execution_control;
    let build_id = request.build_id.clone();
    root_result_into_capability(
        concrete::ActorClient::new(context)
            .submit_spawn(request, args_payload)
            .await,
    )
    .await?;
    if let Some(build_id) = build_id {
        spawn_workers.wake_build(&build_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, AssemblyIdentity,
        DeploymentRevision,
    };
    use skiff_runtime_capability_context::{
        ActorInvocationCancellation, ActorInvocationDeadline, ActorInvocationDeclarationOwner,
        ActorInvocationIdentity, ActorInvocationOutcome, ActorInvocationOwnerFile,
        ActorInvocationOwnerUnit, ActorInvocationRequest,
    };
    use skiff_runtime_transport::actor_method::{
        decode_actor_method_frame, ActorMethodCancelReason, ActorMethodFrame,
    };
    use skiff_runtime_transport::protocol::{
        SpawnSubmitResponseFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use tokio::time::{timeout, Duration};

    const BUILD_ID: &str =
        "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[tokio::test]
    async fn actor_method_cancel_wakes_pending_invocation_and_releases_lease() {
        let cancellation = CancellationToken::new();
        let (parts, request, mut router_receiver, outbound) =
            actor_invocation_fixture(30_000, cancellation.clone(), "actor-invoke-cancel");
        let invocation = invoke_actor_method(parts, request, test_execution_control());
        tokio::pin!(invocation);

        assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
        cancellation.cancel();
        let outcome = timeout(Duration::from_secs(1), &mut invocation)
            .await
            .expect("actor cancellation must wake the pending invocation")
            .expect("actor cancellation is an internal outcome");
        assert_eq!(
            outcome,
            ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::Cancelled)
        );
        assert_actor_cancel_frame(&mut router_receiver, ActorMethodCancelReason::Cancelled).await;
        assert_eq!(
            outbound.cancellation_correlation("actor-invoke-cancel"),
            None,
            "terminal owner must release the actor invocation lease"
        );
    }

    #[tokio::test]
    async fn actor_method_deadline_remains_distinct_and_releases_lease() {
        let (parts, request, mut router_receiver, outbound) =
            actor_invocation_fixture(1, CancellationToken::new(), "actor-invoke-deadline");
        let invocation = invoke_actor_method(parts, request, test_execution_control());
        tokio::pin!(invocation);

        assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
        let outcome = timeout(Duration::from_secs(1), &mut invocation)
            .await
            .expect("actor deadline must wake the pending invocation")
            .expect("actor deadline is a typed outcome");
        assert_eq!(
            outcome,
            ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::DeadlineExceeded)
        );
        assert_actor_cancel_frame(
            &mut router_receiver,
            ActorMethodCancelReason::DeadlineExceeded,
        )
        .await;
        assert_eq!(
            outbound.cancellation_correlation("actor-invoke-deadline"),
            None,
            "deadline owner must release the actor invocation lease"
        );
    }

    #[tokio::test]
    async fn actor_method_cancel_beats_simultaneously_ready_deadline() {
        let cancellation = CancellationToken::new();
        let (parts, request, mut router_receiver, outbound) =
            actor_invocation_fixture(1, cancellation.clone(), "actor-invoke-biased");
        let invocation = invoke_actor_method(parts, request, test_execution_control());
        tokio::pin!(invocation);

        assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        cancellation.cancel();
        let outcome = invocation
            .await
            .expect("ancestor cancellation is a typed internal outcome");
        assert_eq!(
            outcome,
            ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::Cancelled)
        );
        assert_actor_cancel_message(
            router_receiver
                .recv()
                .await
                .expect("cancel frame must settle the invocation"),
            ActorMethodCancelReason::Cancelled,
        );
        assert_eq!(
            outbound.cancellation_correlation("actor-invoke-biased"),
            None
        );
    }

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
        let submit = submit_spawn_and_wake(
            context,
            spawn_workers,
            spawn_submit_request(),
            Vec::new(),
            test_execution_control(),
        );
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
        let submit = submit_spawn_and_wake(
            context,
            spawn_workers,
            spawn_submit_request(),
            Vec::new(),
            test_execution_control(),
        );
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

    fn actor_invocation_fixture(
        timeout_ms: u64,
        cancellation: CancellationToken,
        invocation_id: &str,
    ) -> (
        RuntimeOwnedActorParts,
        ActorInvocationRequest,
        mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
        Arc<ActorMethodOutboundRegistry>,
    ) {
        let (router_sender, router_receiver) = mpsc::unbounded_channel();
        let actor_method_outbound = Arc::new(ActorMethodOutboundRegistry::default());
        let implementation_identity = ActorImplementationIdentity::new(format!(
            "skiff-actor-implementation-v1:sha256:{}",
            "b".repeat(64)
        ));
        let parts = RuntimeOwnedActorParts {
            runtime_id: "runtime-test".to_string(),
            service_id: "service-test".to_string(),
            service_version: "v1".to_string(),
            request_id: "request-test".to_string(),
            request_target: "program.test".to_string(),
            request_build_id: BUILD_ID.to_string(),
            request_service_protocol_identity: "protocol-test".to_string(),
            operation_service_protocol_identity: Some("protocol-test".to_string()),
            activation_identity: None,
            trace_id: None,
            router_sender: Some(router_sender),
            outbound_requests: Arc::new(OutboundRequestRegistry::default()),
            actor_method_outbound: actor_method_outbound.clone(),
            spawn_workers: Arc::new(crate::host::spawn_worker::SpawnWorkerRegistry::default()),
            cancellation,
        };
        let request = ActorInvocationRequest {
            actor_ref: ActorRef::new(
                "service-test",
                "actor-type-test",
                "actor-id-type-test",
                "skiff-actor-id-v1",
                vec![1],
                format!("sha256:{}", "d".repeat(64)),
                Some(7),
            ),
            declaration_owner: ActorInvocationDeclarationOwner {
                unit: ActorInvocationOwnerUnit::Service,
                file: ActorInvocationOwnerFile::LoadedFileIndex(0),
                actor_symbol: "TestActor".to_string(),
            },
            identity: ActorInvocationIdentity {
                invocation_id: invocation_id.to_string(),
                expected_epoch: 7,
                actor_abi_identity: ActorAbiIdentity::new(format!(
                    "skiff-actor-abi-v1:sha256:{}",
                    "a".repeat(64)
                )),
                requested_implementation_identity: implementation_identity,
                method_identity: ActorMethodIdentity::new(format!(
                    "skiff-actor-method-v1:sha256:{}",
                    "c".repeat(64)
                )),
                cancellation_correlation: format!("{invocation_id}:cancel"),
            },
            deadline: ActorInvocationDeadline { timeout_ms },
            arguments_payload: b"[]".to_vec(),
        };
        (parts, request, router_receiver, actor_method_outbound)
    }

    async fn assert_actor_invoke_frame<F>(
        router_receiver: &mut mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
        invocation: &mut Pin<&mut F>,
    ) where
        F: Future<
            Output = capability_contract::CapabilityResult<
                capability_contract::ActorInvocationOutcome,
            >,
        >,
    {
        tokio::select! {
            result = invocation.as_mut() => {
                panic!("actor invocation completed before its invoke frame: {result:?}")
            }
            message = router_receiver.recv() => {
                assert_actor_invoke_message(
                    message.expect("actor method invoke frame must be sent")
                );
            }
        }
    }

    async fn assert_actor_cancel_frame(
        router_receiver: &mut mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
        expected_reason: ActorMethodCancelReason,
    ) {
        let message = timeout(Duration::from_secs(1), router_receiver.recv())
            .await
            .expect("actor cancel frame must be emitted")
            .expect("router writer must remain open");
        assert_actor_cancel_message(message, expected_reason);
    }

    fn assert_actor_invoke_message(message: concrete::RouterWriterMessage) {
        let concrete::RouterWriterMessage::Binary(frame) = message else {
            panic!("actor invocation must use a binary frame")
        };
        assert!(matches!(
            decode_actor_method_frame(&frame).expect("actor invoke frame must decode"),
            ActorMethodFrame::Invoke(_, _)
        ));
    }

    fn assert_actor_cancel_message(
        message: concrete::RouterWriterMessage,
        expected_reason: ActorMethodCancelReason,
    ) {
        let concrete::RouterWriterMessage::Binary(frame) = message else {
            panic!("actor cancellation must use a binary frame")
        };
        let ActorMethodFrame::Cancel(cancel) =
            decode_actor_method_frame(&frame).expect("actor cancel frame must decode")
        else {
            panic!("expected actor method cancel frame")
        };
        assert_eq!(cancel.reason, expected_reason);
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
                    "skiff-runtime-assembly-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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

    fn test_execution_control() -> capability_contract::OwnedExecutionControl {
        use skiff_runtime_request::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

        let budget = Arc::new(ExecutionBudget::new(
            ExecutionBudgetConfig::disabled(),
            None,
        ));
        let execution =
            skiff_runtime_request::ExecutionControl::new(CancellationToken::new(), &budget);
        super::super::execution_control(execution).owned()
    }
}
