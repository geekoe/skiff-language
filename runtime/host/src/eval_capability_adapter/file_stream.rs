use super::*;

pub(super) struct RuntimeFileCapabilitySource(pub(super) concrete::FileCapabilitySource);

impl capability_contract::FileCapabilitySourceApi for RuntimeFileCapabilitySource {
    fn context_for_request(
        &self,
        db_context: capability_contract::DbCapabilityContext,
    ) -> capability_contract::FileCapabilityContext {
        let db_context = concrete_db_context(&db_context).clone();
        capability_contract::FileCapabilityContext::new(RuntimeFileCapabilityContext(
            self.0.context_for_request(db_context),
        ))
    }
}

#[derive(Clone)]
struct RuntimeFileCapabilityContext(concrete::FileCapabilityContext);

impl capability_contract::FileCapabilityApi for RuntimeFileCapabilityContext {
    fn source(&self) -> capability_contract::FileCapabilitySource {
        file_source(self.0.source())
    }

    fn create_file<'a>(
        &'a self,
        target: &'a str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .create_file(target, input, options)
                .await
                .map_err(root_error_into_file)
        })
    }

    fn read_file_wire<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .read_file_wire(target, file)
                .await
                .map_err(root_error_into_file)
        })
    }

    fn read_text_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .read_text_file(target, file)
                .await
                .map_err(root_error_into_file)
        })
    }

    fn file_info<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .file_info(target, file)
                .await
                .map_err(root_error_into_file)
        })
    }

    fn delete_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .delete_file(target, file)
                .await
                .map_err(root_error_into_file)
        })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        target: &'a str,
        options: FileCreateOptions,
        mut next_chunk: capability_contract::FileChunkSource<'a>,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .create_file_from_chunks(target, options, move || next_chunk())
                .await
                .map_err(root_error_into_file)
        })
    }
}

fn root_error_into_file(error: root_error::RuntimeError) -> FileCapabilityError {
    match error {
        root_error::RuntimeError::Decode(message) => FileCapabilityError::Decode(message),
        root_error::RuntimeError::Unsupported(message) => FileCapabilityError::Decode(message),
        root_error::RuntimeError::ProviderUnavailable { target, reason } => {
            FileCapabilityError::ProviderUnavailable { target, reason }
        }
        root_error::RuntimeError::Protocol { target, message } => {
            FileCapabilityError::Decode(format!("protocol error for {target}: {message}"))
        }
        root_error::RuntimeError::ExternalErrorPayload {
            code,
            message,
            details,
            ..
        } if code == "ResourceLimitExceeded" => {
            file_resource_limit_from_details(message, details)
        }
        root_error::RuntimeError::Opaque(error) => file_capability_error_from_wire_payload(error),
        error => FileCapabilityError::opaque(error),
    }
}

fn file_capability_error_from_wire_payload(
    error: Box<dyn skiff_runtime_model::error::WirePayload>,
) -> FileCapabilityError {
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::FileCapabilityError>()
    {
        return file_capability_error_from_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_model::error::RuntimeModelError>()
    {
        return file_capability_error_from_model_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_boundary::error::RuntimeError>()
    {
        return file_capability_error_from_boundary_ref(error);
    }
    FileCapabilityError::Opaque(error)
}

fn file_capability_error_from_ref(error: &FileCapabilityError) -> FileCapabilityError {
    match error {
        FileCapabilityError::Decode(message) => FileCapabilityError::Decode(message.clone()),
        FileCapabilityError::File(message) => FileCapabilityError::File(message.clone()),
        FileCapabilityError::Opaque(error) => {
            FileCapabilityError::Decode(error.to_string())
        }
        FileCapabilityError::ProviderUnavailable { target, reason } => {
            FileCapabilityError::ProviderUnavailable {
                target: target.clone(),
                reason: reason.clone(),
            }
        }
        FileCapabilityError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => FileCapabilityError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        FileCapabilityError::Stream(error) => FileCapabilityError::Stream(match error {
            skiff_runtime_capability_context::StreamRuntimeError::Decode(message) => {
                skiff_runtime_capability_context::StreamRuntimeError::Decode(message.clone())
            }
            skiff_runtime_capability_context::StreamRuntimeError::Cancelled => {
                skiff_runtime_capability_context::StreamRuntimeError::Cancelled
            }
            skiff_runtime_capability_context::StreamRuntimeError::Producer(error) => {
                return file_capability_error_from_wire_payload_ref(error.as_ref());
            }
        }),
        FileCapabilityError::Execution(error) => FileCapabilityError::Execution(*error),
    }
}

fn file_capability_error_from_wire_payload_ref(
    error: &dyn skiff_runtime_model::error::WirePayload,
) -> FileCapabilityError {
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_capability_context::FileCapabilityError>()
    {
        return file_capability_error_from_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_model::error::RuntimeModelError>()
    {
        return file_capability_error_from_model_ref(error);
    }
    if let Some(error) = error
        .as_any()
        .downcast_ref::<skiff_runtime_boundary::error::RuntimeError>()
    {
        return file_capability_error_from_boundary_ref(error);
    }
    FileCapabilityError::Decode(error.to_string())
}

fn file_capability_error_from_model_ref(
    error: &skiff_runtime_model::error::RuntimeModelError,
) -> FileCapabilityError {
    match error {
        skiff_runtime_model::error::RuntimeModelError::Decode(message) => {
            FileCapabilityError::Decode(message.clone())
        }
        skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => FileCapabilityError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        skiff_runtime_model::error::RuntimeModelError::Json(_) => {
            FileCapabilityError::Decode(error.to_string())
        }
    }
}

fn file_capability_error_from_boundary_ref(
    error: &skiff_runtime_boundary::error::RuntimeError,
) -> FileCapabilityError {
    match error {
        skiff_runtime_boundary::error::RuntimeError::Decode(message)
        | skiff_runtime_boundary::error::RuntimeError::Unsupported(message)
        | skiff_runtime_boundary::error::RuntimeError::InvalidArtifact(message) => {
            FileCapabilityError::Decode(message.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::DecodeTarget { target, message } => {
            FileCapabilityError::Decode(format!("decode error for {target}: {message}"))
        }
        skiff_runtime_boundary::error::RuntimeError::BytesDecode { target, message } => {
            FileCapabilityError::Decode(format!("bytes decode error for {target}: {message}"))
        }
        skiff_runtime_boundary::error::RuntimeError::DbDecode { target, message } => {
            FileCapabilityError::Decode(format!("db decode error for {target}: {message}"))
        }
        skiff_runtime_boundary::error::RuntimeError::FileError { message } => {
            FileCapabilityError::File(message.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::HttpError { message, .. } => {
            FileCapabilityError::Decode(message.clone())
        }
        skiff_runtime_boundary::error::RuntimeError::Recoverable(error) => {
            FileCapabilityError::Decode(error.to_string())
        }
        skiff_runtime_boundary::error::RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => FileCapabilityError::ResourceLimitExceeded {
            resource: resource.clone(),
            reason: reason.clone(),
            limit: *limit,
            current: *current,
            requested_delta: *requested_delta,
        },
        skiff_runtime_boundary::error::RuntimeError::Json(_) => {
            FileCapabilityError::Decode(error.to_string())
        }
    }
}

fn file_resource_limit_from_details(message: String, details: Option<Value>) -> FileCapabilityError {
    let Some(details) = details else {
        return FileCapabilityError::Decode(message);
    };
    let Some(resource) = details.get("resource").and_then(Value::as_str) else {
        return FileCapabilityError::Decode(message);
    };
    let Some(reason) = details.get("reason").and_then(Value::as_str) else {
        return FileCapabilityError::Decode(message);
    };
    let Some(limit) = details
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return FileCapabilityError::Decode(message);
    };
    let Some(current) = details
        .get("current")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return FileCapabilityError::Decode(message);
    };
    let Some(requested_delta) = details
        .get("requestedDelta")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return FileCapabilityError::Decode(message);
    };
    FileCapabilityError::ResourceLimitExceeded {
        resource: resource.to_string(),
        reason: reason.to_string(),
        limit,
        current,
        requested_delta,
    }
}

#[derive(Clone)]
pub(super) struct RuntimeOwnedFileSourceStreamContext {
    pub(super) stream_runtime: concrete::StreamRuntime,
    pub(super) execution: skiff_runtime_request::OwnedExecutionControl,
}

impl capability_contract::FileSourceStreamApi for RuntimeOwnedFileSourceStreamContext {
    fn stream_runtime_handle(&self) -> capability_contract::StreamRuntime {
        capability_contract::StreamRuntime::new(RuntimeStreamRuntime(self.stream_runtime.clone()))
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            concrete::FileSourceStreamContext::new(
                self.stream_runtime.clone(),
                self.execution.borrow(),
            )
            .next_file_source_stream_item(stream)
            .await
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct RuntimeStreamRuntime(pub(super) concrete::StreamRuntime);

impl capability_contract::StreamRuntimeApi for RuntimeStreamRuntime {
    fn channel_stream(&self) -> (Value, capability_contract::StreamSink) {
        let (value, sink) = self.0.channel_stream();
        (
            value,
            capability_contract::StreamSink::new(RuntimeStreamSink(sink)),
        )
    }

    fn pull_stream_with_cancellation(
        &self,
        source: Box<dyn StreamPullSource>,
        cancellation: CancellationToken,
    ) -> Value {
        self.0
            .pull_stream_with_cancellation(BoxedStreamPullSource(source), cancellation)
    }

    fn buffered_stream(&self, items: Vec<Value>) -> Value {
        self.0.buffered_stream(items)
    }

    fn next_with_cancel<'a>(
        &'a self,
        value: &'a Value,
        signals: &'a [capability_contract::StreamCancelSignal],
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(async move {
            let signals = concrete_stream_cancel_signals(signals)?;
            self.0.next_with_cancel(value, &signals, cancel_flags).await
        })
    }

    fn next<'a>(
        &'a self,
        value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(async move { self.0.next(value).await })
    }

    fn cancel(&self, value: &Value) {
        self.0.cancel(value);
    }
}

struct BoxedStreamPullSource(Box<dyn StreamPullSource>);

impl StreamPullSource for BoxedStreamPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        self.0.next()
    }
}

#[derive(Clone, Debug)]
struct RuntimeStreamSink(concrete::StreamSink);

impl capability_contract::StreamSinkApi for RuntimeStreamSink {
    fn send<'a>(
        &'a self,
        item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move { self.0.send(item).await })
    }

    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move { self.0.send_with_cancel(item, cancel_flags).await })
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.0.end().await })
    }

    fn fail<'a>(
        &'a self,
        error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.0.fail(error).await })
    }

    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    fn is_same_stream(&self, other: &capability_contract::StreamSink) -> bool {
        other
            .downcast_ref::<RuntimeStreamSink>()
            .is_some_and(|other| self.0.is_same_stream(&other.0))
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.0.cancel_flag()
    }

    fn cancel_signal(&self) -> capability_contract::StreamCancelSignal {
        capability_contract::StreamCancelSignal::new(RuntimeStreamCancelSignal(
            self.0.cancel_signal(),
        ))
    }
}

#[derive(Debug)]
pub(super) struct RuntimeStreamCancelSignal(pub(super) concrete::StreamCancelSignal);

impl capability_contract::StreamCancelSignalApi for RuntimeStreamCancelSignal {}
