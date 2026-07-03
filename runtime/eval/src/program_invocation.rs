use serde_json::Value;
use skiff_runtime_boundary::http::{
    self, HttpBoundaryResponseParts, HttpBoundaryResponseStreamEvent,
};
use skiff_runtime_boundary::{
    binary::decode_payload_plan,
    payload::{PayloadBoundary, PayloadBoundaryKind},
};
use skiff_runtime_capability_context::{
    RequestPayloadContext, RequestPayloadEncoding, StreamRuntimeError,
};
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_linked_program::ConstAddr;
use skiff_runtime_linked_program::{ExecutableAddr, LinkedExecutable};
use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapNode, RuntimeValue},
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
};

#[cfg(any(test, feature = "test-support"))]
use super::runtime_ops::{runtime_response_value_required_plan, runtime_to_wire};
use super::{
    binary_http_boundary::{
        binary_http_request_parameter_value, binary_http_request_parameter_value_with_plan,
        binary_http_response_from_runtime_value, linked_http_response_stream_item_type,
    },
    capabilities::{ExecutionControl, StreamCancelSignal, StreamPoll, TypedStreamSink},
    env::{Env, Flow},
    flow_completion::FlowCompletionPolicy,
    invocation::{
        BinaryHttpRequestPlan, EvalBoundaryProjection, EvalInvocation, EvalProgramProjection,
    },
    program_execution::{ExecutableInvocation, ProgramExecutionContext, ProgramExecutionInput},
    program_ir::executable_has_explicit_self_binding,
    program_stream::{executable_body_contains_emit, linked_stream_item_type},
    recoverable_behavior::EvalRecoverableBehaviorHooks,
    recoverable_spawn_payload::{
        decode_spawn_args_payload, executable_request_recoverable_expected_plan,
    },
    runtime_ops::{
        runtime_coerce_required_plan, runtime_empty_object, runtime_from_wire_required_plan,
        runtime_to_wire_required_plan,
    },
    stream_callback::{
        map_callback_error, map_eval_error, EvalStreamExecutionError, EvalStreamResult,
    },
    Interpreter,
};
use crate::error::{Result, RuntimeError};

pub struct ProgramInvocationInput<'a> {
    pub request: RequestPayloadContext<'a>,
    pub operation: &'a str,
    pub execution: ProgramExecutionInput<'a>,
    pub http_response_max_bytes: usize,
    pub request_heap_limits: RequestHeapLimits,
}

#[derive(Clone)]
pub struct ProgramInvocationContext<'a> {
    request: RequestPayloadContext<'a>,
    operation: &'a str,
    execution_context: ProgramExecutionContext<'a>,
    http_response_max_bytes: usize,
    request_heap_limits: RequestHeapLimits,
}

impl<'a> ProgramInvocationContext<'a> {
    pub fn new(input: ProgramInvocationInput<'a>) -> Self {
        Self {
            request: input.request,
            operation: input.operation,
            execution_context: ProgramExecutionContext::new(input.execution),
            http_response_max_bytes: input.http_response_max_bytes,
            request_heap_limits: input.request_heap_limits,
        }
    }

    pub fn request(&self) -> &RequestPayloadContext<'a> {
        &self.request
    }

    pub fn target(&self) -> &str {
        self.request.target()
    }

    pub fn operation(&self) -> &str {
        self.operation
    }

    pub fn execution(&self) -> ExecutionControl<'a> {
        self.execution_context.execution()
    }

    pub fn execution_context(&self) -> ProgramExecutionContext<'a> {
        self.execution_context.clone()
    }

    pub fn http_response_max_bytes(&self) -> usize {
        self.http_response_max_bytes
    }

    pub fn request_heap(&self) -> RequestHeap {
        RequestHeap::new(self.request_heap_limits.clone())
    }
}

struct PreparedProgramInvocation<'a> {
    executable_invocation: ExecutableInvocation<'a>,
    boundary_projection: Option<EvalBoundaryProjection<'a>>,
    heap: RequestHeap,
    env: Env,
}

impl<'a> PreparedProgramInvocation<'a> {
    fn plan_context(&self) -> PlanContext<'a> {
        PlanContext::from_type_view(
            self.executable_invocation.program_projection().type_view(),
            self.executable_invocation.addr,
        )
    }

    fn projected_binary_http_request_plan(&self) -> Option<BinaryHttpRequestPlan> {
        match self.boundary_projection.as_ref()? {
            EvalBoundaryProjection::BinaryHttpUnary { request }
            | EvalBoundaryProjection::BinaryHttpServerStream { request } => Some(request.clone()),
            _ => None,
        }
    }

    fn projected_runtime_request_payload_plan(&self) -> Option<RuntimeTypePlan> {
        match self.boundary_projection.as_ref()? {
            EvalBoundaryProjection::RuntimeUnary {
                request_payload_plan,
            }
            | EvalBoundaryProjection::RuntimeServerStream {
                request_payload_plan,
            } => Some(request_payload_plan.clone()),
            _ => None,
        }
    }

    fn declare_binary_http_request_parameters(
        &mut self,
        _interpreter: &Interpreter,
        context: &ProgramInvocationContext<'_>,
        addr: &ExecutableAddr,
    ) -> Result<()> {
        if let Some(request_plan) = self.projected_binary_http_request_plan() {
            let binary_http = context.request().require_binary_http()?;
            let value = binary_http_request_parameter_value_with_plan(
                request_plan.parameter_name.as_str(),
                &request_plan.parameter_plan,
                binary_http,
                &mut self.heap,
            )?;
            self.env.declare_program_parameter(
                self.executable_invocation.executable,
                request_plan.parameter_name.as_str(),
                value,
            )?;
            return Ok(());
        }

        let request_params = self
            .executable_invocation
            .executable
            .params
            .iter()
            .skip(usize::from(self.executable_invocation.explicit_self_param));
        for parameter in request_params {
            let binary_http = context.request().require_binary_http()?;
            let value = binary_http_request_parameter_value(
                context.target(),
                self.executable_invocation.executable.symbol.as_str(),
                parameter.name.as_str(),
                Some(&parameter.ty),
                self.executable_invocation.program_projection().type_view(),
                addr,
                binary_http,
                &mut self.heap,
            )?;
            self.env.declare_program_parameter(
                self.executable_invocation.executable,
                &parameter.name,
                value,
            )?;
        }
        Ok(())
    }

    fn declare_runtime_value_request_parameters(
        &mut self,
        _interpreter: &Interpreter,
        request: &RequestPayloadContext<'_>,
        context: &ProgramInvocationContext<'_>,
        addr: &ExecutableAddr,
    ) -> Result<()> {
        let args_plan = match self.projected_runtime_request_payload_plan() {
            Some(args_plan) => args_plan,
            None => executable_request_payload_plan(
                self.executable_invocation.program_projection().type_view(),
                addr,
                self.executable_invocation.executable,
            )?,
        };
        declare_runtime_value_request_parameters(
            request,
            context,
            self.executable_invocation.program_projection(),
            addr,
            self.executable_invocation.executable,
            self.executable_invocation.explicit_self_param,
            &args_plan,
            &mut self.heap,
            &mut self.env,
        )
    }

    fn declare_runtime_args(
        &mut self,
        request: &RequestPayloadContext<'_>,
        args: Vec<RuntimeValue>,
    ) -> Result<()> {
        let request_params = self
            .executable_invocation
            .executable
            .params
            .iter()
            .skip(usize::from(self.executable_invocation.explicit_self_param))
            .collect::<Vec<_>>();
        if request_params.len() != args.len() {
            return Err(RuntimeError::Protocol {
                target: request.target().to_string(),
                message: format!(
                    "runtime adapter argument count mismatch for {}: expected {}, got {}",
                    self.executable_invocation.executable.symbol,
                    request_params.len(),
                    args.len()
                ),
            });
        }
        for (parameter, value) in request_params.into_iter().zip(args.into_iter()) {
            self.env.declare_program_parameter(
                self.executable_invocation.executable,
                &parameter.name,
                value,
            )?;
        }
        Ok(())
    }
}

impl Interpreter {
    #[cfg(any(test, feature = "test-support"))]
    pub async fn execute_program_addr_with_receiver_const(
        &self,
        context: &ProgramInvocationContext<'_>,
        addr: &ExecutableAddr,
        receiver_const: Option<&ConstAddr>,
    ) -> Result<Value> {
        let mut invocation = self.prepare_program_invocation(context, addr)?;
        if context.request().has_binary_http() {
            invocation.declare_binary_http_request_parameters(self, context, addr)?;
        } else {
            invocation.declare_runtime_value_request_parameters(
                self,
                context.request(),
                context,
                addr,
            )?;
        }
        if let Some(receiver_const) = receiver_const {
            let caller_env = invocation.env.clone();
            let receiver_value = self
                .eval_program_const_addr(
                    context.execution_context(),
                    &mut invocation.heap,
                    &caller_env,
                    receiver_const,
                )
                .await?;
            invocation
                .executable_invocation
                .declare_self(&mut invocation.env, receiver_value)?;
        }

        match invocation
            .executable_invocation
            .exec(
                self,
                context.execution_context(),
                &mut invocation.heap,
                &mut invocation.env,
            )
            .await
        {
            Ok(Flow::Return(value)) => {
                let return_type_ref = invocation
                    .executable_invocation
                    .executable
                    .return_type
                    .as_ref();
                if let Some(return_type_ref) = return_type_ref {
                    let plan_context = invocation.plan_context();
                    let response_plan =
                        RuntimeTypePlan::from_linked(return_type_ref, &plan_context)?;
                    runtime_response_value_required_plan(
                        value,
                        Some(&response_plan),
                        &format!(
                            "response {}",
                            invocation.executable_invocation.executable.symbol
                        ),
                        &mut invocation.heap,
                    )
                } else {
                    runtime_to_wire(&value, &invocation.heap)
                }
            }
            Ok(Flow::Continue | Flow::Parked) => Ok(Value::Null),
            Ok(Flow::Break | Flow::LoopContinue) => {
                Err(FlowCompletionPolicy::entry_loop_control_error(
                    invocation.executable_invocation.executable,
                ))
            }
            Ok(Flow::ContinueConsumer) => Err(FlowCompletionPolicy::entry_consumer_error(
                invocation.executable_invocation.executable,
            )),
            Err(error) => Err(self.attach_program_source_context(
                error,
                addr,
                invocation.executable_invocation.file,
                None,
            )),
        }
    }

    pub async fn execute_eval_invocation_binary_http<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        let addr = eval_invocation.addr();
        let mut invocation = self.prepare_eval_invocation(context, eval_invocation)?;
        invocation.declare_binary_http_request_parameters(self, context, addr)?;

        match invocation
            .executable_invocation
            .exec(
                self,
                context.execution_context(),
                &mut invocation.heap,
                &mut invocation.env,
            )
            .await
        {
            Ok(Flow::Return(value)) => {
                let return_type = invocation
                    .executable_invocation
                    .executable
                    .return_type
                    .as_ref();
                binary_http_response_from_runtime_value(
                    &value,
                    return_type,
                    invocation
                        .executable_invocation
                        .program_projection()
                        .type_view(),
                    addr,
                    &mut invocation.heap,
                )
            }
            Ok(Flow::Continue | Flow::Parked) => Err(RuntimeError::Decode(
                "HTTP handler returned no HttpResponse".to_string(),
            )),
            Ok(Flow::Break | Flow::LoopContinue) => {
                Err(FlowCompletionPolicy::entry_loop_control_error(
                    invocation.executable_invocation.executable,
                ))
            }
            Ok(Flow::ContinueConsumer) => Err(FlowCompletionPolicy::entry_consumer_error(
                invocation.executable_invocation.executable,
            )),
            Err(error) => Err(self.attach_program_source_context(
                error,
                addr,
                invocation.executable_invocation.file,
                None,
            )),
        }
    }

    pub async fn execute_eval_invocation_binary_http_response_stream<'a, F, E>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
        mut on_event: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let addr = eval_invocation.addr();
        let mut invocation =
            map_eval_error(self.prepare_eval_invocation(context, eval_invocation))?;
        map_eval_error(invocation.declare_binary_http_request_parameters(self, context, addr))?;
        let return_type_ref = invocation
            .executable_invocation
            .executable
            .return_type
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "HTTP streaming response boundary is missing return type".to_string(),
                )
            })
            .map_err(EvalStreamExecutionError::Eval)?;
        let plan_context = invocation.plan_context();
        map_eval_error(linked_http_response_stream_item_type(
            Some(return_type_ref),
            invocation
                .executable_invocation
                .program_projection()
                .type_view(),
            addr,
        ))?
        .ok_or_else(|| RuntimeError::Protocol {
            target: context.target().to_string(),
            message:
                "binary HTTP serverStream handler must return Stream<std.http.HttpResponseStreamEvent>"
                    .to_string(),
        })
        .map_err(EvalStreamExecutionError::Eval)?;
        let item_type_ref = linked_stream_item_type(Some(return_type_ref)).ok_or_else(|| {
            RuntimeError::Protocol {
                target: context.target().to_string(),
                message:
                    "binary HTTP serverStream handler must return Stream<std.http.HttpResponseStreamEvent>"
                        .to_string(),
            }
        })
        .map_err(EvalStreamExecutionError::Eval)?;
        let expected_plan =
            map_eval_error(RuntimeTypePlan::from_linked(return_type_ref, &plan_context))?;
        let item_type_plan = map_eval_error(RuntimeTypePlan::from_linked_nested_ref(
            item_type_ref,
            &plan_context,
        ))?;

        if executable_body_contains_emit(invocation.executable_invocation.executable) {
            let (stream_value, sink) = self.stream_runtime.channel_stream();
            let cancel_signal = sink.cancel_signal();
            invocation.env.stream_sink = Some(sink.clone());
            invocation.env.current_stream_item_type = Some(item_type_plan.clone());
            invocation.env.response_stream_sink = Some(TypedStreamSink {
                sink: sink.clone(),
                item_type: item_type_plan.clone(),
            });
            let producer_future = async {
                match invocation
                    .executable_invocation
                    .exec(
                        self,
                        context.execution_context(),
                        &mut invocation.heap,
                        &mut invocation.env,
                    )
                    .await
                {
                    Ok(_) => sink.end().await,
                    Err(error) if error.is_cancelled() && sink.is_cancelled() => {}
                    Err(error) => sink.fail(StreamRuntimeError::producer(error)).await,
                }
            };
            let consumer_future = self.consume_binary_http_response_stream(
                context,
                &stream_value,
                &item_type_plan,
                std::slice::from_ref(&cancel_signal),
                &mut on_event,
            );
            tokio::pin!(producer_future);
            tokio::pin!(consumer_future);
            return tokio::select! {
                () = &mut producer_future => consumer_future.await,
                result = &mut consumer_future => {
                    self.stream_runtime.cancel(&stream_value);
                    result
                }
            };
        }

        let stream_value = match invocation
            .executable_invocation
            .exec(
                self,
                context.execution_context(),
                &mut invocation.heap,
                &mut invocation.env,
            )
            .await
        {
            Ok(Flow::Return(value)) => {
                let value = map_eval_error(runtime_coerce_required_plan(
                    &value,
                    &expected_plan,
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                ))?;
                map_eval_error(runtime_to_wire_required_plan(
                    &value,
                    Some(&expected_plan),
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                ))?
            }
            Ok(Flow::Continue | Flow::Parked) => {
                return Err(EvalStreamExecutionError::Eval(RuntimeError::Decode(
                    "HTTP streaming handler returned no Stream".to_string(),
                )));
            }
            Ok(Flow::Break | Flow::LoopContinue) => {
                return Err(EvalStreamExecutionError::Eval(
                    FlowCompletionPolicy::entry_loop_control_error(
                        invocation.executable_invocation.executable,
                    ),
                ));
            }
            Ok(Flow::ContinueConsumer) => {
                return Err(EvalStreamExecutionError::Eval(
                    FlowCompletionPolicy::entry_consumer_error(
                        invocation.executable_invocation.executable,
                    ),
                ));
            }
            Err(error) => {
                return Err(EvalStreamExecutionError::Eval(
                    self.attach_program_source_context(
                        error,
                        addr,
                        invocation.executable_invocation.file,
                        None,
                    ),
                ));
            }
        };

        self.consume_binary_http_response_stream(
            context,
            &stream_value,
            &item_type_plan,
            &[],
            &mut on_event,
        )
        .await
    }

    pub async fn execute_eval_invocation_runtime_args_http_response_stream_with_heap<'a, F, E>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
        args: Vec<RuntimeValue>,
        heap: RequestHeap,
        on_event: &mut F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let mut invocation = map_eval_error(self.prepare_eval_invocation_with_heap(
            context,
            eval_invocation,
            heap,
            false,
        ))?;
        map_eval_error(invocation.declare_runtime_args(context.request(), args))?;
        self.execute_prepared_runtime_args_http_response_stream(context, invocation, on_event)
            .await
    }

    async fn execute_prepared_runtime_args_http_response_stream<'a, F, E>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        mut invocation: PreparedProgramInvocation<'a>,
        on_event: &mut F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let addr = invocation.executable_invocation.addr;
        let return_type_ref = invocation
            .executable_invocation
            .executable
            .return_type
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "HTTP streaming response boundary is missing return type".to_string(),
                )
            })
            .map_err(EvalStreamExecutionError::Eval)?;
        let plan_context = invocation.plan_context();
        map_eval_error(linked_http_response_stream_item_type(
            Some(return_type_ref),
            invocation
                .executable_invocation
                .program_projection()
                .type_view(),
            addr,
        ))?
        .ok_or_else(|| RuntimeError::Protocol {
            target: context.target().to_string(),
            message:
                "binary HTTP serverStream handler must return Stream<std.http.HttpResponseStreamEvent>"
                    .to_string(),
        })
        .map_err(EvalStreamExecutionError::Eval)?;
        let item_type_ref = linked_stream_item_type(Some(return_type_ref)).ok_or_else(|| {
            RuntimeError::Protocol {
                target: context.target().to_string(),
                message:
                    "binary HTTP serverStream handler must return Stream<std.http.HttpResponseStreamEvent>"
                        .to_string(),
            }
        })
        .map_err(EvalStreamExecutionError::Eval)?;
        let expected_plan =
            map_eval_error(RuntimeTypePlan::from_linked(return_type_ref, &plan_context))?;
        let item_type_plan = map_eval_error(RuntimeTypePlan::from_linked_nested_ref(
            item_type_ref,
            &plan_context,
        ))?;

        if executable_body_contains_emit(invocation.executable_invocation.executable) {
            let (stream_value, sink) = self.stream_runtime.channel_stream();
            let cancel_signal = sink.cancel_signal();
            invocation.env.stream_sink = Some(sink.clone());
            invocation.env.current_stream_item_type = Some(item_type_plan.clone());
            invocation.env.response_stream_sink = Some(TypedStreamSink {
                sink: sink.clone(),
                item_type: item_type_plan.clone(),
            });
            let producer_future = async {
                match invocation
                    .executable_invocation
                    .exec(
                        self,
                        context.execution_context(),
                        &mut invocation.heap,
                        &mut invocation.env,
                    )
                    .await
                {
                    Ok(_) => sink.end().await,
                    Err(error) if error.is_cancelled() && sink.is_cancelled() => {}
                    Err(error) => sink.fail(StreamRuntimeError::producer(error)).await,
                }
            };
            let consumer_future = self.consume_binary_http_response_stream(
                context,
                &stream_value,
                &item_type_plan,
                std::slice::from_ref(&cancel_signal),
                on_event,
            );
            tokio::pin!(producer_future);
            tokio::pin!(consumer_future);
            return tokio::select! {
                () = &mut producer_future => consumer_future.await,
                result = &mut consumer_future => {
                    self.stream_runtime.cancel(&stream_value);
                    result
                }
            };
        }

        let stream_value = match invocation
            .executable_invocation
            .exec(
                self,
                context.execution_context(),
                &mut invocation.heap,
                &mut invocation.env,
            )
            .await
        {
            Ok(Flow::Return(value)) => {
                let value = map_eval_error(runtime_coerce_required_plan(
                    &value,
                    &expected_plan,
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                ))?;
                map_eval_error(runtime_to_wire_required_plan(
                    &value,
                    Some(&expected_plan),
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                ))?
            }
            Ok(Flow::Continue | Flow::Parked) => {
                return Err(EvalStreamExecutionError::Eval(RuntimeError::Decode(
                    "HTTP streaming handler returned no Stream".to_string(),
                )));
            }
            Ok(Flow::Break | Flow::LoopContinue) => {
                return Err(EvalStreamExecutionError::Eval(
                    FlowCompletionPolicy::entry_loop_control_error(
                        invocation.executable_invocation.executable,
                    ),
                ));
            }
            Ok(Flow::ContinueConsumer) => {
                return Err(EvalStreamExecutionError::Eval(
                    FlowCompletionPolicy::entry_consumer_error(
                        invocation.executable_invocation.executable,
                    ),
                ));
            }
            Err(error) => {
                return Err(EvalStreamExecutionError::Eval(
                    self.attach_program_source_context(
                        error,
                        addr,
                        invocation.executable_invocation.file,
                        None,
                    ),
                ));
            }
        };

        self.consume_binary_http_response_stream(
            context,
            &stream_value,
            &item_type_plan,
            &[],
            on_event,
        )
        .await
    }

    pub async fn execute_eval_invocation_runtime_args_with_heap<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
        args: Vec<RuntimeValue>,
        heap: RequestHeap,
    ) -> Result<(RuntimeValue, RuntimeTypePlan, RequestHeap)> {
        let mut invocation =
            self.prepare_eval_invocation_with_heap(context, eval_invocation, heap, false)?;
        invocation.declare_runtime_args(context.request(), args)?;
        self.execute_prepared_runtime_value(context, invocation)
            .await
    }

    pub async fn execute_eval_invocation_runtime_value<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<(RuntimeValue, RuntimeTypePlan, RequestHeap)> {
        let request = eval_invocation.request();
        let addr = eval_invocation.addr();
        let mut invocation = self.prepare_eval_invocation(context, eval_invocation)?;
        invocation.declare_runtime_value_request_parameters(self, &request, context, addr)?;
        self.execute_prepared_runtime_value(context, invocation)
            .await
    }

    async fn execute_prepared_runtime_value<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        mut invocation: PreparedProgramInvocation<'a>,
    ) -> Result<(RuntimeValue, RuntimeTypePlan, RequestHeap)> {
        match invocation
            .executable_invocation
            .exec(
                self,
                context.execution_context(),
                &mut invocation.heap,
                &mut invocation.env,
            )
            .await
        {
            Ok(Flow::Return(value)) => {
                let return_type_ref = invocation
                    .executable_invocation
                    .executable
                    .return_type
                    .as_ref();
                // Build the response plan directly from the return-type
                // `LinkedTypeRef` (or the JSON fallback when there is none),
                // bypassing the `program_type_descriptor -> Value` round-trip.
                // `from_linked` is proven equivalent to
                // `from_descriptor(program_type_descriptor(..))` by the oracle,
                // so this single plan now drives BOTH the coercion below and the
                // payload encode on the caller side. Coercion previously ran
                // against `program_type_descriptor(..).unwrap_or_else(json_descriptor)`,
                // which is exactly the plan `from_descriptor` would build from
                // that resolved descriptor — i.e. the same plan as `return_plan`.
                let plan_context = invocation.plan_context();
                let return_plan = match return_type_ref {
                    Some(return_type_ref) => {
                        RuntimeTypePlan::from_linked(return_type_ref, &plan_context)?
                    }
                    None => RuntimeTypePlan::json_value_plan(),
                };
                let value = runtime_coerce_required_plan(
                    &value,
                    &return_plan,
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                )?;
                Ok((value, return_plan, invocation.heap))
            }
            Ok(Flow::Continue | Flow::Parked) => Ok((
                RuntimeValue::Null,
                RuntimeTypePlan::json_value_plan(),
                invocation.heap,
            )),
            Ok(Flow::Break | Flow::LoopContinue) => {
                Err(FlowCompletionPolicy::entry_loop_control_error(
                    invocation.executable_invocation.executable,
                ))
            }
            Ok(Flow::ContinueConsumer) => Err(FlowCompletionPolicy::entry_consumer_error(
                invocation.executable_invocation.executable,
            )),
            Err(error) => Err(self.attach_program_source_context(
                error,
                invocation.executable_invocation.addr,
                invocation.executable_invocation.file,
                None,
            )),
        }
    }

    pub async fn execute_eval_invocation_runtime_response_stream<'a, F, E>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
        mut on_item: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(Value, &RuntimeTypePlan) -> std::result::Result<(), E>,
    {
        let addr = eval_invocation.addr();
        let request = eval_invocation.request();
        let mut invocation =
            map_eval_error(self.prepare_eval_invocation(context, eval_invocation))?;
        map_eval_error(
            invocation.declare_runtime_value_request_parameters(self, &request, context, addr),
        )?;
        let return_type_ref = invocation
            .executable_invocation
            .executable
            .return_type
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "runtime serverStream response boundary is missing return type".to_string(),
                )
            })
            .map_err(EvalStreamExecutionError::Eval)?;
        let item_type_ref = linked_stream_item_type(Some(return_type_ref))
            .ok_or_else(|| RuntimeError::Protocol {
                target: context.target().to_string(),
                message: "serverStream handler must return Stream<T>".to_string(),
            })
            .map_err(EvalStreamExecutionError::Eval)?;
        let plan_context = invocation.plan_context();
        let expected_plan =
            map_eval_error(RuntimeTypePlan::from_linked(return_type_ref, &plan_context))?;
        let item_type_plan = map_eval_error(RuntimeTypePlan::from_linked_nested_ref(
            item_type_ref,
            &plan_context,
        ))?;

        if executable_body_contains_emit(invocation.executable_invocation.executable) {
            let (stream_value, sink) = self.stream_runtime.channel_stream();
            let cancel_signal = sink.cancel_signal();
            invocation.env.stream_sink = Some(sink.clone());
            invocation.env.current_stream_item_type = Some(item_type_plan.clone());
            let producer_future = async {
                match invocation
                    .executable_invocation
                    .exec(
                        self,
                        context.execution_context(),
                        &mut invocation.heap,
                        &mut invocation.env,
                    )
                    .await
                {
                    Ok(_) => sink.end().await,
                    Err(error) if error.is_cancelled() && sink.is_cancelled() => {}
                    Err(error) => sink.fail(StreamRuntimeError::producer(error)).await,
                }
            };
            let consumer_future = self.consume_runtime_response_stream(
                context,
                &stream_value,
                &item_type_plan,
                std::slice::from_ref(&cancel_signal),
                &mut on_item,
            );
            tokio::pin!(producer_future);
            tokio::pin!(consumer_future);
            return tokio::select! {
                () = &mut producer_future => consumer_future.await,
                result = &mut consumer_future => {
                    self.stream_runtime.cancel(&stream_value);
                    result
                }
            };
        }

        let stream_value = match invocation
            .executable_invocation
            .exec(
                self,
                context.execution_context(),
                &mut invocation.heap,
                &mut invocation.env,
            )
            .await
        {
            Ok(Flow::Return(value)) => {
                let value = map_eval_error(runtime_coerce_required_plan(
                    &value,
                    &expected_plan,
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                ))?;
                map_eval_error(runtime_to_wire_required_plan(
                    &value,
                    Some(&expected_plan),
                    &format!(
                        "response {}",
                        invocation.executable_invocation.executable.symbol
                    ),
                    &mut invocation.heap,
                ))?
            }
            Ok(Flow::Continue | Flow::Parked) => {
                return Err(EvalStreamExecutionError::Eval(RuntimeError::Decode(
                    "serverStream handler returned no Stream".to_string(),
                )));
            }
            Ok(Flow::Break | Flow::LoopContinue) => {
                return Err(EvalStreamExecutionError::Eval(
                    FlowCompletionPolicy::entry_loop_control_error(
                        invocation.executable_invocation.executable,
                    ),
                ));
            }
            Ok(Flow::ContinueConsumer) => {
                return Err(EvalStreamExecutionError::Eval(
                    FlowCompletionPolicy::entry_consumer_error(
                        invocation.executable_invocation.executable,
                    ),
                ));
            }
            Err(error) => {
                return Err(EvalStreamExecutionError::Eval(
                    self.attach_program_source_context(
                        error,
                        addr,
                        invocation.executable_invocation.file,
                        None,
                    ),
                ));
            }
        };

        self.consume_runtime_response_stream(
            context,
            &stream_value,
            &item_type_plan,
            &[],
            &mut on_item,
        )
        .await
    }

    async fn consume_runtime_response_stream<F, E>(
        &self,
        context: &ProgramInvocationContext<'_>,
        stream_value: &Value,
        item_type: &RuntimeTypePlan,
        cancel_signals: &[StreamCancelSignal],
        on_item: &mut F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(Value, &RuntimeTypePlan) -> std::result::Result<(), E>,
    {
        let execution = context.execution();
        loop {
            map_eval_error(execution.add_instruction_units(1))?;
            let item = map_eval_error(
                self.stream_runtime
                    .next_with_cancel(stream_value, cancel_signals, &[execution.cancel_flag()])
                    .await,
            )?;
            let item = match item {
                StreamPoll::Item(item) => item,
                StreamPoll::End => return Ok(()),
            };
            let mut heap = context.request_heap();
            let coerced = map_eval_error(runtime_from_wire_required_plan(
                &item,
                Some(item_type),
                "serverStream response item",
                &mut heap,
            ))?;
            let wire = map_eval_error(runtime_to_wire_required_plan(
                &coerced,
                Some(item_type),
                "serverStream response item",
                &mut heap,
            ))?;
            let callback_result = on_item(wire, item_type);
            map_callback_error(callback_result)?;
        }
    }

    async fn consume_binary_http_response_stream<F, E>(
        &self,
        context: &ProgramInvocationContext<'_>,
        stream_value: &Value,
        item_type: &RuntimeTypePlan,
        cancel_signals: &[StreamCancelSignal],
        on_event: &mut F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let execution = context.execution();
        loop {
            map_eval_error(execution.add_instruction_units(1))?;
            let item = map_eval_error(
                self.stream_runtime
                    .next_with_cancel(stream_value, cancel_signals, &[execution.cancel_flag()])
                    .await,
            )?;
            let item = match item {
                StreamPoll::Item(item) => item,
                StreamPoll::End => return Ok(()),
            };
            let mut heap = context.request_heap();
            let coerced = map_eval_error(runtime_from_wire_required_plan(
                &item,
                Some(item_type),
                "HTTP response stream item",
                &mut heap,
            ))?;
            let wire = map_eval_error(runtime_to_wire_required_plan(
                &coerced,
                Some(item_type),
                "HTTP response stream item",
                &mut heap,
            ))?;
            let event = map_eval_error(http::http_response_stream_event_from_wire(&wire))?;
            let should_stop = matches!(event, HttpBoundaryResponseStreamEvent::End);
            let callback_result = on_event(event);
            map_callback_error(callback_result)?;
            if should_stop {
                self.stream_runtime.cancel(stream_value);
                return Ok(());
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn prepare_program_invocation<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        addr: &'a ExecutableAddr,
    ) -> Result<PreparedProgramInvocation<'a>> {
        self.prepare_program_invocation_with_heap(context, addr, context.request_heap(), true)
    }

    fn prepare_eval_invocation<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<PreparedProgramInvocation<'a>> {
        self.prepare_eval_invocation_with_heap(
            context,
            eval_invocation,
            context.request_heap(),
            true,
        )
    }

    fn prepare_eval_invocation_with_heap<'a>(
        &'a self,
        _context: &ProgramInvocationContext<'_>,
        eval_invocation: EvalInvocation<'a>,
        heap: RequestHeap,
        validate_request_args: bool,
    ) -> Result<PreparedProgramInvocation<'a>> {
        let request = eval_invocation.request();
        let boundary_projection = eval_invocation.boundary_projection().clone();
        self.prepare_executable_invocation(
            &request,
            ExecutableInvocation::from_eval_invocation(eval_invocation),
            heap,
            validate_request_args,
            Some(boundary_projection),
        )
    }

    fn prepare_program_invocation_with_heap<'a>(
        &'a self,
        context: &ProgramInvocationContext<'_>,
        addr: &'a ExecutableAddr,
        heap: RequestHeap,
        validate_request_args: bool,
    ) -> Result<PreparedProgramInvocation<'a>> {
        let invocation = ExecutableInvocation::resolve(self, addr)?;
        self.prepare_executable_invocation(
            context.request(),
            invocation,
            heap,
            validate_request_args,
            None,
        )
    }

    fn prepare_executable_invocation<'a>(
        &'a self,
        request: &RequestPayloadContext<'_>,
        invocation: ExecutableInvocation<'a>,
        mut heap: RequestHeap,
        validate_request_args: bool,
        boundary_projection: Option<EvalBoundaryProjection<'a>>,
    ) -> Result<PreparedProgramInvocation<'a>> {
        if validate_request_args {
            validate_program_request_args(request, invocation.executable)?;
        }

        let mut env = invocation.env()?;
        let self_value = runtime_empty_object(&mut heap)?;
        invocation.declare_self(&mut env, self_value)?;

        Ok(PreparedProgramInvocation {
            executable_invocation: invocation,
            boundary_projection,
            heap,
            env,
        })
    }
}

fn validate_program_request_args(
    request: &RequestPayloadContext<'_>,
    executable: &LinkedExecutable,
) -> Result<()> {
    let explicit_self_param = executable_has_explicit_self_binding(executable);
    if request.has_binary_http() {
        let expected = executable
            .params
            .iter()
            .skip(usize::from(explicit_self_param))
            .count();
        if expected == 1 {
            return Ok(());
        }
        return Err(RuntimeError::Protocol {
            target: request.target().to_string(),
            message: format!(
                "binary HTTP request.start requires exactly one HttpRequest parameter, got {expected}"
            ),
        });
    }
    Ok(())
}

/// Build the request-payload args record plan directly from the executable's
/// parameter `LinkedTypeRef`s, bypassing the synthetic JSON descriptor + the
/// `program_type_descriptor -> Value -> from_descriptor` round-trip.
///
/// This is equivalent to the previous path which constructed
/// `{"kind":"record","fields":{<param>: program_type_descriptor(param.ty)}}` and
/// fed it through `from_descriptor`:
///   * each field's `ty` is `RuntimeTypePlan::from_linked(&param.ty, ..)`, which
///     the oracle proves equal to `from_descriptor(program_type_descriptor(..))`;
///   * the synthetic descriptor's `fields` `serde_json::Map` preserves insertion
///     order (serde_json's `preserve_order`/IndexMap is enabled in this build),
///     so fields were iterated in parameter declaration order — we keep that
///     order and do NOT sort;
///   * `required` was `!is_nullable_descriptor(resolved_descriptor)`, which is
///     exactly `!matches!(field_plan.node(), Nullable)` because `from_descriptor`
///     checks `nullable_inner` first;
///   * the top-level synthetic record's label/named_type_name/boundary_record_kind
///     are reproduced by `RuntimeTypePlan::synthetic_request_record`.
pub fn executable_request_payload_plan<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
) -> Result<RuntimeTypePlan> {
    let program = program.into();
    let explicit_self_param = executable_has_explicit_self_binding(executable);
    let ctx = PlanContext::from_type_view(program, addr);
    let mut fields = Vec::new();
    for parameter in executable
        .params
        .iter()
        .skip(usize::from(explicit_self_param))
    {
        let ty = RuntimeTypePlan::from_linked(&parameter.ty, &ctx)?;
        let required = !matches!(ty.node(), RuntimeTypeNode::Nullable(_));
        fields.push(RuntimeRecordFieldPlan {
            name: parameter.name.clone(),
            ty,
            required,
            identity: None,
        });
    }
    // The old synthetic descriptor stored fields in a `serde_json::Map` whose
    // iteration order is insertion order (serde_json's `preserve_order` feature
    // is enabled in this build), i.e. parameter declaration order. Preserve it.
    Ok(RuntimeTypePlan::synthetic_request_record(fields))
}

fn declare_runtime_value_request_parameters(
    request: &RequestPayloadContext<'_>,
    context: &ProgramInvocationContext<'_>,
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
    explicit_self_param: bool,
    args_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    env: &mut Env,
) -> Result<()> {
    let request_params = executable
        .params
        .iter()
        .skip(usize::from(explicit_self_param))
        .collect::<Vec<_>>();
    if request_params.is_empty() && request.payload_bytes().is_empty() {
        return Ok(());
    }
    let actor_context = context.execution_context().actor_context();
    let spawn_decode = RecoverableSpawnDecodeContext {
        program,
        addr,
        executable,
        artifact_identity: actor_context.request_service_protocol_identity(),
        build_id: actor_context.request_build_id(),
    };
    let decoded = decode_request_args_payload(request, args_plan, heap, Some(spawn_decode))?;
    let object = match &decoded {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Object(object) => object.fields().clone(),
            _ => {
                return Err(RuntimeError::Decode(
                    "decoded request payload must be an args record".to_string(),
                ));
            }
        },
        _ => {
            return Err(RuntimeError::Decode(
                "decoded request payload must be an args record".to_string(),
            ));
        }
    };
    for parameter in request_params {
        let value = object
            .get(&parameter.name)
            .cloned()
            .ok_or_else(|| RuntimeError::Protocol {
                target: request.target().to_string(),
                message: format!("missing required request parameter {}", parameter.name),
            })?;
        env.declare_program_parameter(executable, &parameter.name, value)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RecoverableSpawnDecodeContext<'a> {
    program: EvalProgramProjection<'a>,
    addr: &'a ExecutableAddr,
    executable: &'a LinkedExecutable,
    artifact_identity: &'a str,
    build_id: &'a str,
}

fn decode_request_args_payload(
    request: &RequestPayloadContext<'_>,
    args_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    spawn_decode: Option<RecoverableSpawnDecodeContext<'_>>,
) -> Result<RuntimeValue> {
    match request.payload_encoding() {
        RequestPayloadEncoding::RuntimeBinary => {
            let boundary =
                PayloadBoundary::external_untrusted(PayloadBoundaryKind::InboundServiceCall);
            Ok(decode_payload_plan(
                request.payload_bytes(),
                args_plan,
                &boundary,
                heap,
            )?)
        }
        RequestPayloadEncoding::RecoverableSpawnPayload => {
            let spawn_decode = spawn_decode.ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "spawn request payload decode requires linked recoverable spawn context"
                        .to_string(),
                )
            })?;
            let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
            let expected = executable_request_recoverable_expected_plan(
                spawn_decode.program.type_view(),
                spawn_decode.addr,
                spawn_decode.executable,
            )?;
            let behavior_hooks = EvalRecoverableBehaviorHooks::new(
                spawn_decode.program,
                spawn_decode.artifact_identity,
                spawn_decode.build_id,
            )?;
            Ok(decode_spawn_args_payload(
                request.payload_bytes(),
                &expected,
                &boundary,
                heap,
                &behavior_hooks,
            )?)
        }
    }
}

#[cfg(test)]
mod recoverable_spawn_payload_tests {
    use std::{collections::HashMap, sync::Arc};

    use serde_json::json;
    use skiff_runtime_boundary::{
        binary::encode_recoverable_payload_plan,
        payload::{PayloadBoundary, PayloadBoundaryKind},
        type_descriptor::RuntimeTypePlanDescriptorExt,
    };
    use skiff_runtime_capability_context::{RequestPayloadContext, RequestPayloadEncoding};
    use skiff_runtime_linked_program::{
        ExecutableAddr, ExecutableKind, LinkOverlay, LinkedExecutable, LinkedExecutableBody,
        LinkedFileUnit, LinkedTypeRef, PackageUnit, ParamIr, RuntimeTypeContext, SlotIr,
        SlotLayoutIr,
    };
    use skiff_runtime_model::{
        request_heap::RequestHeap,
        runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue},
        type_plan::{RuntimeRecordFieldPlan, RuntimeTypePlan},
    };

    use super::{decode_request_args_payload, RecoverableSpawnDecodeContext};
    use crate::invocation::EvalProgramProjection;

    struct TestProgram {
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<PackageUnit>>,
        package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
        spawn_routes: HashMap<String, ExecutableAddr>,
        link_overlay: LinkOverlay,
        types: RuntimeTypeContext,
    }

    impl TestProgram {
        fn empty() -> Self {
            Self {
                service_files: Vec::new(),
                packages: Vec::new(),
                package_files: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext::default(),
            }
        }

        fn projection(&self) -> EvalProgramProjection<'_> {
            EvalProgramProjection::new(
                &self.service_files,
                &self.packages,
                &self.package_files,
                &self.spawn_routes,
                &self.link_overlay,
                &self.types,
            )
        }
    }

    fn string_plan() -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "string",
            "args": []
        }))
        .expect("string plan should build")
    }

    fn args_record_plan() -> RuntimeTypePlan {
        RuntimeTypePlan::synthetic_request_record(vec![RuntimeRecordFieldPlan {
            name: "name".to_string(),
            ty: string_plan(),
            required: true,
            identity: None,
        }])
    }

    fn recoverable_spawn_args_bytes(plan: &RuntimeTypePlan) -> Vec<u8> {
        let mut heap = RequestHeap::default();
        let value = RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "name".to_string(),
                RuntimeValue::String("Ada".to_string()),
            )])))
            .expect("args object should allocate"),
        );
        encode_recoverable_payload_plan(
            &value,
            plan,
            &PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload),
            &heap,
        )
        .expect("spawn args should encode")
    }

    fn string_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "target".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "name".to_string(),
                slot: 0,
                ty: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            }],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "name".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    #[test]
    fn worker_spawn_request_args_decode_uses_recoverable_payload_encoding() {
        let plan = args_record_plan();
        let bytes = recoverable_spawn_args_bytes(&plan);
        assert_eq!(&bytes[..4], b"SKRE");
        let request = RequestPayloadContext::new("function:target", &bytes, None)
            .with_payload_encoding(RequestPayloadEncoding::RecoverableSpawnPayload);
        let program = TestProgram::empty();
        let executable = string_executable();
        let addr = ExecutableAddr::service(0, 0);
        let spawn_decode = RecoverableSpawnDecodeContext {
            program: program.projection(),
            addr: &addr,
            executable: &executable,
            artifact_identity: "skiff-protocol-v1:sha256:test",
            build_id: "skiff-service-build-v1:sha256:test",
        };

        let mut heap = RequestHeap::default();
        let decoded = decode_request_args_payload(&request, &plan, &mut heap, Some(spawn_decode))
            .expect("worker spawn args should decode from recoverable envelope");

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(object) = heap.get(handle).expect("args object resolves") else {
            panic!("decoded args should be an object");
        };
        assert_eq!(
            object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
    }

    #[test]
    fn ordinary_request_args_decode_still_rejects_recoverable_envelope_magic() {
        let plan = args_record_plan();
        let bytes = recoverable_spawn_args_bytes(&plan);
        let request = RequestPayloadContext::new("function:target", &bytes, None);

        let error = decode_request_args_payload(&request, &plan, &mut RequestHeap::default(), None)
            .expect_err("ordinary request args should not accept recoverable envelope bytes");

        assert!(error.to_string().contains("missing SKPV magic"));
    }
}

#[cfg(all(test, any()))]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::program::{
        anonymous_type_decl, ExecutableKind, LinkedExecutableBody, LinkedFileUnit,
        LinkedTypeDescriptor, LinkedTypeRef, ParamIr, RuntimeProgram, RuntimeTypeContext,
        ServiceMeta, SlotLayoutIr, TypeAddr,
    };
    use crate::program::{FileAddr, UnitAddr};
    use skiff_runtime_model::type_plan::RuntimeTypePlan;

    fn empty_program() -> RuntimeProgram {
        RuntimeProgram {
            service: ServiceMeta {
                id: "svc".to_string(),
                display_name: Some("Service".to_string()),
                metadata: Default::default(),
            },
            version: "v1".to_string(),
            build_id: "build:program".to_string(),
            service_files: vec![Arc::new(LinkedFileUnit {
                schema_version: "skiff-file-ir-v3".to_string(),
                file_ir_identity: "file:svc".to_string(),
                source_ast_hash: "source:svc".to_string(),
                module_path: "svc.main".to_string(),
                ir_format_version: None,
                opcode_table_version: None,
                source_map: Default::default(),
                declarations: Default::default(),
                link_targets: Default::default(),
                types: Vec::new(),
                constants: Vec::new(),
                executables: Vec::new(),
                external_refs: Default::default(),
            })],
            packages: Vec::new(),
            package_files: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            routes: Default::default(),
            spawn_routes: Default::default(),
            operations: Default::default(),
            operation_receivers: Default::default(),
            db: Vec::new(),
            actors: Vec::new(),
            link_overlay: Default::default(),
            gateway: Default::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn service_type_addr(type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index,
        }
    }

    fn builtin(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    #[test]
    fn request_payload_plan_uses_linked_nominal_field_plans() {
        let mut program = empty_program();
        // Intern an Address-targeted record descriptor so one param resolves
        // through the Address arm (record with nested builtins).
        program.types.descriptors.insert(
            service_type_addr(0),
            anonymous_type_decl(
                "AddressPayload",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([
                        ("id".to_string(), builtin("string")),
                        ("count".to_string(), builtin("number")),
                    ]),
                },
            ),
        );

        // Three params, intentionally NOT in alphabetical order, including a
        // nullable param and an Address(record) param, to exercise field
        // ordering + `required` derivation.
        let executable = LinkedExecutable {
            kind: ExecutableKind::Operation,
            symbol: "svc.main.handler".to_string(),
            type_params: Vec::new(),
            params: vec![
                ParamIr {
                    name: "zeta".to_string(),
                    slot: 0,
                    ty: LinkedTypeRef::Nullable {
                        inner: Box::new(builtin("string")),
                    },
                },
                ParamIr {
                    name: "alpha".to_string(),
                    slot: 1,
                    ty: LinkedTypeRef::Address {
                        addr: service_type_addr(0),
                    },
                },
                ParamIr {
                    name: "mid".to_string(),
                    slot: 2,
                    ty: builtin("number"),
                },
            ],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };

        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();

        let new_plan = executable_request_payload_plan(&image, &addr, &executable)
            .expect("plan-native request payload builder should succeed");

        let RuntimeTypeNode::Record { fields, .. } = new_plan.node() else {
            panic!("request payload plan should be a record");
        };
        assert_eq!(
            fields
                .iter()
                .map(|field| (field.name.as_str(), field.required))
                .collect::<Vec<_>>(),
            vec![("zeta", false), ("alpha", true), ("mid", true)]
        );
        assert_eq!(
            fields[1].ty.boundary_record_kind(),
            Some("AddressPayload"),
            "linked request payload plan should preserve nominal field type"
        );
    }

    /// The runtime-value response path coerces against the linked return plan,
    /// so nominal return types remain available to downstream boundaries.
    #[test]
    fn response_return_plan_preserves_linked_nominal_type() {
        let mut program = empty_program();
        program.types.descriptors.insert(
            service_type_addr(0),
            anonymous_type_decl(
                "AddressReturn",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([
                        ("id".to_string(), builtin("string")),
                        ("count".to_string(), builtin("number")),
                    ]),
                },
            ),
        );

        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let return_type_ref = LinkedTypeRef::Address {
            addr: service_type_addr(0),
        };

        // New path: coerce plan built directly from the return-type ref.
        let new_plan =
            RuntimeTypePlan::from_linked(&return_type_ref, &PlanContext::new(&image, &addr))
                .expect("from_linked should succeed");

        assert_eq!(
            new_plan.boundary_record_kind(),
            Some("AddressReturn"),
            "linked response plan should preserve nominal return type"
        );
    }
}
