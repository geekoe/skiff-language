//! Eval-owned projection from program execution context to native capability contexts.

use super::capabilities::{
    HttpResponseStreamCapabilityContext, RuntimeNativeActorCapabilityContext,
    RuntimeNativeFileCapabilityContext, RuntimeNativeHttpClientCapabilityContext,
    RuntimeNativeHttpResponseStreamCapabilityContext, RuntimeNativeTelemetryCapabilityContext,
    RuntimeNativeTimeCapabilityContext, RuntimeNativeWebsocketCapabilityContext,
    StreamCapabilityContext,
};
use super::program_execution::ProgramExecutionContext;
use crate::invocation::EvalProgramProjection;
use skiff_runtime_capability_context::{
    project_native_capability_context, NativeCapabilityContexts, NativeCapabilityProjectionSource,
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
    program: EvalProgramProjection<'context>,
    stream_context: StreamCapabilityContext,
}

impl<'context, 'execution> RuntimeNativeCapabilityProjectionSource<'context, 'execution> {
    fn new(
        context: &'context ProgramExecutionContext<'execution>,
        program: EvalProgramProjection<'context>,
        stream_context: StreamCapabilityContext,
    ) -> Self {
        Self {
            context,
            program,
            stream_context,
        }
    }
}

#[derive(Clone, Copy)]
pub struct RuntimeNativeResourceCapabilityContext<'a> {
    resources: skiff_runtime_linked_program::RuntimeProgramResourceView<'a>,
}

impl<'a> RuntimeNativeResourceCapabilityContext<'a> {
    fn new(resources: skiff_runtime_linked_program::RuntimeProgramResourceView<'a>) -> Self {
        Self { resources }
    }
}

impl skiff_runtime_native::capability::NativeResourceCapability
    for RuntimeNativeResourceCapabilityContext<'_>
{
    fn resources(&self) -> skiff_runtime_linked_program::RuntimeProgramResourceView<'_> {
        self.resources
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
        RuntimeNativeFileCapabilityContext::new(
            self.context.file_context(),
            self.context.file_source_stream_context(),
            self.context.request_heap_limits(),
        )
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
        RuntimeNativeResourceCapabilityContext::new(self.program.resource_view())
    }
}

pub fn project_runtime_native_capability_context<'context, 'execution>(
    context: &'context ProgramExecutionContext<'execution>,
    program: EvalProgramProjection<'context>,
    stream_context: StreamCapabilityContext,
    required_context: NativeRequiredContext,
) -> RuntimeNativeCapabilityContexts<'context, 'execution> {
    let source = RuntimeNativeCapabilityProjectionSource::new(context, program, stream_context);
    project_native_capability_context(required_context, &source)
}
