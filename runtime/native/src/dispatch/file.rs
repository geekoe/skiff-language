use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_boundary::file::{
    create_options_from_wire, immutable_file_from_wire, FileCreateOptions, ImmutableFileRef,
};
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};
use skiff_runtime_capability_context::{FileCapabilityError, StreamConsumerCleanup};

use super::{unsupported_native_target, RuntimeNativeInvocation};
use crate::error::{Result, RuntimeError};
use crate::{
    call_helpers::runtime_string_arg,
    capability::{NativeFileCapability, NativeFileChunkFuture, NativeFileSourceStreamCapability},
    runtime_value_facade::{
        bytes_payload, RequestHeap, RequestHeapLimits, RuntimeTypeNode, RuntimeTypePlan,
        RuntimeValue,
    },
};

pub(super) struct FileNativeDispatch;

impl FileNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "std.file.create"
                | "std.file.createText"
                | "std.file.read"
                | "std.file.readText"
                | "std.file.info"
                | "std.file.delete"
                | "std.file.createFromStream"
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch<FileContext>(
        file_context: &FileContext,
        file_source_stream_context: &impl NativeFileSourceStreamCapability,
        request_heap_limits: RequestHeapLimits,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        FileContext: NativeFileCapability,
    {
        let binding_key = invocation.binding_key();
        let output = match binding_key {
            "std.file.create" => {
                let content = bytes_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                let options =
                    file_options_arg(diagnostic_target, invocation, &args, 1, None, heap)?;
                file_context
                    .create_file(diagnostic_target, Bytes::from(content), options)
                    .await?
            }
            "std.file.createText" => {
                let content = string_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                let options = file_options_arg(
                    diagnostic_target,
                    invocation,
                    &args,
                    1,
                    Some("text/plain; charset=utf-8"),
                    heap,
                )?;
                file_context
                    .create_file(
                        diagnostic_target,
                        Bytes::from(content.into_bytes()),
                        options,
                    )
                    .await?
            }
            "std.file.read" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context
                    .read_file_wire(diagnostic_target, &file)
                    .await?
            }
            "std.file.readText" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context
                    .read_text_file(diagnostic_target, &file)
                    .await?
            }
            "std.file.info" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context.file_info(diagnostic_target, &file).await?
            }
            "std.file.delete" => {
                let file = file_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                file_context.delete_file(diagnostic_target, &file).await?;
                Value::Null
            }
            "std.file.createFromStream" => {
                let stream = stream_arg_from_plan(diagnostic_target, invocation, &args, 0, heap)?;
                let cleanup = file_source_stream_context.stream_consumer_cleanup(&stream);
                let options =
                    file_options_arg(diagnostic_target, invocation, &args, 1, None, heap)?;
                let item_plan =
                    file_stream_item_plan(diagnostic_target, invocation.arg_plan(0)?)?.clone();
                create_file_from_stream(
                    file_context,
                    file_source_stream_context,
                    diagnostic_target,
                    CreateFileFromStreamInput {
                        stream,
                        options,
                        item_plan,
                        request_heap_limits,
                        cleanup,
                    },
                )
                .await?
            }
            _ => return Err(unsupported_native_target(binding_key)),
        };

        invocation.native_boundary()?.from_wire_return(
            &output,
            &format!("{diagnostic_target} response"),
            heap,
        )
    }
}

struct CreateFileFromStreamInput {
    stream: Value,
    options: FileCreateOptions,
    item_plan: RuntimeTypePlan,
    request_heap_limits: RequestHeapLimits,
    cleanup: StreamConsumerCleanup,
}

async fn create_file_from_stream<FileContext, SourceContext>(
    file_context: &FileContext,
    file_source_stream_context: &SourceContext,
    diagnostic_target: &str,
    input: CreateFileFromStreamInput,
) -> Result<Value>
where
    FileContext: NativeFileCapability,
    SourceContext: NativeFileSourceStreamCapability,
{
    let CreateFileFromStreamInput {
        stream,
        options,
        item_plan,
        request_heap_limits,
        mut cleanup,
    } = input;
    let end_marker = cleanup.end_marker();
    let source_context = file_source_stream_context.clone();
    let chunk_end_marker = end_marker.clone();
    let output = file_context
        .create_file_from_chunks(
            diagnostic_target,
            options,
            Box::new(move || {
                let source_context = source_context.clone();
                let stream = stream.clone();
                let item_plan = item_plan.clone();
                let request_heap_limits = request_heap_limits.clone();
                let end_marker = chunk_end_marker.clone();
                Box::pin(async move {
                    let Some(item) = source_context.next_file_source_stream_item(&stream).await?
                    else {
                        end_marker.mark_reached_end();
                        return Ok(None);
                    };
                    let mut item_heap = RequestHeap::new(request_heap_limits.clone());
                    let codec = RuntimeBoundaryContract::default().codec_for_expected(
                        &item_plan,
                        BoundaryUse::TypedJson,
                        "std.file.createFromStream item",
                    );
                    let value = codec
                        .from_wire_json(&item, &mut item_heap)
                        .map_err(|error| {
                            file_capability_error_from_native(RuntimeError::from(error))
                        })?;
                    let wire = codec
                        .to_wire_json(&value, &mut item_heap)
                        .map_err(|error| {
                            file_capability_error_from_native(RuntimeError::from(error))
                        })?;
                    let bytes = bytes_payload(&wire).ok_or_else(|| {
                        FileCapabilityError::Decode(
                            "std.file.createFromStream item must be bytes".to_string(),
                        )
                    })?;
                    Ok(Some(Bytes::from(bytes)))
                }) as NativeFileChunkFuture<'_>
            }),
        )
        .await;
    if output.is_ok() && end_marker.has_reached_end() {
        cleanup.disarm_after_end();
    }
    output
}

fn file_capability_error_from_native(error: RuntimeError) -> FileCapabilityError {
    match error {
        RuntimeError::Decode(message) => FileCapabilityError::Decode(message),
        RuntimeError::DecodeTarget { target, message } => {
            FileCapabilityError::Decode(format!("decode error for {target}: {message}"))
        }
        RuntimeError::BytesDecode { target, message } => {
            FileCapabilityError::Decode(format!("bytes decode error for {target}: {message}"))
        }
        RuntimeError::DbDecode { target, message } => {
            FileCapabilityError::Decode(format!("db decode error for {target}: {message}"))
        }
        RuntimeError::FileError { message } => FileCapabilityError::File(message),
        RuntimeError::ResourceError { path, message } => {
            FileCapabilityError::Decode(format!("resource error for {path}: {message}"))
        }
        RuntimeError::Cancelled => FileCapabilityError::Execution(
            skiff_runtime_capability_context::ExecutionControlError::Cancelled,
        ),
        RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => FileCapabilityError::Execution(
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                skiff_runtime_capability_context::ExecutionBudgetFailure {
                    reason: file_capability_budget_reason(reason),
                    instruction_count,
                    limit,
                    elapsed_ms,
                },
            ),
        ),
        RuntimeError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        } => FileCapabilityError::ResourceLimitExceeded {
            resource,
            reason,
            limit,
            current,
            requested_delta,
        },
        RuntimeError::InvalidArtifact(message)
        | RuntimeError::HttpError { message, detail: _ }
        | RuntimeError::Unsupported(message) => FileCapabilityError::Decode(message),
        RuntimeError::Recoverable(error) => FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::producer(
                RuntimeError::Recoverable(error),
            ),
        ),
        RuntimeError::Opaque(error) => FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::Producer(error),
        ),
        RuntimeError::Json(error) => FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::producer(RuntimeError::Json(
                error,
            )),
        ),
    }
}

fn file_capability_budget_reason(
    reason: crate::error::BudgetReason,
) -> skiff_runtime_capability_context::ExecutionBudgetReason {
    match reason {
        crate::error::BudgetReason::Cancelled => {
            skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
        }
        crate::error::BudgetReason::DeadlineExceeded => {
            skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded
        }
        crate::error::BudgetReason::InstructionLimitExceeded => {
            skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded
        }
    }
}

fn bytes_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<Vec<u8>> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires argument {index}")))?;
    let wire = invocation.native_boundary()?.to_wire_arg(
        index,
        arg,
        &format!("{target} argument {index}"),
        heap,
    )?;
    bytes_payload(&wire)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} argument {index} must be bytes")))
}

fn string_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<String> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires argument {index}")))?;
    let coerced = invocation.native_boundary()?.coerce_arg(
        index,
        arg,
        &format!("{target} argument {index}"),
        heap,
    )?;
    runtime_string_arg(&coerced, &format!("{target} argument {index}")).map(str::to_string)
}

fn file_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<ImmutableFileRef> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires file")))?;
    let wire =
        invocation
            .native_boundary()?
            .to_wire_arg(index, arg, &format!("{target} file"), heap)?;
    Ok(immutable_file_from_wire(&wire, target)?)
}

fn file_options_arg(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    default_content_type: Option<&str>,
    heap: &mut RequestHeap,
) -> Result<FileCreateOptions> {
    let Some(arg) = args.get(index) else {
        return Ok(create_options_from_wire(
            None,
            default_content_type,
            target,
        )?);
    };
    let wire = invocation.native_boundary()?.to_wire_arg(
        index,
        arg,
        &format!("{target} options"),
        heap,
    )?;
    Ok(create_options_from_wire(
        Some(&wire),
        default_content_type,
        target,
    )?)
}

fn stream_arg_from_plan(
    target: &str,
    invocation: &RuntimeNativeInvocation,
    args: &[RuntimeValue],
    index: usize,
    heap: &mut RequestHeap,
) -> Result<Value> {
    let arg = args
        .get(index)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} requires stream")))?;
    invocation
        .native_boundary()?
        .to_wire_arg(index, arg, &format!("{target} stream"), heap)
}

fn file_stream_item_plan<'a>(
    target: &str,
    stream_plan: &'a RuntimeTypePlan,
) -> Result<&'a RuntimeTypePlan> {
    match stream_plan.node() {
        RuntimeTypeNode::Stream(item) if matches!(item.node(), RuntimeTypeNode::Bytes) => Ok(item),
        RuntimeTypeNode::Stream(_) => Err(RuntimeError::InvalidArtifact(format!(
            "{target} source must be Stream<bytes>"
        ))),
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "{target} source argument is not a Stream"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        fmt,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use skiff_runtime_boundary::stream::stream_value;
    use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};
    use skiff_runtime_native_contract::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

    #[derive(Debug)]
    struct DummyWirePayload;

    impl fmt::Display for DummyWirePayload {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("dummy producer payload")
        }
    }

    impl std::error::Error for DummyWirePayload {}

    impl WirePayload for DummyWirePayload {
        fn payload(&self) -> RuntimeErrorPayload {
            RuntimeErrorPayload {
                code: "test.FileProducer".to_string(),
                message: "dummy producer payload".to_string(),
                status: None,
                details: Some(serde_json::json!({ "producer": true })),
            }
        }

        fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
            Some((
                TypeIdentity::builtin("test.FileProducerCatch"),
                serde_json::json!({ "caught": true }),
            ))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn file_capability_error_from_native_preserves_opaque_producer_payload() {
        let error =
            file_capability_error_from_native(RuntimeError::Opaque(Box::new(DummyWirePayload)));

        match error {
            FileCapabilityError::Stream(
                skiff_runtime_capability_context::StreamRuntimeError::Producer(error),
            ) => {
                assert_eq!(error.payload().code, "test.FileProducer");
                assert_eq!(
                    error.catch_projection(),
                    Some((
                        TypeIdentity::builtin("test.FileProducerCatch"),
                        serde_json::json!({ "caught": true }),
                    ))
                );
            }
            error => panic!("expected stream producer, got {error:?}"),
        }
    }

    #[derive(Clone)]
    struct TestFileSourceStream {
        items: Arc<Mutex<VecDeque<Value>>>,
        polls: Arc<AtomicUsize>,
        registry_entries: Arc<AtomicUsize>,
        cancellations: Arc<AtomicUsize>,
    }

    impl TestFileSourceStream {
        fn new(items: impl IntoIterator<Item = Value>) -> Self {
            Self {
                items: Arc::new(Mutex::new(items.into_iter().collect())),
                polls: Arc::new(AtomicUsize::new(0)),
                registry_entries: Arc::new(AtomicUsize::new(1)),
                cancellations: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn poll_count(&self) -> usize {
            self.polls.load(Ordering::Acquire)
        }

        fn cancellation_count(&self) -> usize {
            self.cancellations.load(Ordering::Acquire)
        }

        fn registry_entry_count(&self) -> usize {
            self.registry_entries.load(Ordering::Acquire)
        }
    }

    impl NativeFileSourceStreamCapability for TestFileSourceStream {
        fn stream_consumer_cleanup(
            &self,
            stream: &Value,
        ) -> skiff_runtime_capability_context::StreamConsumerCleanup {
            let cancellations = self.cancellations.clone();
            let registry_entries = self.registry_entries.clone();
            skiff_runtime_capability_context::StreamConsumerCleanup::from_cancel(
                stream,
                move |_| {
                    registry_entries.store(0, Ordering::Release);
                    cancellations.fetch_add(1, Ordering::AcqRel);
                },
            )
        }

        fn next_file_source_stream_item<'a>(
            &'a self,
            _stream: &'a Value,
        ) -> skiff_runtime_capability_context::FileCapabilityFuture<'a, Option<Value>> {
            Box::pin(async move {
                self.polls.fetch_add(1, Ordering::AcqRel);
                let item = self.items.lock().expect("source items lock").pop_front();
                if item.is_none() {
                    self.registry_entries.store(0, Ordering::Release);
                }
                Ok(item)
            })
        }
    }

    #[derive(Clone, Copy)]
    enum TestFileBehavior {
        ConsumeToEnd,
        WriteErrorAfterFirstChunk,
        CommitErrorAfterEnd,
    }

    #[derive(Clone)]
    struct TestFileCapability {
        behavior: TestFileBehavior,
        chunks: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl TestFileCapability {
        fn new(behavior: TestFileBehavior) -> Self {
            Self {
                behavior,
                chunks: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn chunks(&self) -> Vec<Vec<u8>> {
            self.chunks.lock().expect("file chunks lock").clone()
        }
    }

    impl NativeFileCapability for TestFileCapability {
        fn create_file<'a>(
            &'a self,
            _target: &'a str,
            _input: Bytes,
            _options: FileCreateOptions,
        ) -> crate::capability::NativeCapabilityFuture<'a, Value> {
            Box::pin(async { panic!("unexpected create_file call") })
        }

        fn read_file_wire<'a>(
            &'a self,
            _target: &'a str,
            _file: &'a ImmutableFileRef,
        ) -> crate::capability::NativeCapabilityFuture<'a, Value> {
            Box::pin(async { panic!("unexpected read_file_wire call") })
        }

        fn read_text_file<'a>(
            &'a self,
            _target: &'a str,
            _file: &'a ImmutableFileRef,
        ) -> crate::capability::NativeCapabilityFuture<'a, Value> {
            Box::pin(async { panic!("unexpected read_text_file call") })
        }

        fn file_info<'a>(
            &'a self,
            _target: &'a str,
            _file: &'a ImmutableFileRef,
        ) -> crate::capability::NativeCapabilityFuture<'a, Value> {
            Box::pin(async { panic!("unexpected file_info call") })
        }

        fn delete_file<'a>(
            &'a self,
            _target: &'a str,
            _file: &'a ImmutableFileRef,
        ) -> crate::capability::NativeCapabilityFuture<'a, ()> {
            Box::pin(async { panic!("unexpected delete_file call") })
        }

        fn create_file_from_chunks<'a>(
            &'a self,
            _target: &'a str,
            _options: FileCreateOptions,
            mut next_chunk: crate::capability::NativeFileChunkSource<'a>,
        ) -> crate::capability::NativeCapabilityFuture<'a, Value> {
            Box::pin(async move {
                loop {
                    let Some(chunk) = next_chunk().await.map_err(RuntimeError::from)? else {
                        break;
                    };
                    self.chunks
                        .lock()
                        .expect("file chunks lock")
                        .push(chunk.to_vec());
                    if matches!(self.behavior, TestFileBehavior::WriteErrorAfterFirstChunk) {
                        return Err(RuntimeError::file_error("test file write failed"));
                    }
                }

                if matches!(self.behavior, TestFileBehavior::CommitErrorAfterEnd) {
                    return Err(RuntimeError::file_error("test file commit failed"));
                }
                Ok(Value::Null)
            })
        }
    }

    async fn dispatch_create_from_stream(
        items: impl IntoIterator<Item = Value>,
        file_behavior: TestFileBehavior,
    ) -> (
        Result<RuntimeValue>,
        TestFileSourceStream,
        TestFileCapability,
    ) {
        let bytes_plan = RuntimeTypePlan::new("bytes", None, RuntimeTypeNode::Bytes);
        let stream_plan = RuntimeTypePlan::synthetic_stream(bytes_plan);
        let return_plan = RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null);
        let invocation = RuntimeNativeInvocation::new(
            "std.file.createFromStream".to_string(),
            "std.file.createFromStream",
            Some(NativeCallPlan::new(
                NativeBindingKey::from_static("std.file.createFromStream"),
                vec![stream_plan.clone()],
                return_plan,
                NativeRequiredContext::File,
            )),
            None,
            None,
        );
        let mut heap = RequestHeap::default();
        let stream = stream_value("create-from-stream-test");
        let stream_arg = RuntimeBoundaryContract::default()
            .codec_for_expected(&stream_plan, BoundaryUse::NativeArg, "test stream")
            .from_wire_json(&stream, &mut heap)
            .expect("stream argument should decode");
        let source = TestFileSourceStream::new(items);
        let file = TestFileCapability::new(file_behavior);
        let result = FileNativeDispatch::dispatch(
            &file,
            &source,
            RequestHeapLimits::default(),
            &invocation,
            "std.file.createFromStream",
            vec![stream_arg],
            &mut heap,
        )
        .await;
        (result, source, file)
    }

    #[tokio::test]
    async fn create_from_stream_item_decode_error_cancels_immediately() {
        let (result, source, file) = dispatch_create_from_stream(
            [Value::String("not bytes".to_string())],
            TestFileBehavior::ConsumeToEnd,
        )
        .await;

        assert!(matches!(result, Err(RuntimeError::Decode(_))));
        assert_eq!(source.cancellation_count(), 1);
        assert_eq!(source.registry_entry_count(), 0);
        assert_eq!(source.poll_count(), 1);
        assert!(file.chunks().is_empty());
    }

    #[tokio::test]
    async fn create_from_stream_file_write_error_cancels_immediately() {
        let (result, source, file) = dispatch_create_from_stream(
            [crate::runtime_value_facade::bytes_value(b"first")],
            TestFileBehavior::WriteErrorAfterFirstChunk,
        )
        .await;

        assert!(matches!(result, Err(RuntimeError::FileError { .. })));
        assert_eq!(source.cancellation_count(), 1);
        assert_eq!(source.registry_entry_count(), 0);
        assert_eq!(source.poll_count(), 1);
        assert_eq!(file.chunks(), vec![b"first".to_vec()]);
    }

    #[tokio::test]
    async fn create_from_stream_commit_error_after_end_still_cancels() {
        let (result, source, file) = dispatch_create_from_stream(
            [crate::runtime_value_facade::bytes_value(b"complete")],
            TestFileBehavior::CommitErrorAfterEnd,
        )
        .await;

        assert!(matches!(result, Err(RuntimeError::FileError { .. })));
        assert_eq!(source.cancellation_count(), 1);
        assert_eq!(source.registry_entry_count(), 0);
        assert_eq!(source.poll_count(), 2);
        assert_eq!(file.chunks(), vec![b"complete".to_vec()]);
    }

    #[tokio::test]
    async fn create_from_stream_natural_end_disarms_without_extra_cancel() {
        let (result, source, file) = dispatch_create_from_stream(
            [
                crate::runtime_value_facade::bytes_value(b"first"),
                crate::runtime_value_facade::bytes_value(b"second"),
            ],
            TestFileBehavior::ConsumeToEnd,
        )
        .await;

        assert_eq!(
            result.expect("createFromStream should succeed"),
            RuntimeValue::Null
        );
        assert_eq!(source.cancellation_count(), 0);
        assert_eq!(source.registry_entry_count(), 0);
        assert_eq!(source.poll_count(), 3);
        assert_eq!(file.chunks(), vec![b"first".to_vec(), b"second".to_vec()]);
    }
}
