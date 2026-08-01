use super::*;

#[derive(Clone)]
pub(super) struct RuntimeOwnedRequestParts {
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
    pub(super) cancellation: CancellationToken,
}

pub(super) fn actor<'a>(
    actor_context: concrete::ActorClientContext<'a>,
    request_context: concrete::RequestClientContext<'a>,
    owned: RuntimeOwnedRequestParts,
) -> eval_capabilities::ActorCapabilityContext<'a> {
    eval_capabilities::ActorCapabilityContext::new(RuntimeActorCapabilityContext {
        actor_context,
        request_context,
        owned,
    })
}

pub(super) fn request_capability<'a>(
    actor_context: concrete::ActorClientContext<'a>,
    request_context: concrete::RequestClientContext<'a>,
    owned: RuntimeOwnedRequestParts,
) -> eval_capabilities::RequestCapabilityContext<'a> {
    eval_capabilities::RequestCapabilityContext::new(RuntimeActorCapabilityContext {
        actor_context,
        request_context,
        owned,
    })
}

#[derive(Clone)]
pub(super) struct RuntimeActorCapabilityContext<'a> {
    actor_context: concrete::ActorClientContext<'a>,
    request_context: concrete::RequestClientContext<'a>,
    owned: RuntimeOwnedRequestParts,
}

impl capability_contract::ActorCapabilityApi for RuntimeActorCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedActorCapabilityContext {
        capability_contract::ActorCapabilityContext::new(RuntimeOwnedRequestCapabilityContext(
            self.owned.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::ActorCapabilityContext<'_> {
        actor(
            self.actor_context.clone(),
            self.request_context.clone(),
            self.owned.clone(),
        )
    }

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(self.actor_context.clone())
                    .get_or_create_in_scope(request, bootstrap_payload, scope)
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
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(self.actor_context.clone())
                    .replace_in_scope(request, bootstrap_payload, scope)
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
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(self.actor_context.clone())
                    .find_in_scope(request, scope)
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
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(self.actor_context.clone())
                    .remove_in_scope(request, scope)
                    .await,
            )
            .await
        })
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

impl capability_contract::RequestCapabilityApi for RuntimeActorCapabilityContext<'_> {
    fn owned(&self) -> capability_contract::OwnedRequestCapabilityContext {
        capability_contract::RequestCapabilityContext::new(RuntimeOwnedRequestCapabilityContext(
            self.owned.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::RequestCapabilityContext<'_> {
        request_capability(
            self.actor_context.clone(),
            self.request_context.clone(),
            self.owned.clone(),
        )
    }

    fn runtime_id(&self) -> &str {
        self.request_context.runtime_id()
    }
    fn service_id(&self) -> &str {
        self.request_context.service_id()
    }
    fn service_version(&self) -> &str {
        self.request_context.service_version()
    }
    fn request_id(&self) -> &str {
        self.request_context.request_id()
    }
    fn request_target(&self) -> &str {
        self.request_context.request_target()
    }
    fn request_build_id(&self) -> &str {
        self.request_context.request_build_id()
    }
    fn spawn_service_protocol_identity(&self) -> &str {
        self.request_context.spawn_service_protocol_identity()
    }
    fn request_service_protocol_identity(&self) -> &str {
        self.request_context.request_service_protocol_identity()
    }
    fn operation_service_protocol_identity(&self) -> Option<&str> {
        self.request_context.operation_service_protocol_identity()
    }
    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        self.request_context.activation_identity()
    }
    fn trace_id(&self) -> Option<&str> {
        self.request_context.trace_id()
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(submit_spawn(
            self.request_context.clone(),
            request,
            args_payload,
            execution_control,
        ))
    }
}

struct RuntimeOwnedRequestCapabilityContext(RuntimeOwnedRequestParts);

impl capability_contract::ActorCapabilityApi for RuntimeOwnedRequestCapabilityContext {
    fn owned(&self) -> capability_contract::OwnedActorCapabilityContext {
        capability_contract::ActorCapabilityContext::new(RuntimeOwnedRequestCapabilityContext(
            self.0.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::ActorCapabilityContext<'_> {
        actor(
            concrete_actor_context_from_owned(&self.0),
            concrete_request_context_from_owned(&self.0),
            self.0.clone(),
        )
    }

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .get_or_create_in_scope(request, bootstrap_payload, scope)
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
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .replace_in_scope(request, bootstrap_payload, scope)
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
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .find_in_scope(request, scope)
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
            let scope = actor_execution_scope(&execution_control)?;
            root_result_into_capability(
                concrete::ActorClient::new(concrete_actor_context_from_owned(&self.0))
                    .remove_in_scope(request, scope)
                    .await,
            )
            .await
        })
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

impl capability_contract::RequestCapabilityApi for RuntimeOwnedRequestCapabilityContext {
    fn owned(&self) -> capability_contract::OwnedRequestCapabilityContext {
        capability_contract::RequestCapabilityContext::new(RuntimeOwnedRequestCapabilityContext(
            self.0.clone(),
        ))
    }

    fn borrow(&self) -> capability_contract::RequestCapabilityContext<'_> {
        request_capability(
            concrete_actor_context_from_owned(&self.0),
            concrete_request_context_from_owned(&self.0),
            self.0.clone(),
        )
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

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::CapabilityFuture<'a, ()> {
        Box::pin(submit_spawn(
            concrete_request_context_from_owned(&self.0),
            request,
            args_payload,
            execution_control,
        ))
    }
}

async fn invoke_actor_method(
    parts: RuntimeOwnedRequestParts,
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

    let scope = actor_execution_scope(&execution_control)?;
    if scope.terminal_at(std::time::Instant::now()).is_some() {
        return std::future::pending().await;
    }
    let (scope_lease, scope_completion) = scope.acquire_lease();
    let invocation_id = request.identity.invocation_id.clone();
    let cancellation_correlation = request.identity.cancellation_correlation.clone();
    let primitive_timeout_ms = request.deadline.timeout_ms;
    let wire_timeout_ms =
        actor_method_wire_timeout_ms(&scope, primitive_timeout_ms, std::time::Instant::now());
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
    let timeout_ms = i64::try_from(wire_timeout_ms).map_err(|_| {
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
                timeout_ms: wire_timeout_ms,
                expires_at,
            },
            cancellation_correlation: cancellation_correlation.clone(),
            trace_id: None,
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

    let response_committed = lease.response_committed();
    let send_cancel = |reason| {
        let cancel = ActorMethodFrame::Cancel(ActorMethodCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.cancel".to_string(),
            invocation_id: invocation_id.clone(),
            cancellation_correlation: cancellation_correlation.clone(),
            reason,
        });
        if let Ok(wire) = encode_actor_method_frame(&cancel) {
            let _ = sender.send(concrete::RouterWriterMessage::Binary(wire));
        }
    };
    tokio::select! {
        biased;
        outcome = lease.receive() => {
            let _ = scope_completion.complete();
            match outcome {
                Ok(Ok(outcome)) => Ok(outcome),
                Ok(Err(error)) => Err(capability_contract::CapabilityError::protocol(
                    "actor.method.invoke",
                    format!("Actor owner transport failure {}: {}", error.code, error.message),
                )),
                Err(_) => Err(capability_contract::CapabilityError::provider_unavailable(
                    "actor.method.invoke",
                    "Actor invocation response channel closed",
                )),
            }
        },
        _ = response_committed.wait() => {
            let outcome = lease.receive().await;
            let _ = scope_completion.complete();
            match outcome {
                Ok(Ok(outcome)) => Ok(outcome),
                Ok(Err(error)) => Err(capability_contract::CapabilityError::protocol(
                    "actor.method.invoke",
                    format!("Actor owner transport failure {}: {}", error.code, error.message),
                )),
                Err(_) => Err(capability_contract::CapabilityError::provider_unavailable(
                    "actor.method.invoke",
                    "Actor invocation response channel closed",
                )),
            }
        }
        terminal = scope_lease.wait() => {
            let capability_contract::ExecutionScopeLeaseTerminal::Control(terminal) = terminal else {
                unreachable!("scope completion is owned by the Actor response branch")
            };
            let reason = match terminal {
                capability_contract::ExecutionScopeTerminal::AncestorCancelled => {
                    ActorMethodCancelReason::Cancelled
                }
                capability_contract::ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                | capability_contract::ExecutionScopeTerminal::InheritedDeadlineExceeded(_) => {
                    ActorMethodCancelReason::DeadlineExceeded
                }
            };
            send_cancel(reason);
            drop(lease);
            std::future::pending().await
        }
        _ = parts.cancellation.wait_cancelled() => {
            send_cancel(ActorMethodCancelReason::Cancelled);
            Ok(capability_contract::ActorInvocationOutcome::Cancelled(
                capability_contract::ActorInvocationCancellation::Cancelled,
            ))
        }
        _ = tokio::time::sleep(tokio::time::Duration::from_millis(primitive_timeout_ms)) => {
            send_cancel(ActorMethodCancelReason::DeadlineExceeded);
            Ok(capability_contract::ActorInvocationOutcome::Cancelled(
                capability_contract::ActorInvocationCancellation::DeadlineExceeded,
            ))
        }
    }
}

fn actor_method_wire_timeout_ms(
    scope: &capability_contract::ExecutionScope,
    primitive_timeout_ms: u64,
    now: std::time::Instant,
) -> u64 {
    let Some(deadline) = scope.effective_deadline() else {
        return primitive_timeout_ms;
    };
    let remaining = deadline.at().saturating_duration_since(now);
    let remaining_ms = u64::try_from(remaining.as_millis())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::from(remaining.subsec_nanos() % 1_000_000 != 0))
        .max(1);
    primitive_timeout_ms.min(remaining_ms)
}

async fn submit_spawn(
    context: concrete::RequestClientContext<'_>,
    request: SpawnSubmitControlRequest,
    args_payload: Vec<u8>,
    execution_control: capability_contract::OwnedExecutionControl,
) -> capability_contract::CapabilityResult<()> {
    let scope = actor_execution_scope(&execution_control)?;
    root_result_into_capability(
        concrete::RequestClient::new(context)
            .submit_spawn_in_scope(request, args_payload, scope)
            .await,
    )
    .await
    .map(|_| ())
}

fn actor_execution_scope(
    execution_control: &capability_contract::OwnedExecutionControl,
) -> capability_contract::CapabilityResult<capability_contract::ExecutionScope> {
    execution_control.execution_scope().map_err(|error| {
        capability_contract::CapabilityError::protocol(
            "actor.current-scope",
            format!("current execution scope is unavailable: {error}"),
        )
    })
}

#[cfg(test)]
mod tests;
