use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use skiff_artifact_model::validate_activation_token;
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorControlDeadline, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorRemoveControlRequest, ActorReplaceControlRequest,
    CancellationToken, ExecutionScope, ExecutionScopeLeaseTerminal, ExecutionScopeTerminal,
    InvocationContext, OutboundControlMessage, OutboundRequestCancelSendError,
    OutboundRequestCancelSender, OutboundRequestLease, OutboundRequestRegistry, OutboundResponse,
    OutboundResponseReceiver, RequestCancelControl, RouterWriterMessage, TaskCallerKind,
    TaskCancelControlRequest, TaskCancelControlResponse, TaskStatusControlRequest,
    TaskStatusControlResponse, TaskSubmitControlMessage, TaskSubmitControlRequest,
    TaskSubmitResponseControl,
};
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};
use tokio::sync::mpsc;

use crate::error::{Result, RuntimeError};
use crate::telemetry::{telemetry_event, telemetry_timestamp_now, RequestTelemetryContext};
use skiff_runtime_boundary::value::decode_base64;
use skiff_runtime_model::runtime_value::ActorRef;
use skiff_runtime_transport::cancel_reason::request_cancel_wire_reason_for_internal;
use skiff_runtime_transport::protocol::{
    ActorFindResponseFrameHeader, ActorGetOrCreateResponseFrameHeader, ActorRefFrameMetadata,
    ActorRemoveResponseFrameHeader, ActorReplaceResponseFrameHeader, TaskCancelResponseFrameHeader,
    TaskControlRejectionCode, TaskRef, TaskStatusResponseFrameHeader, TaskSubmitRejectionCode,
    TaskSubmitResponseFrameHeader, TelemetryLevel, TelemetrySource, TelemetryTopic,
};

const ACTOR_GET_OR_CREATE_TARGET: &str = "actor.getOrCreate";
const ACTOR_REPLACE_TARGET: &str = "actor.replace";
const ACTOR_FIND_TARGET: &str = "actor.find";
const ACTOR_REMOVE_TARGET: &str = "actor.remove";
const TASK_SUBMIT_TARGET: &str = "task.submit";
const TASK_STATUS_TARGET: &str = "task.status";
const TASK_CANCEL_TARGET: &str = "task.cancel";
const ACTOR_GET_CREATE_DEADLINE_MS: u64 = 30_000;

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
        request.rpc_id = control_rpc_id(&self.context, ACTOR_GET_OR_CREATE_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_GET_OR_CREATE_TARGET)?;
        request.deadline = Some(actor_control_deadline(
            scope.as_ref(),
            std::time::Instant::now(),
        ));
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorGetOrCreate {
            request,
            payload: bootstrap_payload,
        };
        let response: ActorGetOrCreateResponseFrameHeader = send_control_request(
            &self.context,
            ACTOR_GET_OR_CREATE_TARGET,
            &rpc_id,
            command,
            scope,
        )
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
        request.rpc_id = control_rpc_id(&self.context, ACTOR_REPLACE_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_REPLACE_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorReplace {
            request,
            payload: bootstrap_payload,
        };
        let response: ActorReplaceResponseFrameHeader =
            send_control_request(&self.context, ACTOR_REPLACE_TARGET, &rpc_id, command, scope)
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
        request.rpc_id = control_rpc_id(&self.context, ACTOR_FIND_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_FIND_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorFind { request };
        let response: ActorFindResponseFrameHeader =
            send_control_request(&self.context, ACTOR_FIND_TARGET, &rpc_id, command, scope).await?;
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
        request.rpc_id = control_rpc_id(&self.context, ACTOR_REMOVE_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(ACTOR_REMOVE_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::ActorRemove { request };
        let response: ActorRemoveResponseFrameHeader =
            send_control_request(&self.context, ACTOR_REMOVE_TARGET, &rpc_id, command, scope)
                .await?;
        Ok(response.removed)
    }
}

pub struct RequestClient<'a> {
    context: RequestClientContext<'a>,
}

impl<'a> RequestClient<'a> {
    pub fn new(context: RequestClientContext<'a>) -> Self {
        Self { context }
    }

    pub async fn submit_task(
        &self,
        request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        caller_kind: TaskCallerKind,
    ) -> Result<TaskSubmitResponseControl> {
        self.submit_task_with_scope(request, args_payload, None, caller_kind)
            .await
    }

    pub(crate) async fn submit_task_in_scope(
        &self,
        request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        scope: ExecutionScope,
        caller_kind: TaskCallerKind,
    ) -> Result<TaskSubmitResponseControl> {
        self.submit_task_with_scope(request, args_payload, Some(scope), caller_kind)
            .await
    }

    async fn submit_task_with_scope(
        &self,
        mut request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        scope: Option<ExecutionScope>,
        caller_kind: TaskCallerKind,
    ) -> Result<TaskSubmitResponseControl> {
        request.rpc_id = control_rpc_id(&self.context, TASK_SUBMIT_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        request.activation_identity = self
            .context
            .current_activation_identity(TASK_SUBMIT_TARGET)?;
        let rpc_id = request.rpc_id.clone();
        let task_id = request.task_id.clone();
        let message = TaskSubmitControlMessage {
            request,
            payload: args_payload,
            caller_kind,
        };
        let response: TaskSubmitResponseFrameHeader = match send_task_submit_request(
            &self.context,
            TASK_SUBMIT_TARGET,
            &rpc_id,
            message,
            scope,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                match &error {
                    RuntimeError::TaskSubmitRejected { code, message } => {
                        self.context.emit_task_submit_event(
                            "task.submit.rejected",
                            TelemetryLevel::Warn,
                            task_id.as_deref(),
                            Some(code),
                            Some(message),
                        );
                    }
                    _ => {
                        self.context.emit_task_submit_event(
                            "task.submit.uncertain",
                            TelemetryLevel::Warn,
                            task_id.as_deref(),
                            None,
                            Some(&error.to_string()),
                        );
                    }
                }
                return Err(error);
            }
        };
        validate_task_submit_response(&response, &rpc_id)?;
        self.context.emit_task_submit_event(
            "task.submit.accepted",
            TelemetryLevel::Info,
            task_id.as_deref().or(Some(response.task_id.as_str())),
            None,
            None,
        );
        Ok(TaskSubmitResponseControl {
            task_ref: response.task_ref.into_string(),
            task_id: response.task_id,
            request_id: response.request_id,
        })
    }

    pub async fn status_task(
        &self,
        request: TaskStatusControlRequest,
    ) -> Result<TaskStatusControlResponse> {
        self.status_task_with_scope(request, None).await
    }

    pub(crate) async fn status_task_in_scope(
        &self,
        request: TaskStatusControlRequest,
        scope: ExecutionScope,
    ) -> Result<TaskStatusControlResponse> {
        self.status_task_with_scope(request, Some(scope)).await
    }

    async fn status_task_with_scope(
        &self,
        mut request: TaskStatusControlRequest,
        scope: Option<ExecutionScope>,
    ) -> Result<TaskStatusControlResponse> {
        request.rpc_id = control_rpc_id(&self.context, TASK_STATUS_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::TaskStatus { request };
        let response: TaskStatusResponseFrameHeader =
            send_control_request(&self.context, TASK_STATUS_TARGET, &rpc_id, command, scope)
                .await?;
        validate_task_status_response(&response, &rpc_id)?;
        Ok(TaskStatusControlResponse {
            task_ref: response.task_ref.into_string(),
            kind: response.status.kind.as_str().to_string(),
        })
    }

    pub async fn cancel_task(
        &self,
        request: TaskCancelControlRequest,
    ) -> Result<TaskCancelControlResponse> {
        self.cancel_task_with_scope(request, None).await
    }

    pub(crate) async fn cancel_task_in_scope(
        &self,
        request: TaskCancelControlRequest,
        scope: ExecutionScope,
    ) -> Result<TaskCancelControlResponse> {
        self.cancel_task_with_scope(request, Some(scope)).await
    }

    async fn cancel_task_with_scope(
        &self,
        mut request: TaskCancelControlRequest,
        scope: Option<ExecutionScope>,
    ) -> Result<TaskCancelControlResponse> {
        request.rpc_id = control_rpc_id(&self.context, TASK_CANCEL_TARGET);
        request.runtime_id = self.context.runtime_id().to_string();
        let rpc_id = request.rpc_id.clone();
        let command = OutboundControlMessage::TaskCancel { request };
        let response: TaskCancelResponseFrameHeader =
            send_control_request(&self.context, TASK_CANCEL_TARGET, &rpc_id, command, scope)
                .await?;
        validate_task_cancel_response(&response, &rpc_id)?;
        Ok(TaskCancelControlResponse {
            task_ref: response.task_ref.into_string(),
            kind: response.result.kind.as_str().to_string(),
        })
    }
}

#[derive(Clone)]
pub struct ActorClientContext<'a> {
    runtime_id: &'a str,
    request_id: &'a str,
    activation_identity: Option<&'a ActivationIdentityControl>,
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
            request_id: invocation.request_id(),
            activation_identity,
            router_sender,
            outbound_requests,
            cancellation,
        }
    }

    pub fn from_parts(
        runtime_id: &'a str,
        request_id: &'a str,
        activation_identity: Option<&'a ActivationIdentityControl>,
        router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
        outbound_requests: &'a OutboundRequestRegistry,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            runtime_id,
            request_id,
            activation_identity,
            router_sender,
            outbound_requests,
            cancellation,
        }
    }

    pub fn runtime_id(&self) -> &'a str {
        self.runtime_id
    }

    pub fn request_id(&self) -> &'a str {
        self.request_id
    }

    pub fn activation_identity(&self) -> Option<&'a ActivationIdentityControl> {
        self.activation_identity
    }
}

impl<'a, 'ctx> From<&'a RequestClientContext<'ctx>> for ActorClientContext<'a> {
    fn from(context: &'a RequestClientContext<'ctx>) -> Self {
        Self {
            runtime_id: context.runtime_id,
            request_id: context.request_id,
            activation_identity: context.activation_identity,
            router_sender: context.router_sender,
            outbound_requests: context.outbound_requests,
            cancellation: context.cancellation.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RequestClientContext<'a> {
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
    telemetry: Option<RequestTelemetryContext>,
    router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
    outbound_requests: &'a OutboundRequestRegistry,
    cancellation: CancellationToken,
}

impl<'a> RequestClientContext<'a> {
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
            operation_service_protocol_identity: Some(invocation.task_service_protocol_identity()),
            activation_identity,
            trace_id: invocation.trace_id(),
            telemetry: None,
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
        telemetry: Option<RequestTelemetryContext>,
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
            telemetry,
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

    pub fn task_service_protocol_identity(&self) -> &'a str {
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

    pub fn trace_id(&self) -> Option<&'a str> {
        self.trace_id
    }

    /// Attaches the request platform telemetry context (task submission
    /// observability). `None` keeps the client silent.
    pub fn with_telemetry(mut self, telemetry: Option<RequestTelemetryContext>) -> Self {
        self.telemetry = telemetry;
        self
    }

    fn emit_task_submit_event(
        &self,
        name: &str,
        level: TelemetryLevel,
        task_id: Option<&str>,
        code: Option<&str>,
        message: Option<&str>,
    ) {
        let Some(telemetry) = self.telemetry.as_ref() else {
            return;
        };
        let mut event = telemetry_event(
            TelemetryTopic::Log,
            telemetry_timestamp_now(),
            TelemetrySource::Runtime,
        );
        event.name = Some(name.to_string());
        event.level = Some(level);
        event.service_id = Some(self.service_id.to_string());
        event.runtime_id = Some(self.runtime_id.to_string());
        event.request_id = Some(self.request_id.to_string());
        event.trace_id = self.trace_id.map(str::to_string);
        event.target = Some(TASK_SUBMIT_TARGET.to_string());
        let mut attrs = Map::new();
        if let Some(task_id) = task_id {
            attrs.insert("taskId".to_string(), Value::String(task_id.to_string()));
        }
        if let Some(code) = code {
            attrs.insert("code".to_string(), Value::String(code.to_string()));
        }
        if let Some(message) = message {
            attrs.insert("reason".to_string(), Value::String(message.to_string()));
        }
        event.attrs = Some(attrs);
        let _ = telemetry.emit(event);
    }
}

trait ControlContext {
    fn request_id(&self) -> &str;
    fn current_activation_identity(&self, target: &str) -> Result<ActivationIdentityControl>;
    fn outbound_requests(&self) -> &OutboundRequestRegistry;
    fn open_outbound_response_lease(
        &self,
        request_id: &str,
    ) -> Result<(OutboundResponseReceiver, OutboundRequestLease)>;
    fn send_outbound_request(
        &self,
        request_id: &str,
        command: OutboundControlMessage,
    ) -> Result<()>;
    fn send_task_submit(&self, request_id: &str, message: TaskSubmitControlMessage) -> Result<()>;
    fn cancellation_token(&self) -> CancellationToken;
    fn outbound_cancel_sender(&self) -> Option<OutboundRequestCancelSender>;
}

impl ControlContext for ActorClientContext<'_> {
    fn request_id(&self) -> &str {
        self.request_id
    }

    fn outbound_requests(&self) -> &OutboundRequestRegistry {
        self.outbound_requests
    }

    fn current_activation_identity(&self, target: &str) -> Result<ActivationIdentityControl> {
        current_activation_identity(self.activation_identity, target)
    }

    fn open_outbound_response_lease(
        &self,
        request_id: &str,
    ) -> Result<(OutboundResponseReceiver, OutboundRequestLease)> {
        open_outbound_response_lease(self, request_id)
    }

    fn send_outbound_request(
        &self,
        request_id: &str,
        command: OutboundControlMessage,
    ) -> Result<()> {
        send_outbound_request(self.router_sender, request_id, command)
    }

    fn send_task_submit(&self, request_id: &str, message: TaskSubmitControlMessage) -> Result<()> {
        send_task_submit(self.router_sender, request_id, message)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn outbound_cancel_sender(&self) -> Option<OutboundRequestCancelSender> {
        outbound_cancel_sender(self.router_sender)
    }
}

impl ControlContext for RequestClientContext<'_> {
    fn request_id(&self) -> &str {
        self.request_id
    }

    fn outbound_requests(&self) -> &OutboundRequestRegistry {
        self.outbound_requests
    }

    fn current_activation_identity(&self, target: &str) -> Result<ActivationIdentityControl> {
        current_activation_identity(self.activation_identity, target)
    }

    fn open_outbound_response_lease(
        &self,
        request_id: &str,
    ) -> Result<(OutboundResponseReceiver, OutboundRequestLease)> {
        open_outbound_response_lease(self, request_id)
    }

    fn send_outbound_request(
        &self,
        request_id: &str,
        command: OutboundControlMessage,
    ) -> Result<()> {
        send_outbound_request(self.router_sender, request_id, command)
    }

    fn send_task_submit(&self, request_id: &str, message: TaskSubmitControlMessage) -> Result<()> {
        send_task_submit(self.router_sender, request_id, message)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn outbound_cancel_sender(&self) -> Option<OutboundRequestCancelSender> {
        outbound_cancel_sender(self.router_sender)
    }
}

fn current_activation_identity(
    activation_identity: Option<&ActivationIdentityControl>,
    target: &str,
) -> Result<ActivationIdentityControl> {
    activation_identity
        .cloned()
        .ok_or_else(|| RuntimeError::Protocol {
            target: target.to_string(),
            message: format!("{target} requires a current pinned ActivationContext"),
        })
}

fn open_outbound_response_lease(
    context: &impl ControlContext,
    request_id: &str,
) -> Result<(OutboundResponseReceiver, OutboundRequestLease)> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let lease = context.outbound_requests().insert_with_lease(
        request_id.to_string(),
        sender,
        context.outbound_cancel_sender(),
        "caller_cancel",
    )?;
    Ok((receiver, lease))
}

fn send_outbound_request(
    router_sender: Option<&mpsc::UnboundedSender<RouterWriterMessage>>,
    request_id: &str,
    command: OutboundControlMessage,
) -> Result<()> {
    let sender = router_sender.ok_or_else(|| RuntimeError::ProviderUnavailable {
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

fn send_task_submit(
    router_sender: Option<&mpsc::UnboundedSender<RouterWriterMessage>>,
    request_id: &str,
    message: TaskSubmitControlMessage,
) -> Result<()> {
    let sender = router_sender.ok_or_else(|| RuntimeError::ProviderUnavailable {
        target: request_id.to_string(),
        reason: "router writer is not available".to_string(),
    })?;
    sender
        .send(RouterWriterMessage::TaskSubmit(message))
        .map_err(|_| RuntimeError::ProviderUnavailable {
            target: request_id.to_string(),
            reason: "router writer channel closed".to_string(),
        })
}

fn outbound_cancel_sender(
    router_sender: Option<&mpsc::UnboundedSender<RouterWriterMessage>>,
) -> Option<OutboundRequestCancelSender> {
    let sender = router_sender.cloned()?;
    Some(std::sync::Arc::new(move |request_id, reason| {
        sender
            .send(cancel_message(request_id, reason))
            .map_err(|_| OutboundRequestCancelSendError::Closed)
    }))
}

async fn send_control_request<C, TResponse>(
    context: &C,
    target: &str,
    rpc_id: &str,
    command: OutboundControlMessage,
    scope: Option<ExecutionScope>,
) -> Result<TResponse>
where
    C: ControlContext,
    TResponse: DeserializeOwned,
{
    let payload = send_raw_control_request(context, target, rpc_id, command, scope).await?;
    serde_json::from_slice(&payload).map_err(|error| {
        RuntimeError::decode_target(
            target,
            format!("control response header is not valid JSON: {error}"),
        )
    })
}

async fn send_raw_control_request<C: ControlContext>(
    context: &C,
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
    let (response_rx, lease) = context.open_outbound_response_lease(rpc_id)?;
    if let Err(error) = context.send_outbound_request(rpc_id, command) {
        let _ = lease.cancel("runtime_disconnect");
        return Err(error);
    }

    match scope {
        Some(scope) => {
            await_control_response_in_scope(context, target, lease, response_rx, scope).await
        }
        None => await_control_response(context, target, lease, response_rx).await,
    }
}

async fn send_task_submit_request<C, TResponse>(
    context: &C,
    target: &str,
    rpc_id: &str,
    message: TaskSubmitControlMessage,
    scope: Option<ExecutionScope>,
) -> Result<TResponse>
where
    C: ControlContext,
    TResponse: DeserializeOwned,
{
    let payload = send_raw_task_submit_request(context, target, rpc_id, message, scope).await?;
    serde_json::from_slice(&payload).map_err(|error| {
        RuntimeError::decode_target(
            target,
            format!("control response header is not valid JSON: {error}"),
        )
    })
}

async fn send_raw_task_submit_request<C: ControlContext>(
    context: &C,
    target: &str,
    rpc_id: &str,
    message: TaskSubmitControlMessage,
    scope: Option<ExecutionScope>,
) -> Result<Vec<u8>> {
    if scope
        .as_ref()
        .and_then(|scope| scope.terminal_at(std::time::Instant::now()))
        .is_some()
    {
        return Err(RuntimeError::cancelled());
    }
    let (response_rx, lease) = context.open_outbound_response_lease(rpc_id)?;
    if let Err(error) = context.send_task_submit(rpc_id, message) {
        let _ = lease.cancel("runtime_disconnect");
        return Err(error);
    }

    match scope {
        Some(scope) => {
            await_control_response_in_scope(context, target, lease, response_rx, scope).await
        }
        None => await_control_response(context, target, lease, response_rx).await,
    }
}

fn control_rpc_id<C: ControlContext>(context: &C, target: &str) -> String {
    format!(
        "{}:{}:{}",
        context.request_id(),
        target,
        uuid::Uuid::new_v4()
    )
}

async fn await_control_response<C: ControlContext>(
    context: &C,
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

async fn await_control_response_in_scope<C: ControlContext>(
    context: &C,
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
            if target == TASK_STATUS_TARGET || target == TASK_CANCEL_TARGET {
                if let Some(code) = TaskControlRejectionCode::parse(&error.code) {
                    return Err(RuntimeError::TaskControlRejected {
                        code: code.as_str().to_string(),
                        message: error.message,
                    });
                }
            } else if let Some(code) = TaskSubmitRejectionCode::parse(&error.code) {
                return Err(RuntimeError::TaskSubmitRejected {
                    code: code.as_str().to_string(),
                    message: error.message,
                });
            }
            Err(RuntimeError::ProviderUnavailable {
                target: target.to_string(),
                reason: error.message,
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

async fn wait_request_cancelled<C: ControlContext>(context: &C) {
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

fn validate_task_submit_response(
    response: &TaskSubmitResponseFrameHeader,
    expected_rpc_id: &str,
) -> Result<()> {
    if response.rpc_id != expected_rpc_id {
        return Err(RuntimeError::Protocol {
            target: TASK_SUBMIT_TARGET.to_string(),
            message: format!(
                "task.submit.response rpcId {} does not match request {}",
                response.rpc_id, expected_rpc_id
            ),
        });
    }
    if response.status != "submitted" {
        return Err(RuntimeError::Protocol {
            target: TASK_SUBMIT_TARGET.to_string(),
            message: format!(
                "task.submit.response status must be submitted, got {}",
                response.status
            ),
        });
    }
    validate_task_submit_identity("taskId", &response.task_id)?;
    validate_task_submit_identity("requestId", &response.request_id)
}

fn validate_task_submit_identity(label: &str, value: &str) -> Result<()> {
    validate_activation_token(value, label).map_err(|message| RuntimeError::Protocol {
        target: TASK_SUBMIT_TARGET.to_string(),
        message: format!("task.submit.response {message}"),
    })
}

fn validate_task_status_response(
    response: &TaskStatusResponseFrameHeader,
    expected_rpc_id: &str,
) -> Result<()> {
    if response.rpc_id != expected_rpc_id {
        return Err(RuntimeError::Protocol {
            target: TASK_STATUS_TARGET.to_string(),
            message: format!(
                "task.status.response rpcId {} does not match request {}",
                response.rpc_id, expected_rpc_id
            ),
        });
    }
    TaskRef::parse(response.task_ref.as_str()).map_err(|error| RuntimeError::Protocol {
        target: TASK_STATUS_TARGET.to_string(),
        message: format!("task.status.response {error}"),
    })?;
    Ok(())
}

fn validate_task_cancel_response(
    response: &TaskCancelResponseFrameHeader,
    expected_rpc_id: &str,
) -> Result<()> {
    if response.rpc_id != expected_rpc_id {
        return Err(RuntimeError::Protocol {
            target: TASK_CANCEL_TARGET.to_string(),
            message: format!(
                "task.cancel.response rpcId {} does not match request {}",
                response.rpc_id, expected_rpc_id
            ),
        });
    }
    TaskRef::parse(response.task_ref.as_str()).map_err(|error| RuntimeError::Protocol {
        target: TASK_CANCEL_TARGET.to_string(),
        message: format!("task.cancel.response {error}"),
    })?;
    Ok(())
}

fn actor_control_deadline(
    scope: Option<&ExecutionScope>,
    now: std::time::Instant,
) -> ActorControlDeadline {
    let timeout_ms = match scope.and_then(|scope| scope.effective_deadline()) {
        Some(deadline) => {
            let remaining = deadline.at().saturating_duration_since(now);
            let remaining_ms = u64::try_from(remaining.as_millis())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::from(remaining.subsec_nanos() % 1_000_000 != 0))
                .max(1);
            ACTOR_GET_CREATE_DEADLINE_MS.min(remaining_ms)
        }
        None => ACTOR_GET_CREATE_DEADLINE_MS,
    };
    let expires_at = (OffsetDateTime::now_utc()
        + TimeDuration::milliseconds(i64::try_from(timeout_ms).unwrap_or(i64::MAX)))
    .format(&Rfc3339)
    .unwrap_or_else(|_| String::new());
    ActorControlDeadline {
        timeout_ms,
        expires_at,
    }
}

#[cfg(test)]
mod tests;
