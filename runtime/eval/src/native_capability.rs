//! Eval-owned projection from program execution context to native capability contexts.

use super::capabilities::{
    HttpResponseStreamCapabilityContext, RuntimeNativeActorCapabilityContext,
    RuntimeNativeFileCapabilityContext, RuntimeNativeHttpClientCapabilityContext,
    RuntimeNativeHttpResponseStreamCapabilityContext, RuntimeNativeTelemetryCapabilityContext,
    RuntimeNativeTimeCapabilityContext, RuntimeNativeWebsocketCapabilityContext,
    StreamCapabilityContext,
};
use super::program_execution::ProgramExecutionContext;
use crate::assembly_execution::RuntimeExecutionProjection;
use crate::invocation::EvalProgramProjection;
use skiff_runtime_capability_context::{
    project_native_capability_context, NativeCapabilityContexts, NativeCapabilityProjectionSource,
    SupervisedStreamConsumptionChild,
};
use skiff_runtime_native_contract::NativeRequiredContext;

type RuntimeNativeCapabilityContexts<'context, 'execution> = NativeCapabilityContexts<
    RuntimeNativeActorCapabilityContext<'execution>,
    RuntimeNativeFileCapabilityContext<'execution>,
    RuntimeNativeTimeCapabilityContext<'execution>,
    RuntimeNativeHttpClientCapabilityContext,
    RuntimeNativeHttpResponseStreamCapabilityContext<'execution>,
    RuntimeNativeWebsocketCapabilityContext<'execution>,
    RuntimeNativeTelemetryCapabilityContext,
    RuntimeNativeResourceCapabilityContext<'context>,
>;

struct RuntimeNativeCapabilityProjectionSource<'context, 'execution> {
    context: &'context ProgramExecutionContext<'execution>,
    program: RuntimeExecutionProjection<'context>,
    stream_context: StreamCapabilityContext,
    stream_supervision: Option<SupervisedStreamConsumptionChild>,
}

impl<'context, 'execution> RuntimeNativeCapabilityProjectionSource<'context, 'execution> {
    fn new(
        context: &'context ProgramExecutionContext<'execution>,
        program: RuntimeExecutionProjection<'context>,
        stream_context: StreamCapabilityContext,
    ) -> Self {
        Self {
            context,
            program,
            stream_context,
            stream_supervision: None,
        }
    }

    fn new_supervised(
        context: &'context ProgramExecutionContext<'execution>,
        program: RuntimeExecutionProjection<'context>,
        stream_context: StreamCapabilityContext,
        stream_supervision: SupervisedStreamConsumptionChild,
    ) -> Self {
        Self {
            context,
            program,
            stream_context,
            stream_supervision: Some(stream_supervision),
        }
    }
}

#[derive(Clone)]
pub struct RuntimeNativeResourceCapabilityContext<'a> {
    projection: RuntimeExecutionProjection<'a>,
}

impl<'a> RuntimeNativeResourceCapabilityContext<'a> {
    fn new(projection: RuntimeExecutionProjection<'a>) -> Self {
        Self { projection }
    }
}

impl skiff_runtime_native::capability::NativeResourceCapability
    for RuntimeNativeResourceCapabilityContext<'_>
{
    fn resources(&self) -> skiff_runtime_linked_program::RuntimeProgramResourceView<'_> {
        self.projection.resource_view()
    }
}

impl<'context, 'execution> NativeCapabilityProjectionSource
    for RuntimeNativeCapabilityProjectionSource<'context, 'execution>
{
    type Actor = RuntimeNativeActorCapabilityContext<'execution>;
    type File = RuntimeNativeFileCapabilityContext<'execution>;
    type Time = RuntimeNativeTimeCapabilityContext<'execution>;
    type HttpClient = RuntimeNativeHttpClientCapabilityContext;
    type HttpResponseStream = RuntimeNativeHttpResponseStreamCapabilityContext<'execution>;
    type Websocket = RuntimeNativeWebsocketCapabilityContext<'execution>;
    type Telemetry = RuntimeNativeTelemetryCapabilityContext;
    type Resource = RuntimeNativeResourceCapabilityContext<'context>;

    fn actor(&self) -> Self::Actor {
        RuntimeNativeActorCapabilityContext::new(self.context.actor_context())
    }

    fn file(&self) -> Self::File {
        match &self.stream_supervision {
            Some(supervision) => RuntimeNativeFileCapabilityContext::new_supervised(
                self.context.file_context(),
                self.context.file_source_stream_context(),
                self.context.request_heap_limits(),
                supervision.clone(),
            ),
            None => RuntimeNativeFileCapabilityContext::new(
                self.context.file_context(),
                self.context.file_source_stream_context(),
                self.context.request_heap_limits(),
            ),
        }
    }

    fn time(&self) -> Self::Time {
        RuntimeNativeTimeCapabilityContext::new(self.context.time_context())
    }

    fn http_client(&self) -> Self::HttpClient {
        RuntimeNativeHttpClientCapabilityContext::new(
            self.context.http_client_context(),
            self.context.test_effect_double_context(),
        )
    }

    fn http_response_stream(&self) -> Self::HttpResponseStream {
        RuntimeNativeHttpResponseStreamCapabilityContext::new(
            HttpResponseStreamCapabilityContext::new(
                self.context.execution(),
                self.stream_context.clone(),
            ),
        )
    }

    fn websocket(&self) -> Self::Websocket {
        RuntimeNativeWebsocketCapabilityContext::new(self.context.websocket_context())
    }

    fn telemetry(&self) -> Self::Telemetry {
        RuntimeNativeTelemetryCapabilityContext::new(self.context.telemetry_context())
    }

    fn resource(&self) -> Self::Resource {
        RuntimeNativeResourceCapabilityContext::new(self.program.clone())
    }
}

pub fn project_runtime_native_capability_context<'context, 'execution>(
    context: &'context ProgramExecutionContext<'execution>,
    program: EvalProgramProjection<'context>,
    stream_context: StreamCapabilityContext,
    required_context: NativeRequiredContext,
) -> RuntimeNativeCapabilityContexts<'context, 'execution> {
    project_runtime_execution_native_capability_context(
        context,
        RuntimeExecutionProjection::Legacy(program),
        stream_context,
        required_context,
    )
}

pub(crate) fn project_runtime_execution_native_capability_context<'context, 'execution>(
    context: &'context ProgramExecutionContext<'execution>,
    program: RuntimeExecutionProjection<'context>,
    stream_context: StreamCapabilityContext,
    required_context: NativeRequiredContext,
) -> RuntimeNativeCapabilityContexts<'context, 'execution> {
    let source = RuntimeNativeCapabilityProjectionSource::new(context, program, stream_context);
    project_native_capability_context(required_context, &source)
}

pub(crate) fn project_runtime_execution_native_capability_context_supervised<
    'context,
    'execution,
>(
    context: &'context ProgramExecutionContext<'execution>,
    program: RuntimeExecutionProjection<'context>,
    stream_context: StreamCapabilityContext,
    required_context: NativeRequiredContext,
    stream_supervision: SupervisedStreamConsumptionChild,
) -> RuntimeNativeCapabilityContexts<'context, 'execution> {
    let source = RuntimeNativeCapabilityProjectionSource::new_supervised(
        context,
        program,
        stream_context,
        stream_supervision,
    );
    project_native_capability_context(required_context, &source)
}
