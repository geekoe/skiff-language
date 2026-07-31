use super::*;

fn emit_fixture(value: &str) -> EvaluatorFixture {
    EvaluatorFixture::new(
        vec![LinkedExprIr::Literal {
            value: LiteralIr::String {
                value: value.to_string(),
            },
        }],
        vec![
            LinkedStmtIr::Emit {
                operation: "emit".to_string(),
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

#[tokio::test]
async fn f445h_e4r_spine_emit_detached_ready_keeps_actor_segment() {
    let fixture = emit_fixture("ready");
    let (_stream, sink) = fixture.interpreter.stream_runtime.channel_stream();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("buffered detached emit");

    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish ready emit frame");
}

#[tokio::test]
async fn f445h_e4r_spine_emit_detached_pending_cuts_actor_segment_once() {
    let fixture = emit_fixture("pending");
    let (stream, sink) = fixture.interpreter.stream_runtime.channel_stream();
    sink.send(json!("prefill")).await.expect("prefill stream");
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    let mut eval = fixture.eval_context(frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(!frame.has_execution_lease());
    assert!(matches!(
        fixture
            .interpreter
            .stream_runtime
            .next(&stream)
            .await
            .expect("consume prefill"),
        skiff_runtime_capability_context::StreamPoll::Item(value) if value == json!("prefill")
    ));
    execution.await.expect("pending detached emit completes");
    drop(eval);
    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish pending emit frame");
}

#[derive(Debug)]
struct ProjectingSink {
    pending: Mutex<Option<oneshot::Receiver<()>>>,
    sends: Arc<AtomicUsize>,
    cancellation: StreamSink,
}

impl ProjectingSink {
    fn ready() -> (StreamSink, Arc<AtomicUsize>) {
        Self::new(None)
    }

    fn pending() -> (StreamSink, Arc<AtomicUsize>, oneshot::Sender<()>) {
        let (sender, receiver) = oneshot::channel();
        let (sink, sends) = Self::new(Some(receiver));
        (sink, sends, sender)
    }

    fn new(receiver: Option<oneshot::Receiver<()>>) -> (StreamSink, Arc<AtomicUsize>) {
        let runtime = test_runtime::runtime_factory().stream_runtime();
        let (_, cancellation) = runtime.channel_stream();
        let sends = Arc::new(AtomicUsize::new(0));
        (
            StreamSink::new(Self {
                pending: Mutex::new(receiver),
                sends: Arc::clone(&sends),
                cancellation,
            }),
            sends,
        )
    }
}

impl StreamSinkApi for ProjectingSink {
    fn project_runtime_item(
        &self,
        item: RuntimeValue,
        _source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        Ok(Some(StreamInternalItem::new(item, RequestHeap::default())))
    }

    fn send_internal_with_cancellation<'a>(
        &'a self,
        _item: StreamInternalItem,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        self.sends.fetch_add(1, Ordering::AcqRel);
        let pending = self.pending.lock().expect("pending sink mutex").take();
        Box::pin(async move {
            if let Some(pending) = pending {
                pending
                    .await
                    .map_err(|_| StreamRuntimeError::decode("projected send gate dropped"))?;
            }
            Ok(())
        })
    }

    fn send<'a>(
        &'a self,
        _item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn send_with_cancel<'a>(
        &'a self,
        _item: Value,
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        _item: Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn fail<'a>(
        &'a self,
        _error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn is_cancelled(&self) -> bool {
        false
    }

    fn is_same_stream(&self, _other: &StreamSink) -> bool {
        false
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancellation.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        self.cancellation.cancel_signal()
    }
}

#[tokio::test]
async fn f445h_e4r_spine_emit_projected_ready_keeps_actor_segment() {
    let fixture = emit_fixture("projected-ready");
    let (sink, sends) = ProjectingSink::ready();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("projected Ready emit");

    assert_eq!(sends.load(Ordering::Acquire), 1);
    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish projected Ready frame");
}

#[tokio::test]
async fn f445h_e4r_spine_emit_projected_pending_reacquires_before_completion() {
    let fixture = emit_fixture("projected-pending");
    let (sink, sends, release) = ProjectingSink::pending();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    let mut eval = fixture.eval_context(frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(sends.load(Ordering::Acquire), 1);
    assert!(!frame.has_execution_lease());
    release.send(()).expect("release projected send");
    execution.await.expect("projected Pending emit");
    drop(eval);
    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish projected Pending frame");
}
