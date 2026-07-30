use serde::de::DeserializeOwned;
use skiff_artifact_model::validate_activation_token;
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationToken, ExecutionScope,
    ExecutionScopeLeaseTerminal, ExecutionScopeTerminal, InvocationContext, OutboundControlMessage,
    OutboundRequestCancelSendError, OutboundRequestCancelSender, OutboundRequestLease,
    OutboundRequestRegistry, OutboundResponse, OutboundResponseReceiver, RequestCancelControl,
    RouterWriterMessage, SpawnSubmitControlRequest,
};
use tokio::sync::mpsc;

use crate::error::{Result, RuntimeError};
use skiff_runtime_boundary::value::decode_base64;
use skiff_runtime_model::runtime_value::ActorRef;
use skiff_runtime_transport::cancel_reason::request_cancel_wire_reason_for_internal;
use skiff_runtime_transport::protocol::{
    ActorFindResponseFrameHeader, ActorGetOrCreateResponseFrameHeader, ActorRefFrameMetadata,
    ActorRemoveResponseFrameHeader, ActorReplaceResponseFrameHeader,
    SpawnSubmitResponseFrameHeader,
};

const ACTOR_GET_OR_CREATE_TARGET: &str = "actor.getOrCreate";
const ACTOR_REPLACE_TARGET: &str = "actor.replace";
const ACTOR_FIND_TARGET: &str = "actor.find";
const ACTOR_REMOVE_TARGET: &str = "actor.remove";
const SPAWN_SUBMIT_TARGET: &str = "spawn.submit";

pub struct ActorClient<'a> {
    context: ActorClientContext<'a>,
}

impl<'a> ActorClient<'a> {
    pub fn new(context: impl Into<ActorClientContext<'a>>) -> Self {
        Self {
            context: context.into(),
        }
    }

    pub async fn get_or_create(
        &self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> Result<ActorRef> {
        self.get_or_create_with_scope(request, bootstrap_payload, None)
            .await
    }

    pub(crate) async fn get_or_create_in_scope(
        &self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        scope: ExecutionScope,
    ) -> Result<ActorRef> {
        self.get_or_create_with_scope(request, bootstrap_payload, Some(scope))
            .await
    }

    async fn get_or_create_with_scope(
        &self,
        mut request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
        scope: Option<ExecutionScope>,
    ) -> Result<ActorRef> {
        request.rpc_id = self.control_rpc_id(ACTOR_GET_OR_CREATE_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_GET_OR_CREATE_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorGetOrCreate {
            request,
            payload: bootstrap_payload,
        };
        let response: ActorGetOrCreateResponseFrameHeader = self
            .send_control_request(ACTOR_GET_OR_CREATE_TARGET, &rpc_id, command, scope)
            .await?;
        Ok(actor_ref_from_metadata(response.actor_ref)?)
    }

    pub async fn replace(
        &self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> Result<ActorRef> {
        self.replace_with_scope(request, bootstrap_payload, None)
            .await
    }

    pub(crate) async fn replace_in_scope(
        &self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        scope: ExecutionScope,
    ) -> Result<ActorRef> {
        self.replace_with_scope(request, bootstrap_payload, Some(scope))
            .await
    }

    async fn replace_with_scope(
        &self,
        mut request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
        scope: Option<ExecutionScope>,
    ) -> Result<ActorRef> {
        request.rpc_id = self.control_rpc_id(ACTOR_REPLACE_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_REPLACE_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorReplace {
            request,
            payload: bootstrap_payload,
        };
        let response: ActorReplaceResponseFrameHeader = self
            .send_control_request(ACTOR_REPLACE_TARGET, &rpc_id, command, scope)
            .await?;
        Ok(actor_ref_from_metadata(response.actor_ref)?)
    }

    pub async fn find(&self, request: ActorFindControlRequest) -> Result<Option<ActorRef>> {
        self.find_with_scope(request, None).await
    }

    pub(crate) async fn find_in_scope(
        &self,
        request: ActorFindControlRequest,
        scope: ExecutionScope,
    ) -> Result<Option<ActorRef>> {
        self.find_with_scope(request, Some(scope)).await
    }

    async fn find_with_scope(
        &self,
        mut request: ActorFindControlRequest,
        scope: Option<ExecutionScope>,
    ) -> Result<Option<ActorRef>> {
        request.rpc_id = self.control_rpc_id(ACTOR_FIND_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_FIND_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorFind { request };
        let response: ActorFindResponseFrameHeader = self
            .send_control_request(ACTOR_FIND_TARGET, &rpc_id, command, scope)
            .await?;
        if !response.found {
            return Ok(None);
        }
        let actor_ref = response.actor_ref.ok_or_else(|| RuntimeError::Protocol {
            target: ACTOR_FIND_TARGET.to_string(),
            message: "actor.find.response found=true missing actorRef".to_string(),
        })?;
        Ok(Some(actor_ref_from_metadata(actor_ref)?))
    }

    pub async fn remove(&self, request: ActorRemoveControlRequest) -> Result<bool> {
        self.remove_with_scope(request, None).await
    }

    pub(crate) async fn remove_in_scope(
        &self,
        request: ActorRemoveControlRequest,
        scope: ExecutionScope,
    ) -> Result<bool> {
        self.remove_with_scope(request, Some(scope)).await
    }

    async fn remove_with_scope(
        &self,
        mut request: ActorRemoveControlRequest,
        scope: Option<ExecutionScope>,
    ) -> Result<bool> {
        request.rpc_id = self.control_rpc_id(ACTOR_REMOVE_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_REMOVE_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorRemove { request };
        let response: ActorRemoveResponseFrameHeader = self
            .send_control_request(ACTOR_REMOVE_TARGET, &rpc_id, command, scope)
            .await?;
        Ok(response.removed)
    }

    pub async fn submit_spawn(
        &self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> Result<SpawnSubmitResponseFrameHeader> {
        self.submit_spawn_with_scope(request, args_payload, None)
            .await
    }

    pub(crate) async fn submit_spawn_in_scope(
        &self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        scope: ExecutionScope,
    ) -> Result<SpawnSubmitResponseFrameHeader> {
        self.submit_spawn_with_scope(request, args_payload, Some(scope))
            .await
    }

    async fn submit_spawn_with_scope(
        &self,
        mut request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
        scope: Option<ExecutionScope>,
    ) -> Result<SpawnSubmitResponseFrameHeader> {
        request.rpc_id = self.control_rpc_id(SPAWN_SUBMIT_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(SPAWN_SUBMIT_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::SpawnSubmit {
            request,
            payload: args_payload,
        };
        let response: SpawnSubmitResponseFrameHeader = self
            .send_control_request(SPAWN_SUBMIT_TARGET, &rpc_id, command, scope)
            .await?;
        validate_spawn_submit_response(&response, &rpc_id)?;
        Ok(response)
    }

    async fn send_control_request<TResponse>(
        &self,
        target: &str,
        rpc_id: &str,
        command: OutboundControlMessage,
        scope: Option<ExecutionScope>,
    ) -> Result<TResponse>
    where
        TResponse: DeserializeOwned,
    {
        let payload = self
            .send_raw_control_request(target, rpc_id, command, scope)
            .await?;
        serde_json::from_slice(&payload).map_err(|error| {
            RuntimeError::decode_target(
                target,
                format!("control response header is not valid JSON: {error}"),
            )
        })
    }

    async fn send_raw_control_request(
        &self,
        target: &str,
        rpc_id: &str,
        command: OutboundControlMessage,
        scope: Option<ExecutionScope>,
    ) -> Result<Vec<u8>> {
        if scope
            .as_ref()
            .and_then(|scope| scope.terminal_at(std::time::Instant::now()))
            .is_some()
        {
            return Err(RuntimeError::cancelled());
        }
        let (response_rx, lease) = self.context.open_outbound_response_lease(rpc_id)?;
        if let Err(error) = self.context.send_outbound_request(rpc_id, command) {
            let _ = lease.cancel("runtime_disconnect");
            return Err(error);
        }

        match scope {
            Some(scope) => {
                await_control_response_in_scope(&self.context, target, lease, response_rx, scope)
                    .await
            }
            None => await_control_response(&self.context, target, lease, response_rx).await,
        }
    }

    fn control_rpc_id(&self, target: &str) -> String {
        format!(
            "{}:{}:{}",
            self.context.request_id(),
            target,
            uuid::Uuid::new_v4()
        )
    }
}

#[derive(Clone)]
pub struct ActorClientContext<'a> {
    runtime_id: &'a str,
    service_id: &'a str,
    service_version: &'a str,
    request_id: &'a str,
    request_target: &'a str,
    request_build_id: &'a str,
    request_service_protocol_identity: &'a str,
    operation_service_protocol_identity: Option<&'a str>,
    activation_identity: Option<&'a ActivationIdentityControl>,
    trace_id: Option<&'a str>,
    router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
    outbound_requests: &'a OutboundRequestRegistry,
    cancellation: CancellationToken,
}

pub type ActorCapabilityContext<'a> = ActorClientContext<'a>;

impl<'a> ActorClientContext<'a> {
    pub fn new(
        invocation: InvocationContext<'a>,
        activation_identity: Option<&'a ActivationIdentityControl>,
        router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
        outbound_requests: &'a OutboundRequestRegistry,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            runtime_id: invocation.runtime_id(),
            service_id: invocation.service_id(),
            service_version: invocation.service_version(),
            request_id: invocation.request_id(),
            request_target: invocation.request_target(),
            request_build_id: invocation.request_build_id(),
            request_service_protocol_identity: invocation.actor_service_protocol_identity(),
            operation_service_protocol_identity: Some(invocation.spawn_service_protocol_identity()),
            activation_identity,
            trace_id: invocation.trace_id(),
            router_sender,
            outbound_requests,
            cancellation,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        runtime_id: &'a str,
        service_id: &'a str,
        service_version: &'a str,
        request_id: &'a str,
        request_target: &'a str,
        request_build_id: &'a str,
        request_service_protocol_identity: &'a str,
        operation_service_protocol_identity: Option<&'a str>,
        activation_identity: Option<&'a ActivationIdentityControl>,
        trace_id: Option<&'a str>,
        router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
        outbound_requests: &'a OutboundRequestRegistry,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            runtime_id,
            service_id,
            service_version,
            request_id,
            request_target,
            request_build_id,
            request_service_protocol_identity,
            operation_service_protocol_identity,
            activation_identity,
            trace_id,
            router_sender,
            outbound_requests,
            cancellation,
        }
    }

    pub fn runtime_id(&self) -> &'a str {
        self.runtime_id
    }

    pub fn service_id(&self) -> &'a str {
        self.service_id
    }

    pub fn service_version(&self) -> &'a str {
        self.service_version
    }

    pub fn request_id(&self) -> &'a str {
        self.request_id
    }

    pub fn request_target(&self) -> &'a str {
        self.request_target
    }

    pub fn request_build_id(&self) -> &'a str {
        self.request_build_id
    }

    pub fn spawn_service_protocol_identity(&self) -> &'a str {
        self.operation_service_protocol_identity
            .unwrap_or(self.request_service_protocol_identity)
    }

    pub fn request_service_protocol_identity(&self) -> &'a str {
        self.request_service_protocol_identity
    }

    pub fn operation_service_protocol_identity(&self) -> Option<&'a str> {
        self.operation_service_protocol_identity
    }

    pub fn activation_identity(&self) -> Option<&'a ActivationIdentityControl> {
        self.activation_identity
    }

    fn current_activation_identity(&self, target: &str) -> Result<ActivationIdentityControl> {
        self.activation_identity
            .cloned()
            .ok_or_else(|| RuntimeError::Protocol {
                target: target.to_string(),
                message: format!("{target} requires a current pinned ActivationContext"),
            })
    }

    pub fn trace_id(&self) -> Option<&'a str> {
        self.trace_id
    }

    fn open_outbound_response_lease(
        &self,
        request_id: &str,
    ) -> Result<(OutboundResponseReceiver, OutboundRequestLease)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let lease = self.outbound_requests.insert_with_lease(
            request_id.to_string(),
            sender,
            self.outbound_cancel_sender(),
            "caller_cancel",
        )?;
        Ok((receiver, lease))
    }

    fn send_outbound_request(
        &self,
        request_id: &str,
        command: OutboundControlMessage,
    ) -> Result<()> {
        let sender = self
            .router_sender
            .ok_or_else(|| RuntimeError::ProviderUnavailable {
                target: request_id.to_string(),
                reason: "router writer is not available".to_string(),
            })?;
        sender
            .send(RouterWriterMessage::Control(command))
            .map_err(|_| RuntimeError::ProviderUnavailable {
                target: request_id.to_string(),
                reason: "router writer channel closed".to_string(),
            })
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn outbound_cancel_sender(&self) -> Option<OutboundRequestCancelSender> {
        let sender = self.router_sender.cloned()?;
        Some(std::sync::Arc::new(move |request_id, reason| {
            sender
                .send(cancel_message(request_id, reason))
                .map_err(|_| OutboundRequestCancelSendError::Closed)
        }))
    }
}

async fn await_control_response(
    context: &ActorClientContext<'_>,
    target: &str,
    lease: OutboundRequestLease,
    mut receiver: OutboundResponseReceiver,
) -> Result<Vec<u8>> {
    let response_committed = lease.terminal_signal();
    tokio::select! {
        biased;
        _ = response_committed.wait_terminal() => {
            finish_control_response(target, &lease, receiver.recv().await)
        }
        _ = wait_request_cancelled(context) => {
            let (result, _) = cancel_control_response_or_receive(
                target,
                &lease,
                &mut receiver,
                "caller_cancel",
            ).await;
            result
        }
    }
}

async fn await_control_response_in_scope(
    context: &ActorClientContext<'_>,
    target: &str,
    lease: OutboundRequestLease,
    mut receiver: OutboundResponseReceiver,
    scope: ExecutionScope,
) -> Result<Vec<u8>> {
    let response_committed = lease.terminal_signal();
    let (scope_lease, scope_completion) = scope.acquire_lease();
    tokio::select! {
        biased;
        _ = response_committed.wait_terminal() => {
            let result = finish_control_response(target, &lease, receiver.recv().await);
            let _ = scope_completion.complete();
            result
        }
        terminal = scope_lease.wait() => {
            let ExecutionScopeLeaseTerminal::Control(terminal) = terminal else {
                unreachable!("scope completion is owned by the response branch")
            };
            let (result, response_won) = cancel_control_response_or_receive(
                target,
                &lease,
                &mut receiver,
                scope_cancel_reason(&terminal),
            ).await;
            if response_won {
                let _ = scope_completion.complete();
            }
            result
        }
        _ = wait_request_cancelled(context) => {
            let (result, response_won) = cancel_control_response_or_receive(
                target,
                &lease,
                &mut receiver,
                "caller_cancel",
            ).await;
            if response_won {
                let _ = scope_completion.complete();
            }
            result
        }
    }
}

async fn cancel_control_response_or_receive(
    target: &str,
    lease: &OutboundRequestLease,
    receiver: &mut OutboundResponseReceiver,
    reason: &str,
) -> (Result<Vec<u8>>, bool) {
    if lease.cancel(reason) {
        (Err(RuntimeError::cancelled()), false)
    } else {
        (
            finish_control_response(target, lease, receiver.recv().await),
            true,
        )
    }
}

fn finish_control_response(
    target: &str,
    lease: &OutboundRequestLease,
    result: Option<OutboundResponse>,
) -> Result<Vec<u8>> {
    match result {
        Some(OutboundResponse::End { payload }) => {
            lease.complete();
            Ok(payload)
        }
        Some(OutboundResponse::Error(error)) => {
            lease.complete();
            Err(RuntimeError::ProviderUnavailable {
                target: target.to_string(),
                reason: error.message,
            })
        }
        Some(other) => {
            let _ = lease.cancel("unexpected_control_response");
            Err(RuntimeError::ProviderUnavailable {
                target: target.to_string(),
                reason: format!("control RPC received {}", other.kind()),
            })
        }
        None => {
            let _ = lease.cancel("response_channel_closed");
            Err(RuntimeError::ProviderUnavailable {
                target: target.to_string(),
                reason: "control response channel closed".to_string(),
            })
        }
    }
}

fn scope_cancel_reason(terminal: &ExecutionScopeTerminal) -> &'static str {
    match terminal {
        ExecutionScopeTerminal::AncestorCancelled => "caller_cancel",
        ExecutionScopeTerminal::LocalDeadlineExceeded(_)
        | ExecutionScopeTerminal::InheritedDeadlineExceeded(_) => "deadline_exceeded",
    }
}

async fn wait_request_cancelled(context: &ActorClientContext<'_>) {
    context.cancellation_token().wait_cancelled().await;
}

fn cancel_message(request_id: &str, reason: &str) -> RouterWriterMessage {
    RouterWriterMessage::Control(OutboundControlMessage::RequestCancel {
        request: RequestCancelControl {
            request_id: request_id.to_string(),
            reason: request_cancel_wire_reason_for_internal(reason).to_string(),
        },
    })
}

fn actor_ref_from_metadata(frame: ActorRefFrameMetadata) -> Result<ActorRef> {
    let canonical_actor_id_key_bytes = decode_base64(&frame.canonical_actor_id_key_bytes_base64)
        .map_err(|error| {
            RuntimeError::decode_target(
                "actorRef",
                format!("canonicalActorIdKeyBytesBase64 is invalid: {error}"),
            )
        })?;
    Ok(ActorRef::new(
        frame.service_id,
        frame.actor_type_identity,
        frame.actor_id_type_identity,
        frame.actor_id_encoding_version,
        canonical_actor_id_key_bytes,
        frame.actor_id_hash,
        frame.epoch,
    ))
}

fn validate_spawn_submit_response(
    response: &SpawnSubmitResponseFrameHeader,
    expected_rpc_id: &str,
) -> Result<()> {
    if response.rpc_id != expected_rpc_id {
        return Err(RuntimeError::Protocol {
            target: SPAWN_SUBMIT_TARGET.to_string(),
            message: format!(
                "spawn.submit.response rpcId {} does not match request {}",
                response.rpc_id, expected_rpc_id
            ),
        });
    }
    if response.status != "submitted" {
        return Err(RuntimeError::Protocol {
            target: SPAWN_SUBMIT_TARGET.to_string(),
            message: format!(
                "spawn.submit.response status must be submitted, got {}",
                response.status
            ),
        });
    }
    validate_spawn_submit_identity("spawnId", &response.spawn_id)?;
    validate_spawn_submit_identity("requestId", &response.request_id)
}

fn validate_spawn_submit_identity(label: &str, value: &str) -> Result<()> {
    validate_activation_token(value, label).map_err(|message| RuntimeError::Protocol {
        target: SPAWN_SUBMIT_TARGET.to_string(),
        message: format!("spawn.submit.response {message}"),
    })
}

#[cfg(test)]
mod tests;
