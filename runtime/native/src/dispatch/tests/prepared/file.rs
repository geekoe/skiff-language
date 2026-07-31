use super::*;

#[derive(Clone)]
pub(super) struct PendingFileCapability {
    calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl NativeFileCapability for PendingFileCapability {
    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
    ) -> NativeCapabilityFuture<'a, Value> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(PendingOnce::new(
            Value::Null,
            true,
            Arc::clone(&self.polls),
            Arc::clone(&self.drops),
        ))
    }

    fn read_file_wire<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        panic!("file read is not under test")
    }

    fn read_text_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        panic!("file readText is not under test")
    }

    fn file_info<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        panic!("file info is not under test")
    }

    fn delete_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, ()> {
        panic!("file delete is not under test")
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        _next_chunk: NativeFileChunkSource<'a>,
    ) -> NativeCapabilityFuture<'a, Value> {
        panic!("createFromStream is covered by its owner-specific tests")
    }
}

#[derive(Clone)]
pub(super) struct UnusedFileSource;

impl NativeFileSourceStreamCapability for UnusedFileSource {
    fn stream_consumer_cleanup(&self, _stream: &Value) -> StreamConsumerCleanup {
        panic!("plain file create does not consume a stream")
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        _stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        panic!("plain file create does not consume a stream")
    }
}

pub(super) struct TestFileBundle {
    file: PendingFileCapability,
}

impl NativeFileCapabilityBundle for TestFileBundle {
    type File = PendingFileCapability;
    type FileSourceStream = UnusedFileSource;

    fn into_native_file_parts(self) -> (Self::File, Self::FileSourceStream, RequestHeapLimits) {
        (self.file, UnusedFileSource, RequestHeapLimits::default())
    }
}

pub(super) struct NoTelemetry;

impl NativeTelemetryCapability for NoTelemetry {
    fn emit_native(&self, _target: &str, _args: &[Value]) -> Result<Value> {
        panic!("telemetry is not under test")
    }
}

fn file_create_invocation() -> RuntimeNativeInvocation {
    RuntimeNativeInvocation::new(
        "std.file.create".to_string(),
        "std.file.create",
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static("std.file.create"),
            vec![scalar_plan("bytes", RuntimeTypeNode::Bytes)],
            scalar_plan("null", RuntimeTypeNode::Null),
            NativeRequiredContext::File,
        )),
        None,
        None,
    )
}

#[test]
fn prepared_file_wait_is_heap_free_and_drop_cancels_the_single_started_future() {
    let calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let file = PendingFileCapability {
        calls: Arc::clone(&calls),
        polls: Arc::clone(&polls),
        drops: Arc::clone(&drops),
    };
    let mut heap = RequestHeap::default();
    let bytes = RuntimeValue::Heap(
        heap.alloc_bytes(b"payload".to_vec())
            .expect("bytes argument"),
    );
    let prepared = FileNativeDispatch::prepare(
        file,
        UnusedFileSource,
        RequestHeapLimits::default(),
        file_create_invocation(),
        "std.file.create".to_string(),
        vec![bytes],
        &mut heap,
    )
    .expect("file create should prepare");
    heap.alloc_bytes(b"caller still owns heap".to_vec())
        .expect("caller heap mutation while wait is live");
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("file create must expose a wait");
    };
    let (mut wait, _finalize) = operation.into_parts();
    assert!(matches!(poll_external_wait(&mut wait), Poll::Pending));
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(polls.load(Ordering::Acquire), 1);
    drop(wait);
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(drops.load(Ordering::Acquire), 1);
}
