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
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
};
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

    fn catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
        Some((
            PlatformBuiltinErrorIdentity::Http.catch_identity(),
            serde_json::json!({ "caught": true }),
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn file_capability_error_from_native_preserves_opaque_producer_payload() {
    let error = file_capability_error_from_native(RuntimeError::Opaque(Box::new(DummyWirePayload)));

    match error {
        FileCapabilityError::Stream(
            skiff_runtime_capability_context::StreamRuntimeError::Producer(error),
        ) => {
            assert_eq!(error.payload().code, "test.FileProducer");
            assert_eq!(
                error.catch_projection(),
                Some((
                    PlatformBuiltinErrorIdentity::Http.catch_identity(),
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
        skiff_runtime_capability_context::StreamConsumerCleanup::from_cancel(stream, move |_| {
            registry_entries.store(0, Ordering::Release);
            cancellations.fetch_add(1, Ordering::AcqRel);
        })
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
