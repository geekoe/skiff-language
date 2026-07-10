use skiff_runtime_native_contract::NativeRequiredContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeFileCapabilityContext<FileContext, FileSourceStreamContext, RequestHeapLimits> {
    file_context: FileContext,
    file_source_stream_context: FileSourceStreamContext,
    request_heap_limits: RequestHeapLimits,
}

impl<FileContext, FileSourceStreamContext, RequestHeapLimits>
    NativeFileCapabilityContext<FileContext, FileSourceStreamContext, RequestHeapLimits>
{
    pub fn new(
        file_context: FileContext,
        file_source_stream_context: FileSourceStreamContext,
        request_heap_limits: RequestHeapLimits,
    ) -> Self {
        Self {
            file_context,
            file_source_stream_context,
            request_heap_limits,
        }
    }

    pub fn into_parts(self) -> (FileContext, FileSourceStreamContext, RequestHeapLimits) {
        (
            self.file_context,
            self.file_source_stream_context,
            self.request_heap_limits,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeHttpClientCapabilityContext<EffectContext> {
    effect_context: EffectContext,
}

impl<EffectContext> NativeHttpClientCapabilityContext<EffectContext> {
    pub fn new(effect_context: EffectContext) -> Self {
        Self { effect_context }
    }

    pub fn into_effect_context(self) -> EffectContext {
        self.effect_context
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeHttpResponseStreamCapabilityContext<ExecutionContext> {
    execution_context: ExecutionContext,
}

impl<ExecutionContext> NativeHttpResponseStreamCapabilityContext<ExecutionContext> {
    pub fn new(execution_context: ExecutionContext) -> Self {
        Self { execution_context }
    }

    pub fn into_execution_context(self) -> ExecutionContext {
        self.execution_context
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeTelemetryCapabilityContext<EffectContext> {
    effect_context: EffectContext,
}

impl<EffectContext> NativeTelemetryCapabilityContext<EffectContext> {
    pub fn new(effect_context: EffectContext) -> Self {
        Self { effect_context }
    }

    pub fn into_effect_context(self) -> EffectContext {
        self.effect_context
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeCapabilityContexts<
    ActorContext,
    FileContext,
    TimeContext,
    HttpClientContext,
    HttpResponseStreamContext,
    WebsocketContext,
    TelemetryContext,
    ResourceContext,
> {
    None,
    Actor(ActorContext),
    File(FileContext),
    Time(TimeContext),
    HttpClient(HttpClientContext),
    HttpResponseStream(HttpResponseStreamContext),
    Websocket(WebsocketContext),
    Telemetry(TelemetryContext),
    Resource(ResourceContext),
}

impl<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >
    NativeCapabilityContexts<
        ActorContext,
        FileContext,
        TimeContext,
        HttpClientContext,
        HttpResponseStreamContext,
        WebsocketContext,
        TelemetryContext,
        ResourceContext,
    >
{
    pub fn required_context(&self) -> NativeRequiredContext {
        match self {
            Self::None => NativeRequiredContext::None,
            Self::Actor(_) => NativeRequiredContext::Actor,
            Self::File(_) => NativeRequiredContext::File,
            Self::Time(_) => NativeRequiredContext::Time,
            Self::HttpClient(_) => NativeRequiredContext::HttpClient,
            Self::HttpResponseStream(_) => NativeRequiredContext::HttpResponseStream,
            Self::Websocket(_) => NativeRequiredContext::Websocket,
            Self::Telemetry(_) => NativeRequiredContext::Telemetry,
            Self::Resource(_) => NativeRequiredContext::Resource,
        }
    }
}

pub trait NativeCapabilityProjectionSource {
    type Actor;
    type File;
    type Time;
    type HttpClient;
    type HttpResponseStream;
    type Websocket;
    type Telemetry;
    type Resource;

    fn actor(&self) -> Self::Actor;
    fn file(&self) -> Self::File;
    fn time(&self) -> Self::Time;
    fn http_client(&self) -> Self::HttpClient;
    fn http_response_stream(&self) -> Self::HttpResponseStream;
    fn websocket(&self) -> Self::Websocket;
    fn telemetry(&self) -> Self::Telemetry;
    fn resource(&self) -> Self::Resource;
}

pub fn project_native_capability_context<Source>(
    required_context: NativeRequiredContext,
    source: &Source,
) -> NativeCapabilityContexts<
    Source::Actor,
    Source::File,
    Source::Time,
    Source::HttpClient,
    Source::HttpResponseStream,
    Source::Websocket,
    Source::Telemetry,
    Source::Resource,
>
where
    Source: NativeCapabilityProjectionSource,
{
    match required_context {
        NativeRequiredContext::None => NativeCapabilityContexts::None,
        NativeRequiredContext::Actor => NativeCapabilityContexts::Actor(source.actor()),
        NativeRequiredContext::File => NativeCapabilityContexts::File(source.file()),
        NativeRequiredContext::Time => NativeCapabilityContexts::Time(source.time()),
        NativeRequiredContext::HttpClient => {
            NativeCapabilityContexts::HttpClient(source.http_client())
        }
        NativeRequiredContext::HttpResponseStream => {
            NativeCapabilityContexts::HttpResponseStream(source.http_response_stream())
        }
        NativeRequiredContext::Websocket => NativeCapabilityContexts::Websocket(source.websocket()),
        NativeRequiredContext::Telemetry => NativeCapabilityContexts::Telemetry(source.telemetry()),
        NativeRequiredContext::Resource => NativeCapabilityContexts::Resource(source.resource()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[derive(Default)]
    struct TestProjectionSource {
        actor: Cell<usize>,
        file: Cell<usize>,
        time: Cell<usize>,
        http_client: Cell<usize>,
        http_response_stream: Cell<usize>,
        websocket: Cell<usize>,
        telemetry: Cell<usize>,
        resource: Cell<usize>,
    }

    impl TestProjectionSource {
        fn increment(counter: &Cell<usize>, value: &'static str) -> &'static str {
            counter.set(counter.get() + 1);
            value
        }

        fn call_counts(&self) -> [usize; 8] {
            [
                self.actor.get(),
                self.file.get(),
                self.time.get(),
                self.http_client.get(),
                self.http_response_stream.get(),
                self.websocket.get(),
                self.telemetry.get(),
                self.resource.get(),
            ]
        }
    }

    impl NativeCapabilityProjectionSource for TestProjectionSource {
        type Actor = &'static str;
        type File = NativeFileCapabilityContext<&'static str, &'static str, &'static str>;
        type Time = &'static str;
        type HttpClient = NativeHttpClientCapabilityContext<&'static str>;
        type HttpResponseStream = NativeHttpResponseStreamCapabilityContext<&'static str>;
        type Websocket = &'static str;
        type Telemetry = NativeTelemetryCapabilityContext<&'static str>;
        type Resource = &'static str;

        fn actor(&self) -> Self::Actor {
            Self::increment(&self.actor, "actor")
        }

        fn file(&self) -> Self::File {
            NativeFileCapabilityContext::new(
                Self::increment(&self.file, "file"),
                "file_source_stream",
                "heap_limits",
            )
        }

        fn time(&self) -> Self::Time {
            Self::increment(&self.time, "time")
        }

        fn http_client(&self) -> Self::HttpClient {
            NativeHttpClientCapabilityContext::new(Self::increment(
                &self.http_client,
                "http_client",
            ))
        }

        fn http_response_stream(&self) -> Self::HttpResponseStream {
            NativeHttpResponseStreamCapabilityContext::new(Self::increment(
                &self.http_response_stream,
                "http_response_stream",
            ))
        }

        fn websocket(&self) -> Self::Websocket {
            Self::increment(&self.websocket, "websocket")
        }

        fn telemetry(&self) -> Self::Telemetry {
            NativeTelemetryCapabilityContext::new(Self::increment(&self.telemetry, "telemetry"))
        }

        fn resource(&self) -> Self::Resource {
            Self::increment(&self.resource, "resource")
        }
    }

    #[test]
    fn native_capability_projection_covers_every_required_context_variant() {
        let cases = [
            NativeRequiredContext::None,
            NativeRequiredContext::Actor,
            NativeRequiredContext::File,
            NativeRequiredContext::Time,
            NativeRequiredContext::HttpClient,
            NativeRequiredContext::HttpResponseStream,
            NativeRequiredContext::Websocket,
            NativeRequiredContext::Telemetry,
            NativeRequiredContext::Resource,
        ];

        for required_context in cases {
            let source = TestProjectionSource::default();
            let projected = project_native_capability_context(required_context, &source);

            assert_eq!(projected.required_context(), required_context);
            match (required_context, projected) {
                (NativeRequiredContext::None, NativeCapabilityContexts::None) => {
                    assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 0, 0, 0]);
                }
                (NativeRequiredContext::Actor, NativeCapabilityContexts::Actor(value)) => {
                    assert_eq!(value, "actor");
                    assert_eq!(source.call_counts(), [1, 0, 0, 0, 0, 0, 0, 0]);
                }
                (NativeRequiredContext::File, NativeCapabilityContexts::File(value)) => {
                    assert_eq!(
                        value.into_parts(),
                        ("file", "file_source_stream", "heap_limits")
                    );
                    assert_eq!(source.call_counts(), [0, 1, 0, 0, 0, 0, 0, 0]);
                }
                (NativeRequiredContext::Time, NativeCapabilityContexts::Time(value)) => {
                    assert_eq!(value, "time");
                    assert_eq!(source.call_counts(), [0, 0, 1, 0, 0, 0, 0, 0]);
                }
                (
                    NativeRequiredContext::HttpClient,
                    NativeCapabilityContexts::HttpClient(value),
                ) => {
                    assert_eq!(value.into_effect_context(), "http_client");
                    assert_eq!(source.call_counts(), [0, 0, 0, 1, 0, 0, 0, 0]);
                }
                (
                    NativeRequiredContext::HttpResponseStream,
                    NativeCapabilityContexts::HttpResponseStream(value),
                ) => {
                    assert_eq!(value.into_execution_context(), "http_response_stream");
                    assert_eq!(source.call_counts(), [0, 0, 0, 0, 1, 0, 0, 0]);
                }
                (NativeRequiredContext::Websocket, NativeCapabilityContexts::Websocket(value)) => {
                    assert_eq!(value, "websocket");
                    assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 1, 0, 0]);
                }
                (NativeRequiredContext::Telemetry, NativeCapabilityContexts::Telemetry(value)) => {
                    assert_eq!(value.into_effect_context(), "telemetry");
                    assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 0, 1, 0]);
                }
                (NativeRequiredContext::Resource, NativeCapabilityContexts::Resource(value)) => {
                    assert_eq!(value, "resource");
                    assert_eq!(source.call_counts(), [0, 0, 0, 0, 0, 0, 0, 1]);
                }
                (expected, actual) => panic!(
                    "required context {expected:?} projected unexpected variant {:?}",
                    actual.required_context()
                ),
            }
        }
    }
}
