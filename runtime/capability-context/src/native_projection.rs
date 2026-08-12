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
    ConfigContext = (),
    DbContext = (),
> {
    None,
    Actor(ActorContext),
    Config(ConfigContext),
    File(FileContext),
    Db(DbContext),
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
        ConfigContext,
        DbContext,
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
        ConfigContext,
        DbContext,
    >
{
    pub fn required_context(&self) -> NativeRequiredContext {
        match self {
            Self::None => NativeRequiredContext::None,
            Self::Actor(_) => NativeRequiredContext::Actor,
            Self::Config(_) => NativeRequiredContext::Config,
            Self::File(_) => NativeRequiredContext::File,
            Self::Db(_) => NativeRequiredContext::Db,
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
    type Config;
    type Db;

    fn actor(&self) -> Self::Actor;
    fn file(&self) -> Self::File;
    fn time(&self) -> Self::Time;
    fn http_client(&self) -> Self::HttpClient;
    fn http_response_stream(&self) -> Self::HttpResponseStream;
    fn websocket(&self) -> Self::Websocket;
    fn telemetry(&self) -> Self::Telemetry;
    fn resource(&self) -> Self::Resource;
    fn config(&self) -> Self::Config;
    fn db(&self) -> Self::Db;
}

pub type ProjectedNativeCapabilityContexts<Source> = NativeCapabilityContexts<
    <Source as NativeCapabilityProjectionSource>::Actor,
    <Source as NativeCapabilityProjectionSource>::File,
    <Source as NativeCapabilityProjectionSource>::Time,
    <Source as NativeCapabilityProjectionSource>::HttpClient,
    <Source as NativeCapabilityProjectionSource>::HttpResponseStream,
    <Source as NativeCapabilityProjectionSource>::Websocket,
    <Source as NativeCapabilityProjectionSource>::Telemetry,
    <Source as NativeCapabilityProjectionSource>::Resource,
    <Source as NativeCapabilityProjectionSource>::Config,
    <Source as NativeCapabilityProjectionSource>::Db,
>;

pub fn project_native_capability_context<Source>(
    required_context: NativeRequiredContext,
    source: &Source,
) -> ProjectedNativeCapabilityContexts<Source>
where
    Source: NativeCapabilityProjectionSource,
{
    match required_context {
        NativeRequiredContext::None => NativeCapabilityContexts::None,
        NativeRequiredContext::Actor => NativeCapabilityContexts::Actor(source.actor()),
        NativeRequiredContext::Config => NativeCapabilityContexts::Config(source.config()),
        NativeRequiredContext::File => NativeCapabilityContexts::File(source.file()),
        NativeRequiredContext::Db => NativeCapabilityContexts::Db(source.db()),
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
mod tests;
