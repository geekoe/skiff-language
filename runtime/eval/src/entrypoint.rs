use std::{collections::HashMap, sync::Arc};

use serde_json::Value;
use skiff_runtime_boundary::{
    binary::encode_payload_plan,
    http::{HttpBoundaryResponseParts, HttpBoundaryResponseStreamEvent},
    payload::{PayloadBoundary, PayloadBoundaryKind},
};
use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
    type_plan::RuntimeTypePlan,
};

use crate::{
    capabilities::{
        EvalRequestExecutionCapabilities, EvalRequestProgramExecutionInput, EvalRuntimeFactory,
        TestEffectDouble,
    },
    error::Result,
    invocation::{EvalInvocation, EvalWebSocketAdapterResult},
    program_invocation::ProgramInvocationInput,
    runtime_ops::runtime_from_wire_required_plan,
    stream_callback::{
        map_callback_error, map_eval_error, EvalStreamExecutionError, EvalStreamResult,
    },
    EvalProgramContext, EvalRuntimeProgram, Interpreter, InterpreterHttpOptions,
};

pub struct EvalEntrypoint {
    interpreter: Interpreter,
}

pub struct EvalRequestExecutor {
    entrypoint: EvalEntrypoint,
}

pub struct EvalRequestExecutorInput {
    pub program: Arc<EvalRuntimeProgram>,
    pub test_effects_enabled: bool,
    pub test_effect_doubles: HashMap<String, Vec<EvalRequestEffectDouble>>,
    pub runtime_factory: EvalRuntimeFactory,
}

pub struct EvalRequestEffectDouble {
    pub expect_request: Option<Value>,
    pub response: Value,
}

pub struct EvalRequestExecutionInput<'a> {
    pub request: RequestPayloadContext<'a>,
    pub operation: &'a str,
    pub capabilities: EvalRequestExecutionCapabilities<'a>,
    pub request_heap_limits: RequestHeapLimits,
    pub http_response_max_bytes: usize,
}

struct EvalEntrypointInput {
    program: Arc<EvalRuntimeProgram>,
    test_effects_enabled: bool,
    test_effect_doubles: HashMap<String, Vec<EvalTestEffectDouble>>,
    http_options: EvalEntrypointHttpOptions,
    runtime_factory: EvalRuntimeFactory,
}

struct EvalTestEffectDouble {
    expect_request: Option<Value>,
    response: Value,
}

#[derive(Clone, Copy, Debug)]
struct EvalEntrypointHttpOptions {
    allow_unsafe_targets: bool,
}

impl EvalEntrypointHttpOptions {
    fn public_network() -> Self {
        Self {
            allow_unsafe_targets: false,
        }
    }

    #[allow(dead_code)]
    fn allowing_unsafe_targets() -> Self {
        Self {
            allow_unsafe_targets: true,
        }
    }
}

impl From<EvalEntrypointHttpOptions> for InterpreterHttpOptions {
    fn from(options: EvalEntrypointHttpOptions) -> Self {
        if options.allow_unsafe_targets {
            InterpreterHttpOptions::allowing_unsafe_targets()
        } else {
            InterpreterHttpOptions::public_network()
        }
    }
}

struct EvalProgramInvocationInput<'a> {
    request: RequestPayloadContext<'a>,
    operation: &'a str,
    capabilities: EvalRequestExecutionCapabilities<'a>,
    request_heap_limits: RequestHeapLimits,
    http_response_max_bytes: usize,
}

impl<'a> From<EvalRequestExecutionInput<'a>> for EvalProgramInvocationInput<'a> {
    fn from(input: EvalRequestExecutionInput<'a>) -> Self {
        Self {
            request: input.request,
            operation: input.operation,
            capabilities: input.capabilities,
            request_heap_limits: input.request_heap_limits,
            http_response_max_bytes: input.http_response_max_bytes,
        }
    }
}

impl EvalRequestExecutor {
    pub fn new(input: EvalRequestExecutorInput) -> Self {
        Self {
            entrypoint: EvalEntrypoint::new(EvalEntrypointInput {
                program: input.program,
                test_effects_enabled: input.test_effects_enabled,
                test_effect_doubles: test_effect_doubles_for_entrypoint(input.test_effect_doubles),
                http_options: EvalEntrypointHttpOptions::public_network(),
                runtime_factory: input.runtime_factory,
            }),
        }
    }

    pub async fn execute_runtime_value<'a>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<(RuntimeValue, RuntimeTypePlan, RequestHeap)> {
        self.entrypoint
            .execute_runtime_value(input.into(), eval_invocation)
            .await
    }

    pub async fn execute_runtime_response_stream_payloads<'a, F, E>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
        on_payload: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(Vec<u8>) -> std::result::Result<(), E>,
    {
        self.entrypoint
            .execute_runtime_response_stream_payloads(input.into(), eval_invocation, on_payload)
            .await
    }

    pub async fn execute_binary_http<'a>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        self.entrypoint
            .execute_binary_http(input.into(), eval_invocation)
            .await
    }

    pub async fn execute_binary_http_response_stream<'a, F, E>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
        on_event: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        self.entrypoint
            .execute_binary_http_response_stream(input.into(), eval_invocation, on_event)
            .await
    }

    pub async fn execute_http_adapter<'a>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        self.entrypoint
            .execute_http_adapter(input.into(), eval_invocation)
            .await
    }

    pub async fn execute_http_raw_adapter_response_stream<'a, F, E>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
        on_event: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        self.entrypoint
            .execute_http_raw_adapter_response_stream(input.into(), eval_invocation, on_event)
            .await
    }

    pub async fn execute_websocket_adapter<'a>(
        &'a self,
        input: EvalRequestExecutionInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<EvalWebSocketAdapterResult> {
        self.entrypoint
            .execute_websocket_adapter(input.into(), eval_invocation)
            .await
    }
}

impl EvalEntrypoint {
    fn new(input: EvalEntrypointInput) -> Self {
        let interpreter = if input.test_effects_enabled || !input.test_effect_doubles.is_empty() {
            Interpreter::with_program_test_effect_double_sequences_http_options(
                input.program,
                test_effect_doubles_for_interpreter(input.test_effect_doubles),
                input.http_options.into(),
                input.runtime_factory,
            )
        } else {
            Interpreter::with_program(input.program, input.runtime_factory)
        };
        Self { interpreter }
    }

    async fn execute_runtime_value<'a>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<(RuntimeValue, RuntimeTypePlan, RequestHeap)> {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_eval_invocation_runtime_value(&invocation_context, eval_invocation)
            .await
    }

    async fn execute_runtime_response_stream_payloads<'a, F, E>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
        mut on_payload: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(Vec<u8>) -> std::result::Result<(), E>,
    {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_eval_invocation_runtime_response_stream(
                &invocation_context,
                eval_invocation,
                |item, item_plan| {
                    let mut item_heap = invocation_context.request_heap();
                    let value = map_eval_error(runtime_from_wire_required_plan(
                        &item,
                        Some(item_plan),
                        "serverStream response item",
                        &mut item_heap,
                    ))?;
                    let boundary =
                        PayloadBoundary::external_untrusted(PayloadBoundaryKind::StreamItem);
                    let payload = map_eval_error(encode_payload_plan(
                        &value, item_plan, &boundary, &item_heap,
                    ))?;
                    map_callback_error(on_payload(payload))
                },
            )
            .await
            .map_err(EvalStreamExecutionError::flatten)
    }

    async fn execute_binary_http<'a>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_eval_invocation_binary_http(&invocation_context, eval_invocation)
            .await
    }

    async fn execute_binary_http_response_stream<'a, F, E>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
        on_event: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_eval_invocation_binary_http_response_stream(
                &invocation_context,
                eval_invocation,
                on_event,
            )
            .await
    }

    async fn execute_http_adapter<'a>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_program_http_adapter(&invocation_context, eval_invocation)
            .await
    }

    async fn execute_http_raw_adapter_response_stream<'a, F, E>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
        on_event: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_program_http_raw_adapter_response_stream(
                &invocation_context,
                eval_invocation,
                on_event,
            )
            .await
    }

    async fn execute_websocket_adapter<'a>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<EvalWebSocketAdapterResult> {
        let invocation_context = self.program_invocation_context(input);
        self.interpreter
            .execute_program_websocket_adapter(&invocation_context, eval_invocation)
            .await
    }

    fn program_invocation_context<'a>(
        &'a self,
        input: EvalProgramInvocationInput<'a>,
    ) -> EvalProgramContext<'a> {
        let execution_request_heap_limits = input.request_heap_limits.clone();
        let execution =
            input
                .capabilities
                .into_program_execution_input(EvalRequestProgramExecutionInput {
                    stream_runtime: self.interpreter.stream_runtime.clone(),
                    http_options: self.interpreter.http_options.clone(),
                    test_effect_doubles: self.interpreter.test_effect_double_context(),
                    request_heap_limits: execution_request_heap_limits,
                });
        EvalProgramContext::new(ProgramInvocationInput {
            request: input.request,
            operation: input.operation,
            execution,
            http_response_max_bytes: input.http_response_max_bytes,
            request_heap_limits: input.request_heap_limits,
        })
    }
}

fn test_effect_doubles_for_entrypoint(
    doubles: HashMap<String, Vec<EvalRequestEffectDouble>>,
) -> HashMap<String, Vec<EvalTestEffectDouble>> {
    doubles
        .into_iter()
        .map(|(target, sequence)| {
            (
                target,
                sequence
                    .into_iter()
                    .map(|double| EvalTestEffectDouble {
                        expect_request: double.expect_request,
                        response: double.response,
                    })
                    .collect(),
            )
        })
        .collect()
}

fn test_effect_doubles_for_interpreter(
    doubles: HashMap<String, Vec<EvalTestEffectDouble>>,
) -> HashMap<String, Vec<TestEffectDouble>> {
    doubles
        .into_iter()
        .map(|(target, sequence)| {
            (
                target,
                sequence
                    .into_iter()
                    .map(|double| TestEffectDouble {
                        expect_request: double.expect_request,
                        response: double.response,
                    })
                    .collect(),
            )
        })
        .collect()
}
