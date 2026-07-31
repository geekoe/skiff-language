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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        scoped_file_future(execution_control, "std.file.create", async move {
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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        scoped_file_future(execution_control, "std.file.read", async move {
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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        scoped_file_future(execution_control, "std.file.readText", async move {
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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        scoped_file_future(execution_control, "std.file.info", async move {
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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, ()> {
        scoped_file_future(execution_control, "std.file.delete", async move {
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
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Value> {
        scoped_file_future(execution_control, "std.file.createFromStream", async move {
            self.0
                .create_file_from_chunks(target, options, move || next_chunk())
                .await
                .map_err(root_error_into_file)
        })
    }
}

fn scoped_file_future<'a, T, F>(
    execution_control: capability_contract::OwnedExecutionControl,
    operation: &'static str,
    lower: F,
) -> FileCapabilityFuture<'a, T>
where
    T: Send + 'a,
    F: Future<Output = capability_contract::FileCapabilityResult<T>> + Send + 'a,
{
    Box::pin(async move {
        let scope = execution_control.execution_scope().map_err(|error| {
            FileCapabilityError::decode(format!(
                "current execution scope is unavailable for {operation}: {error}"
            ))
        })?;
        let (lease, completion) = scope.acquire_lease();
        let lower = async move {
            let output = lower.await;
            (completion.complete(), output)
        };
        tokio::pin!(lower);
        tokio::select! {
            biased;
            (completed, output) = &mut lower => {
                if completed {
                    output
                } else {
                    Err(current_file_scope_terminal(&execution_control, None))
                }
            }
            terminal = lease.wait() => {
                match terminal {
                    capability_contract::ExecutionScopeLeaseTerminal::Control(terminal) => {
                        Err(current_file_scope_terminal(
                            &execution_control,
                            Some(terminal),
                        ))
                    }
                    capability_contract::ExecutionScopeLeaseTerminal::Completed => {
                        unreachable!("file scope lease completion is owned by the lower future")
                    }
                }
            }
        }
    })
}

fn current_file_scope_terminal(
    execution_control: &capability_contract::OwnedExecutionControl,
    terminal: Option<capability_contract::ExecutionScopeTerminal>,
) -> FileCapabilityError {
    match execution_control.borrow().poll_execution_budget() {
        Err(error) => FileCapabilityError::Execution(error),
        Ok(()) => match terminal {
            Some(capability_contract::ExecutionScopeTerminal::AncestorCancelled) => {
                FileCapabilityError::Execution(
                    capability_contract::ExecutionControlError::Cancelled,
                )
            }
            Some(
                capability_contract::ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                | capability_contract::ExecutionScopeTerminal::InheritedDeadlineExceeded(_),
            ) => FileCapabilityError::Execution(
                capability_contract::ExecutionControlError::BudgetExceeded(
                    capability_contract::ExecutionBudgetFailure {
                        reason: capability_contract::ExecutionBudgetReason::DeadlineExceeded,
                        instruction_count: 0,
                        limit: None,
                        elapsed_ms: 0.0,
                    },
                ),
            ),
            None => FileCapabilityError::decode(
                "file scope lease settled without a current execution terminal",
            ),
        },
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
        } if code == "ResourceLimitExceeded" => file_resource_limit_from_details(message, details),
        root_error::RuntimeError::Cancelled => FileCapabilityError::Execution(
            skiff_runtime_capability_context::ExecutionControlError::Cancelled,
        ),
        root_error::RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => FileCapabilityError::Execution(
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                skiff_runtime_capability_context::ExecutionBudgetFailure {
                    reason,
                    instruction_count,
                    limit,
                    elapsed_ms,
                },
            ),
        ),
        root_error::RuntimeError::Opaque(error) => file_capability_error_from_wire_payload(error),
        error => FileCapabilityError::opaque(
            root_error::OrdinaryRuntimeError::try_new(error)
                .expect("file cancellation was split before ordinary trait erasure"),
        ),
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
        FileCapabilityError::Opaque(error) => FileCapabilityError::Decode(error.to_string()),
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

fn file_resource_limit_from_details(
    message: String,
    details: Option<Value>,
) -> FileCapabilityError {
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
    pub(super) stream_runtime: capability_contract::StreamRuntime,
    pub(super) execution: skiff_runtime_request::OwnedExecutionControl,
}

impl capability_contract::FileSourceStreamApi for RuntimeOwnedFileSourceStreamContext {
    fn stream_runtime_handle(&self) -> capability_contract::StreamRuntime {
        self.stream_runtime.clone()
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        scoped_file_future(execution_control, "std.file.source.next", async move {
            concrete::FileSourceStreamContext::new(
                concrete_stream_runtime(&self.stream_runtime).clone(),
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

    fn channel_stream_with_lifetime(
        &self,
        lifetime: capability_contract::StreamLifetimeGuard,
    ) -> (Value, capability_contract::StreamSink) {
        let (value, sink) = self.0.channel_stream_with_lifetime(lifetime);
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

    fn next_with_cancellation<'a>(
        &'a self,
        value: &'a Value,
        signals: &'a [capability_contract::StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(async move {
            let signals = concrete_stream_cancel_signals(signals)?;
            let cancellation = capability_contract::CancellationSignals::from_tokens(cancel_tokens);
            self.0
                .next_with_cancellation(value, &signals, &cancellation)
                .await
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

    fn open_request_scope(&self, request_generation: u64) -> bool {
        self.0.open_scope(request_generation);
        true
    }

    fn close_request_scope(&self, request_generation: u64) {
        self.0.close_scope(request_generation);
    }

    fn close_owner(&self) {
        self.0.close_owner();
    }

    fn channel_stream_in_request_scope(
        &self,
        request_generation: u64,
    ) -> (Value, capability_contract::StreamSink) {
        let (value, sink) = self.0.channel_stream_in_scope(request_generation);
        (
            value,
            capability_contract::StreamSink::new(RuntimeStreamSink(sink)),
        )
    }

    fn channel_stream_with_lifetime_in_request_scope(
        &self,
        request_generation: u64,
        lifetime: capability_contract::StreamLifetimeGuard,
    ) -> (Value, capability_contract::StreamSink) {
        let (value, sink) = self
            .0
            .channel_stream_with_lifetime_in_scope(request_generation, lifetime);
        (
            value,
            capability_contract::StreamSink::new(RuntimeStreamSink(sink)),
        )
    }

    fn pull_stream_with_cancellation_in_request_scope(
        &self,
        request_generation: u64,
        source: Box<dyn StreamPullSource>,
        cancellation: CancellationToken,
    ) -> Value {
        self.0.pull_stream_with_cancellation_in_scope(
            BoxedStreamPullSource(source),
            cancellation,
            request_generation,
        )
    }

    fn buffered_stream_in_request_scope(
        &self,
        request_generation: u64,
        items: Vec<Value>,
    ) -> Value {
        self.0.buffered_stream_in_scope(items, request_generation)
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
    fn send_internal_with_cancellation<'a>(
        &'a self,
        item: capability_contract::StreamInternalItem,
        signals: &'a [capability_contract::StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let signals = concrete_stream_cancel_signals(signals)?;
            let cancellation = capability_contract::CancellationSignals::from_tokens(cancel_tokens);
            self.0
                .send_internal_with_stream_cancellation(item, &signals, &cancellation)
                .await
        })
    }

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

    fn send_with_cancellation<'a>(
        &'a self,
        item: Value,
        signals: &'a [capability_contract::StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let signals = concrete_stream_cancel_signals(signals)?;
            let cancellation = capability_contract::CancellationSignals::from_tokens(cancel_tokens);
            self.0
                .send_with_stream_cancellation(item, &signals, &cancellation)
                .await
        })
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

impl capability_contract::StreamCancelSignalApi for RuntimeStreamCancelSignal {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.0.wait_cancelled().await })
    }
}

#[cfg(test)]
mod tests;
