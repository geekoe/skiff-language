use super::{common::*, imports::*};

#[derive(Clone)]
struct HarnessFileSource;

impl FileCapabilitySourceApi for HarnessFileSource {
    fn context_for_request(&self, _db_context: DbCapabilityContext) -> FileCapabilityContext {
        FileCapabilityContext::new(HarnessFile)
    }
}

#[derive(Clone)]
pub(super) struct HarnessFile;

fn unavailable_file<'a, T>() -> FileCapabilityFuture<'a, T> {
    Box::pin(async {
        Err(skiff_runtime_capability_context::FileCapabilityError::file(
            "combined test file capability is unavailable",
        ))
    })
}

impl FileCapabilityApi for HarnessFile {
    fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::new(HarnessFileSource)
    }

    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn read_file_wire<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn read_text_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn file_info<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn delete_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, ()> {
        unavailable_file()
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        _next_chunk: FileChunkSource<'a>,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }
}

#[derive(Clone)]
pub(super) struct HarnessFileSourceStream {
    pub(super) stream_runtime: StreamRuntime,
}

impl FileSourceStreamApi for HarnessFileSourceStream {
    fn stream_runtime_handle(&self) -> StreamRuntime {
        self.stream_runtime.clone()
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
        _execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            match self.stream_runtime.next(stream).await? {
                StreamPoll::Item(item) => Ok(Some(item)),
                StreamPoll::End => Ok(None),
                StreamPoll::InternalItem(_) => Err(
                    skiff_runtime_capability_context::FileCapabilityError::decode(
                        "combined test does not project internal file stream items",
                    ),
                ),
            }
        })
    }
}

#[derive(Clone)]
pub(super) struct HarnessActor;

impl ActorCapabilityApi for HarnessActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        unsupported_capability()
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        unsupported_capability()
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        unsupported_capability()
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, bool> {
        unsupported_capability()
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: ActorInvocationRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorInvocationOutcome> {
        unsupported_capability()
    }
}

impl RequestCapabilityApi for HarnessActor {
    fn owned(&self) -> OwnedRequestCapabilityContext {
        RequestCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> RequestCapabilityContext<'_> {
        RequestCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "combined-runtime"
    }

    fn service_id(&self) -> &str {
        SERVICE_ID
    }

    fn service_version(&self) -> &str {
        VERSION
    }

    fn request_id(&self) -> &str {
        "combined-request"
    }

    fn request_target(&self) -> &str {
        SERVICE_ID
    }

    fn request_build_id(&self) -> &str {
        "combined-build"
    }

    fn task_service_protocol_identity(&self) -> &str {
        ""
    }

    fn request_service_protocol_identity(&self) -> &str {
        ""
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        None
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        None
    }

    fn trace_id(&self) -> Option<&str> {
        None
    }

    fn submit_task<'a>(
        &'a self,
        _request: TaskSubmitControlRequest,
        _args_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskSubmitResponseControl> {
        unsupported_capability()
    }
}

fn unsupported_capability<'a, T>() -> CapabilityFuture<'a, T> {
    Box::pin(async {
        Err(CapabilityError::unsupported(
            "combined test capability is unavailable",
        ))
    })
}

#[derive(Clone)]
pub(super) struct HarnessWebsocket;

impl WebsocketCapabilityApi for HarnessWebsocket {
    fn owned(&self) -> OwnedWebsocketCapabilityContext {
        SharedWebsocketCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> SharedWebsocketCapabilityContext<'_> {
        SharedWebsocketCapabilityContext::new(self.clone())
    }

    fn service_id(&self) -> &str {
        SERVICE_ID
    }

    fn websocket_entry_id(&self) -> Option<&str> {
        None
    }

    fn send_connection_text_to_business_identity(
        &self,
        _business_identity: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }

    fn send_connection_binary_to_business_identity(
        &self,
        _business_identity: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }

    fn send_connection_text_to_connection(
        &self,
        _connection_id: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }

    fn send_connection_binary_to_connection(
        &self,
        _connection_id: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }
}

pub(super) struct HarnessEffects;

impl EffectDispatchApi for HarnessEffects {
    fn telemetry_context(&self) -> TelemetryCapabilityContext {
        TelemetryCapabilityContext::new(HarnessTelemetry)
    }

    fn http_client_context(
        &self,
        _http_options: HttpRuntimeOptions,
        _stream_runtime: StreamRuntime,
        _test_effect_doubles: TestEffectDoubleContext,
    ) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(HarnessHttp)
    }
}

struct HarnessTelemetry;

impl TelemetryCapabilityApi for HarnessTelemetry {
    fn emit_native(
        &self,
        _target: &str,
        _args: &[Value],
    ) -> skiff_runtime_capability_context::CapabilityResult<Value> {
        Err(CapabilityError::unsupported(
            "combined test telemetry capability is unavailable",
        ))
    }
}

struct HarnessHttp;

impl HttpClientCapabilityApi for HarnessHttp {
    fn with_stream_runtime(&self, _stream_runtime: StreamRuntime) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(Self)
    }

    fn dispatch_http_request<'a>(
        &'a self,
        _input: &'a Value,
        _execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, Value> {
        unsupported_capability()
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        _input: &'a Value,
        _expected_body_item_type: Option<&'a RuntimeTypePlan>,
        _execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, Value> {
        unsupported_capability()
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        _input: &'a Value,
        _expected_item_type: Option<&'a RuntimeTypePlan>,
        _execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, Value> {
        unsupported_capability()
    }
}
