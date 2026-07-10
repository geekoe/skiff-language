use std::{collections::HashMap, future::Future, pin::Pin};

use serde_json::Value;
use skiff_runtime_boundary::{
    binary::{decode_payload_plan, encode_payload_plan},
    payload::{PayloadBoundary, PayloadBoundaryKind, PayloadServiceRef},
};
use skiff_runtime_capability_context::{
    OutboundRequestLease, OutboundResponse, OutboundResponseReceiver, RequestEffectDoubleControl,
    StreamRuntimeError, StreamRuntimeResult,
};
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, ServiceDependencyConstraint, ServiceDependencySymbolRef,
};
use skiff_runtime_linked_type_plan::{ProgramTypeView, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{RuntimeObjectFields, RuntimeValue},
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
};

use super::{
    capabilities::{OutboundServiceContext, OutboundServiceRequestStart, StreamPullSource},
    env::Env,
    runtime_ops::{
        runtime_coerce_required_plan, runtime_from_wire_internal_handle_required_plan,
        runtime_object_from_fields, runtime_to_wire_required_plan,
    },
    Interpreter,
};
use crate::error::{Result, RuntimeError};
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_capability_context::RequestStartControl;

pub async fn call_outbound_service(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
    heap: &mut RequestHeap,
    env: &Env,
    caller_addr: &ExecutableAddr,
    call: &CallIr,
    symbol: &ServiceDependencySymbolRef,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let _ = call;
    let dispatch = OutboundServiceDispatch::from_call(
        interpreter,
        caller_addr,
        symbol,
        context.service_dependencies(),
    )?;
    send_outbound_service_request(interpreter, context, heap, env, &dispatch, args).await
}

pub async fn call_outbound_service_operation(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
    heap: &mut RequestHeap,
    env: &Env,
    caller_addr: &ExecutableAddr,
    dependency_ref: &str,
    operation_abi_id: &str,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let dispatch = OutboundServiceDispatch::from_dependency_operation_abi(
        interpreter,
        caller_addr,
        dependency_ref,
        operation_abi_id,
        context.service_dependencies(),
    )?;
    send_outbound_service_request(interpreter, context, heap, env, &dispatch, args).await
}

async fn send_outbound_service_request(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
    heap: &mut RequestHeap,
    env: &Env,
    dispatch: &OutboundServiceDispatch,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    if dispatch.mode != "unary" && dispatch.mode != "serverStream" {
        return Err(RuntimeError::Unsupported(format!(
            "outbound service call {} mode {} is not supported",
            dispatch.target, dispatch.mode
        )));
    }
    if context
        .effective_timeout_ms(dispatch.timeout_ms)
        .is_some_and(|timeout| timeout == 0)
    {
        return Err(context.outbound_deadline_error().into());
    }

    let payload = encode_outbound_request_payload(dispatch, &args, heap)?;
    let started = context.start_request(
        outbound_request_start(interpreter, context, dispatch),
        payload,
    )?;

    let value = if dispatch.mode == "serverStream" {
        outbound_service_stream_value(
            interpreter,
            context,
            dispatch,
            started.lease,
            started.response_rx,
            heap,
        )?
    } else {
        let response =
            await_outbound_response(context, dispatch, started.lease, started.response_rx).await?;
        let boundary = PayloadBoundary::cross_service(
            PayloadBoundaryKind::InboundServiceCall,
            dispatch.service_ref(),
        );
        let value =
            decode_payload_plan(&response.payload, &dispatch.response_plan, &boundary, heap)?;
        runtime_coerce_required_plan(
            &value,
            &dispatch.response_plan,
            &format!("{} response", dispatch.target),
            heap,
        )?
    };
    if env
        .stream_sink
        .as_ref()
        .is_some_and(|sink| sink.is_cancelled())
    {
        return Err(RuntimeError::Cancelled);
    }
    Ok(value)
}

async fn await_outbound_response(
    context: &OutboundServiceContext,
    dispatch: &OutboundServiceDispatch,
    lease: OutboundRequestLease,
    mut receiver: OutboundResponseReceiver,
) -> Result<OutboundServiceResponse> {
    let timeout = context.effective_timeout_ms(dispatch.timeout_ms);
    let response = match context
        .receive_response(&lease, &dispatch.target, &mut receiver, timeout)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            lease.cancel("response_channel_closed");
            return Err(error);
        }
    };
    match response {
        response @ (OutboundResponse::End { .. } | OutboundResponse::Error(_)) => {
            lease.complete();
            outbound_router_response_into_result(response, &dispatch.target)
        }
        other => {
            lease.cancel("unexpected_stream_response");
            Err(RuntimeError::ProviderUnavailable {
                target: dispatch.target.clone(),
                reason: format!("unary outbound service call received {}", other.kind()),
            })
        }
    }
}

fn outbound_service_stream_value(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
    dispatch: &OutboundServiceDispatch,
    lease: OutboundRequestLease,
    receiver: OutboundResponseReceiver,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let stream_value = interpreter.stream_runtime.pull_stream_with_cancellation(
        OutboundServiceStreamSource {
            lease,
            context: context.clone(),
            target: dispatch.target.clone(),
            target_service: dispatch.service_ref(),
            receiver,
            item_plan: dispatch.response_plan.clone(),
            next_seq: 0,
            started: false,
        },
        context.cancel_signal(),
    );
    runtime_from_wire_internal_handle_required_plan(
        &stream_value,
        Some(&dispatch.stream_return_plan()?),
        &format!("{} response stream", dispatch.target),
        heap,
    )
}

struct OutboundServiceStreamSource {
    lease: OutboundRequestLease,
    context: OutboundServiceContext,
    target: String,
    target_service: PayloadServiceRef,
    receiver: OutboundResponseReceiver,
    item_plan: RuntimeTypePlan,
    next_seq: u64,
    started: bool,
}

impl StreamPullSource for OutboundServiceStreamSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(async move { self.next_item().await })
    }
}

impl OutboundServiceStreamSource {
    async fn next_item(&mut self) -> StreamRuntimeResult<Option<Value>> {
        loop {
            let Some(response) = self.receiver.recv().await else {
                self.abort_pending("response_channel_closed");
                return Err(StreamRuntimeError::producer(
                    RuntimeError::ProviderUnavailable {
                        target: self.target.clone(),
                        reason: "outbound response channel closed".to_string(),
                    },
                ));
            };
            match response {
                OutboundResponse::Start { http_response } => {
                    let _ = http_response;
                    if self.started {
                        self.abort_pending("duplicate_response_start");
                        return Err(StreamRuntimeError::producer(
                            RuntimeError::ProviderUnavailable {
                                target: self.target.clone(),
                                reason: "outbound response.start received more than once"
                                    .to_string(),
                            },
                        ));
                    }
                    self.started = true;
                }
                OutboundResponse::Chunk { seq, payload } => {
                    if !self.started {
                        self.abort_pending("chunk_before_start");
                        return Err(StreamRuntimeError::producer(
                            RuntimeError::ProviderUnavailable {
                                target: self.target.clone(),
                                reason: "outbound response.chunk received before response.start"
                                    .to_string(),
                            },
                        ));
                    }
                    if seq != self.next_seq {
                        self.abort_pending("chunk_seq_mismatch");
                        return Err(StreamRuntimeError::producer(
                            RuntimeError::ProviderUnavailable {
                                target: self.target.clone(),
                                reason: format!(
                                "outbound response.chunk seq {seq} does not match expected seq {}",
                                self.next_seq
                            ),
                            },
                        ));
                    }
                    self.next_seq += 1;
                    return match self.decode_chunk(payload) {
                        Ok(value) => Ok(Some(value)),
                        Err(error) => {
                            self.abort_pending("chunk_decode_error");
                            Err(StreamRuntimeError::producer(error))
                        }
                    };
                }
                OutboundResponse::End { payload } => {
                    if !payload.is_empty() {
                        self.abort_pending("stream_end_payload");
                        return Err(StreamRuntimeError::producer(
                            RuntimeError::ProviderUnavailable {
                                target: self.target.clone(),
                                reason: "outbound serverStream response.end must not carry payload"
                                    .to_string(),
                            },
                        ));
                    }
                    self.lease.complete();
                    return Ok(None);
                }
                OutboundResponse::Error(error) => {
                    self.lease.complete();
                    return Err(StreamRuntimeError::producer(
                        RuntimeError::ProviderUnavailable {
                            target: self.target.clone(),
                            reason: error.message,
                        },
                    ));
                }
            }
        }
    }

    fn decode_chunk(&self, payload: Vec<u8>) -> Result<Value> {
        let mut heap = self.context.request_heap();
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::QueueWorkItemPayload)
            .with_target_service(self.target_service.clone());
        let value = decode_payload_plan(&payload, &self.item_plan, &boundary, &mut heap)?;
        let value = runtime_coerce_required_plan(
            &value,
            &self.item_plan,
            &format!("{} response stream item", self.target),
            &mut heap,
        )?;
        runtime_to_wire_required_plan(
            &value,
            Some(&self.item_plan),
            &format!("{} response stream item", self.target),
            &mut heap,
        )
    }

    fn abort_pending(&self, reason: &str) {
        self.lease.cancel(reason);
    }
}

fn encode_outbound_request_payload(
    dispatch: &OutboundServiceDispatch,
    args: &[RuntimeValue],
    heap: &mut RequestHeap,
) -> Result<Vec<u8>> {
    if args.len() != dispatch.params.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "outbound service call {} expected {} argument(s), got {}",
            dispatch.target,
            dispatch.params.len(),
            args.len()
        )));
    }

    let mut object_fields = RuntimeObjectFields::new();
    for (param, value) in dispatch.params.iter().zip(args.iter()) {
        object_fields.insert(param.name.clone(), value.clone());
    }
    let value = runtime_object_from_fields(object_fields, heap)?;
    let args_plan = dispatch
        .request_plan
        .clone()
        .map_or_else(|| synthetic_request_plan_from_params(&dispatch.params), Ok)?;
    let boundary = PayloadBoundary::cross_service(
        PayloadBoundaryKind::OutboundServiceCall,
        dispatch.service_ref(),
    );
    Ok(encode_payload_plan(&value, &args_plan, &boundary, heap)?)
}

fn outbound_request_start(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
    dispatch: &OutboundServiceDispatch,
) -> OutboundServiceRequestStart {
    let test_effect_doubles = outbound_test_effect_doubles(interpreter, context);
    OutboundServiceRequestStart {
        mode: dispatch.mode.clone(),
        target: dispatch.target.clone(),
        operation_abi_id: dispatch.operation_abi_id.clone(),
        selector: dispatch.selector.clone(),
        service_id: dispatch.service_id.clone(),
        version: dispatch.version.clone(),
        build_id: dispatch.build_id.clone(),
        service_protocol_identity: dispatch.service_protocol_identity.clone(),
        activation_identity: dispatch.activation_identity.clone(),
        timeout_ms: dispatch.timeout_ms,
        test_effect_doubles,
    }
}

fn outbound_test_effect_doubles(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
    if !context.test_effects_enabled() {
        return context.test_effect_doubles();
    }

    let mut targets = context
        .test_effect_doubles()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    targets.sort();
    targets
        .into_iter()
        .filter_map(|target| {
            interpreter.next_test_effect_double(&target).map(|double| {
                (
                    target,
                    vec![RequestEffectDoubleControl {
                        expect_request: double.expect_request,
                        response: double.response,
                    }],
                )
            })
        })
        .collect()
}

fn synthetic_request_plan_from_params(params: &[OutboundServiceParam]) -> Result<RuntimeTypePlan> {
    let mut fields = Vec::with_capacity(params.len());
    for param in params {
        let ty = param
            .request_plan
            .clone()
            .unwrap_or_else(RuntimeTypePlan::json_value_plan);
        let required = !matches!(ty.node(), RuntimeTypeNode::Nullable(_));
        fields.push(RuntimeRecordFieldPlan {
            name: param.name.clone(),
            ty,
            required,
            identity: None,
        });
    }
    Ok(RuntimeTypePlan::synthetic_request_record(fields))
}

#[derive(Clone)]
struct OutboundServiceDispatch {
    service_id: String,
    version: String,
    build_id: String,
    service_protocol_identity: String,
    operation_abi_id: String,
    selector: String,
    target: String,
    mode: String,
    timeout_ms: Option<u64>,
    activation_identity: Option<String>,
    params: Vec<OutboundServiceParam>,
    request_plan: Option<RuntimeTypePlan>,
    response_plan: RuntimeTypePlan,
    return_plan: RuntimeTypePlan,
}

impl OutboundServiceDispatch {
    fn service_ref(&self) -> PayloadServiceRef {
        PayloadServiceRef::new(self.service_id.clone())
            .with_version(self.version.clone())
            .with_build_id(self.build_id.clone())
    }

    fn from_call(
        interpreter: &Interpreter,
        caller_addr: &ExecutableAddr,
        symbol: &ServiceDependencySymbolRef,
        service_dependencies: &[ServiceDependencyConstraint],
    ) -> Result<Self> {
        let dependency = find_service_dependency(service_dependencies, &symbol.dependency_ref)?;
        let operation = find_service_dependency_operation(dependency, symbol)?;
        Self::from_known_operation(
            interpreter,
            caller_addr,
            &symbol.dependency_ref,
            dependency,
            operation,
        )
    }

    fn from_dependency_operation_abi(
        interpreter: &Interpreter,
        caller_addr: &ExecutableAddr,
        dependency_ref: &str,
        operation_abi_id: &str,
        service_dependencies: &[ServiceDependencyConstraint],
    ) -> Result<Self> {
        let dependency = find_service_dependency(service_dependencies, dependency_ref)?;
        let operation = find_service_dependency_operation_by_abi_id(dependency, operation_abi_id)?;
        Self::from_known_operation(
            interpreter,
            caller_addr,
            dependency_ref,
            dependency,
            operation,
        )
    }

    fn from_known_operation(
        interpreter: &Interpreter,
        caller_addr: &ExecutableAddr,
        dependency_ref: &str,
        dependency: &ServiceDependencyConstraint,
        operation: &skiff_artifact_model::PublicationOperationAbi,
    ) -> Result<Self> {
        let program = interpreter.program_projection()?.type_view();
        let (mode, response_type) =
            operation_mode_and_response_type(&operation.public_signature.return_type);
        let params =
            outbound_params_from_operation(program, caller_addr, dependency_ref, operation)?;
        let response_plan = plan_from_artifact_type_ref(
            program,
            caller_addr,
            response_type,
            &format!(
                "service dependency {} operation {} returnType",
                dependency_ref, operation.operation.operation_abi_id
            ),
        )?;
        let return_plan = if mode == "serverStream" {
            plan_from_artifact_type_ref(
                program,
                caller_addr,
                &skiff_artifact_model::TypeRefIr::Native {
                    name: "Stream".to_string(),
                    args: vec![response_type.clone()],
                },
                &format!(
                    "service dependency {} operation {} stream returnType",
                    dependency_ref, operation.operation.operation_abi_id
                ),
            )?
        } else {
            response_plan.clone()
        };

        Ok(Self {
            service_id: dependency.id.clone(),
            version: dependency.version.clone(),
            // build_id and service_protocol_identity are publish-time-frozen boundary
            // compatibility witnesses, not the addressing key. Addressing is by
            // service_id + version; the router resolves the current build for that
            // version at request time and verifies its protocol identity satisfies
            // this frozen expectation.
            build_id: dependency.build_id.clone(),
            service_protocol_identity: dependency.service_protocol_identity.clone(),
            operation_abi_id: operation.operation.operation_abi_id.clone(),
            selector: format!("operation:{}", operation.operation.operation_abi_id),
            target: operation.operation.public_path.clone(),
            mode: mode.to_string(),
            timeout_ms: None,
            activation_identity: None,
            params,
            request_plan: None,
            response_plan,
            return_plan,
        })
    }

    fn stream_return_plan(&self) -> Result<RuntimeTypePlan> {
        match self.return_plan.node() {
            RuntimeTypeNode::Stream(_) => Ok(self.return_plan.clone()),
            _ => Err(RuntimeError::InvalidArtifact(format!(
                "outbound service call {} mode serverStream must return Stream<T>",
                self.target
            ))),
        }
    }
}

#[derive(Clone)]
struct OutboundServiceParam {
    name: String,
    request_plan: Option<RuntimeTypePlan>,
}

fn find_service_dependency<'a>(
    service_dependencies: &'a [ServiceDependencyConstraint],
    alias: &str,
) -> Result<&'a ServiceDependencyConstraint> {
    service_dependencies
        .iter()
        .find(|dependency| dependency.alias == alias)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service dependency alias {alias} is not declared in RuntimeProgram"
            ))
        })
}

fn find_service_dependency_operation<'a>(
    dependency: &'a ServiceDependencyConstraint,
    symbol: &ServiceDependencySymbolRef,
) -> Result<&'a skiff_artifact_model::PublicationOperationAbi> {
    if symbol.operation.operation_abi_id.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service dependency alias {} operation {} is not linked with operationAbiId",
            dependency.alias, symbol.operation.public_path
        )));
    }
    let exported = dependency
        .publication_abi
        .operation_exports
        .iter()
        .any(|candidate| candidate == &symbol.operation);
    if !exported {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service dependency alias {} does not export operationAbiId {}",
            dependency.alias, symbol.operation.operation_abi_id
        )));
    }
    dependency
        .publication_abi
        .operation_abi
        .iter()
        .find(|candidate| {
            candidate.operation.operation_abi_id == symbol.operation.operation_abi_id
        })
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service dependency alias {} does not declare operationAbiId {}",
                dependency.alias, symbol.operation.operation_abi_id
            ))
        })
        .and_then(|operation| {
            if symbol.operation != operation.operation {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "service dependency alias {} operation {} conflicts with operationAbiId {} ({})",
                    dependency.alias,
                    symbol.operation.public_path,
                    symbol.operation.operation_abi_id,
                    operation.operation.public_path
                )));
            }
            Ok(operation)
        })
}

fn find_service_dependency_operation_by_abi_id<'a>(
    dependency: &'a ServiceDependencyConstraint,
    operation_abi_id: &str,
) -> Result<&'a skiff_artifact_model::PublicationOperationAbi> {
    if operation_abi_id.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service dependency alias {} remote operation is not linked with operationAbiId",
            dependency.alias
        )));
    }
    let operation = dependency
        .publication_abi
        .operation_abi
        .iter()
        .find(|candidate| candidate.operation.operation_abi_id == operation_abi_id)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service dependency alias {} does not declare operationAbiId {}",
                dependency.alias, operation_abi_id
            ))
        })?;
    let exported = dependency
        .publication_abi
        .operation_exports
        .iter()
        .any(|candidate| {
            !candidate.operation_abi_id.is_empty()
                && candidate.operation_abi_id == operation.operation.operation_abi_id
                && candidate == &operation.operation
        });
    if !exported {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service dependency alias {} does not export operationAbiId {}",
            dependency.alias, operation_abi_id
        )));
    }
    Ok(operation)
}

fn outbound_params_from_operation(
    program: ProgramTypeView<'_>,
    caller_addr: &ExecutableAddr,
    alias: &str,
    operation: &skiff_artifact_model::PublicationOperationAbi,
) -> Result<Vec<OutboundServiceParam>> {
    operation
        .public_signature
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let request_plan = plan_from_artifact_type_ref(
                program,
                caller_addr,
                &param.ty,
                &format!(
                    "service dependency {alias} operation {} params[{index}].ty",
                    operation.operation.public_path
                ),
            )?;
            Ok(OutboundServiceParam {
                name: param.name.clone(),
                request_plan: Some(request_plan),
            })
        })
        .collect()
}

fn operation_mode_and_response_type(
    return_type: &skiff_artifact_model::TypeRefIr,
) -> (&'static str, &skiff_artifact_model::TypeRefIr) {
    match return_type {
        skiff_artifact_model::TypeRefIr::Native { name, args }
            if name.rsplit('.').next() == Some("Stream") && args.len() == 1 =>
        {
            ("serverStream", &args[0])
        }
        _ => ("unary", return_type),
    }
}

fn plan_from_artifact_type_ref(
    program: ProgramTypeView<'_>,
    caller_addr: &ExecutableAddr,
    type_ref: &skiff_artifact_model::TypeRefIr,
    label: &str,
) -> Result<RuntimeTypePlan> {
    RuntimeTypePlan::from_artifact_type_ref_in_type_view(type_ref, program, caller_addr)
        .map_err(|error| RuntimeError::InvalidArtifact(format!("invalid {label}: {error}")))
}

struct OutboundServiceResponse {
    payload: Vec<u8>,
}

fn outbound_router_response_into_result(
    response: OutboundResponse,
    target: &str,
) -> Result<OutboundServiceResponse> {
    match response {
        OutboundResponse::End { payload } => Ok(OutboundServiceResponse { payload }),
        OutboundResponse::Error(error) => Err(RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: error.message,
        }),
        other => Err(RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: format!("unary outbound service call received {}", other.kind()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use skiff_runtime_capability_context::{
        CancellationToken, OutboundRequestCancelSendError, OutboundRequestCancelSender,
        OutboundRequestRegistry, OutboundResponse, OutboundResponseReceiver,
        OutboundStartedRequest,
    };
    use skiff_runtime_linked_program::ServiceDependencyConstraint;
    use skiff_runtime_model::request_heap::{RequestHeap, RequestHeapLimits};
    use tokio::sync::mpsc;

    use super::super::capabilities::{EvalCapabilityFuture, OutboundServiceApi};
    use super::*;

    #[tokio::test]
    async fn outbound_service_stream_source_drop_cancels_lease_and_registry() {
        let registry = OutboundRequestRegistry::default();
        let (response_sender, response_rx) = mpsc::unbounded_channel();
        let (cancel_sender, mut cancel_rx) = mpsc::unbounded_channel();
        let cancel_sender: OutboundRequestCancelSender = Arc::new(move |request_id, reason| {
            cancel_sender
                .send((request_id.to_string(), reason.to_string()))
                .map_err(|_| OutboundRequestCancelSendError::Closed)
        });
        let lease = registry
            .insert_with_lease(
                "request-stream".to_string(),
                response_sender,
                Some(cancel_sender),
                "stream_cancelled",
            )
            .expect("stream lease should insert");
        let source = OutboundServiceStreamSource {
            lease,
            context: OutboundServiceContext::new(DummyOutboundService),
            target: "service.stream".to_string(),
            target_service: PayloadServiceRef::new("service.test".to_string()),
            receiver: response_rx,
            item_plan: RuntimeTypePlan::json_value_plan(),
            next_seq: 0,
            started: false,
        };

        drop(source);

        let (request_id, reason) =
            tokio::time::timeout(std::time::Duration::from_secs(1), cancel_rx.recv())
                .await
                .expect("source drop should send cancel")
                .expect("cancel receiver should stay open");
        assert_eq!(request_id, "request-stream");
        assert_eq!(reason, "stream_cancelled");
        assert_eq!(registry.pending_count(), 0);
        assert_eq!(registry.active_lease_count(), 0);
    }

    struct DummyOutboundService;

    impl OutboundServiceApi for DummyOutboundService {
        fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
            &[]
        }

        fn test_effects_enabled(&self) -> bool {
            false
        }

        fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
            HashMap::new()
        }

        fn request_heap(&self) -> RequestHeap {
            RequestHeap::new(RequestHeapLimits::default())
        }

        fn effective_timeout_ms(&self, _operation_timeout_ms: Option<u64>) -> Option<u64> {
            None
        }

        fn outbound_deadline_error(&self) -> RuntimeError {
            RuntimeError::Cancelled
        }

        fn start_request(
            &self,
            _start: OutboundServiceRequestStart,
            _payload: Vec<u8>,
        ) -> Result<OutboundStartedRequest> {
            panic!("dummy outbound service is not used by this test")
        }

        fn receive_response<'a>(
            &'a self,
            _lease: &'a OutboundRequestLease,
            _target: &'a str,
            _receiver: &'a mut OutboundResponseReceiver,
            _timeout_ms: Option<u64>,
        ) -> EvalCapabilityFuture<'a, OutboundResponse> {
            Box::pin(async { panic!("dummy outbound service is not used by this test") })
        }

        fn cancel_signal(&self) -> CancellationToken {
            CancellationToken::new()
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn outbound_control_and_payload_for_test(
    interpreter: &Interpreter,
    context: &OutboundServiceContext,
    heap: &mut RequestHeap,
    caller_addr: &ExecutableAddr,
    call: &CallIr,
    symbol: &ServiceDependencySymbolRef,
    args: Vec<RuntimeValue>,
) -> Result<(RequestStartControl, Vec<u8>)> {
    let dispatch = OutboundServiceDispatch::from_call(
        interpreter,
        caller_addr,
        symbol,
        context.service_dependencies(),
    )?;
    let _ = call;
    let payload = encode_outbound_request_payload(&dispatch, &args, heap)?;
    Ok((
        context.request_start_control_for_test(
            outbound_request_start(interpreter, context, &dispatch),
            "request-test-outbound".to_string(),
        ),
        payload,
    ))
}
