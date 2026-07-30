use std::{
    collections::{HashMap, VecDeque},
    sync::{atomic::AtomicBool, Mutex},
    task::{Wake, Waker},
};

use serde_json::Value;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationToken, StreamCancelSignal, StreamInternalItem, StreamRuntimeError,
    StreamRuntimeResult, StreamSink, StreamSinkApi,
};
use skiff_runtime_linked_program::{
    BinaryOpIr, LinkedConcurrentLaneIr, LinkedConcurrentPlanIr, LiteralIr,
};

use super::*;
use crate::{
    env::{Env, Flow},
    eval_context::EvalContext,
};

struct ActorEvaluatorFixture {
    actor: Fixture,
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    addr: ExecutableAddr,
}

impl ActorEvaluatorFixture {
    fn new(
        blocks: Vec<BlockIr>,
        statements: Vec<LinkedStmtIr>,
        expressions: Vec<LinkedExprIr>,
    ) -> Self {
        let actor = fixture(integer(), true);
        let mut file = (*actor_file(integer(), true)).clone();
        let executable = &mut file.executables[0];
        executable.params = Vec::new();
        executable.return_type = None;
        executable.slots = SlotLayoutIr::default();
        executable.body = LinkedExecutableBody {
            blocks,
            statements,
            expressions,
        };
        let file = Arc::new(file);
        let (interpreter, _) = interpreter_for(Arc::clone(&file));
        Self {
            actor,
            interpreter,
            file,
            addr: executable_addr(),
        }
    }

    fn env(&self) -> Env {
        Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )
        .expect("Actor concurrent env")
    }

    fn eval<'a>(
        &'a self,
        frame: ActorExecutionFrame,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context(&self.interpreter).with_actor_execution_frame(frame),
            heap,
            env,
            &self.addr,
            &self.file,
            &self.file.executables[0],
        )
        .expect("Actor concurrent evaluator")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GateEvent {
    Start(i64),
    Complete(i64),
    Dropped(i64),
}

type GateResult = std::result::Result<(), &'static str>;

#[derive(Debug)]
struct GateState {
    receivers: Mutex<HashMap<i64, VecDeque<oneshot::Receiver<GateResult>>>>,
    senders: Mutex<HashMap<i64, VecDeque<oneshot::Sender<GateResult>>>>,
    events: Mutex<Vec<GateEvent>>,
    starts: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    drops: AtomicUsize,
    cancellation: StreamSink,
}

#[derive(Clone)]
struct GateProbe {
    state: Arc<GateState>,
}

impl GateProbe {
    fn release(&self, id: i64, result: GateResult) -> bool {
        self.state
            .senders
            .lock()
            .expect("Actor gate senders lock")
            .get_mut(&id)
            .and_then(VecDeque::pop_front)
            .is_some_and(|sender| sender.send(result).is_ok())
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    fn max_active(&self) -> usize {
        self.state.max_active.load(Ordering::Acquire)
    }

    fn drops(&self) -> usize {
        self.state.drops.load(Ordering::Acquire)
    }

    fn events(&self) -> Vec<GateEvent> {
        self.state
            .events
            .lock()
            .expect("Actor gate events lock")
            .clone()
    }
}

#[derive(Debug)]
struct GateSink {
    state: Arc<GateState>,
}

impl GateSink {
    fn gated(ids: &[i64]) -> (StreamSink, GateProbe) {
        let mut receivers = HashMap::<i64, VecDeque<_>>::new();
        let mut senders = HashMap::<i64, VecDeque<_>>::new();
        for id in ids {
            let (sender, receiver) = oneshot::channel();
            receivers.entry(*id).or_default().push_back(receiver);
            senders.entry(*id).or_default().push_back(sender);
        }
        let runtime = test_runtime::runtime_factory().stream_runtime();
        let (_, cancellation) = runtime.channel_stream();
        let state = Arc::new(GateState {
            receivers: Mutex::new(receivers),
            senders: Mutex::new(senders),
            events: Mutex::new(Vec::new()),
            starts: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            drops: AtomicUsize::new(0),
            cancellation,
        });
        (
            StreamSink::new(Self {
                state: Arc::clone(&state),
            }),
            GateProbe { state },
        )
    }
}

struct ActiveSend {
    state: Arc<GateState>,
    id: i64,
    completed: bool,
}

impl ActiveSend {
    fn new(state: Arc<GateState>, id: i64) -> Self {
        let active = state.active.fetch_add(1, Ordering::AcqRel) + 1;
        state.max_active.fetch_max(active, Ordering::AcqRel);
        Self {
            state,
            id,
            completed: false,
        }
    }
}

impl Drop for ActiveSend {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::AcqRel);
        if !self.completed {
            self.state.drops.fetch_add(1, Ordering::AcqRel);
            self.state
                .events
                .lock()
                .expect("Actor gate events lock")
                .push(GateEvent::Dropped(self.id));
        }
    }
}

impl StreamSinkApi for GateSink {
    fn project_runtime_item(
        &self,
        item: RuntimeValue,
        _source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        Ok(Some(StreamInternalItem::new(item, RequestHeap::default())))
    }

    fn send_internal_with_cancellation<'a>(
        &'a self,
        item: StreamInternalItem,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        let (value, _) = item.into_parts();
        let RuntimeValue::Number(value) = value else {
            return Box::pin(async {
                Err(StreamRuntimeError::decode(
                    "Actor gate expected a numeric lane id",
                ))
            });
        };
        let id = value as i64;
        let receiver = self
            .state
            .receivers
            .lock()
            .expect("Actor gate receivers lock")
            .get_mut(&id)
            .and_then(VecDeque::pop_front);
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        self.state
            .events
            .lock()
            .expect("Actor gate events lock")
            .push(GateEvent::Start(id));
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut active = ActiveSend::new(Arc::clone(&state), id);
            let result = match receiver {
                Some(receiver) => receiver
                    .await
                    .map_err(|_| StreamRuntimeError::decode("Actor lane gate sender dropped"))?
                    .map_err(StreamRuntimeError::decode),
                None => Ok(()),
            };
            active.completed = true;
            state
                .events
                .lock()
                .expect("Actor gate events lock")
                .push(GateEvent::Complete(id));
            result
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
        self.state.cancellation.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        self.state.cancellation.cancel_signal()
    }
}

fn block(label: &str, statements: &[u32]) -> BlockIr {
    BlockIr {
        label: label.to_string(),
        statements: statements
            .iter()
            .map(|statement| StmtRefIr {
                statement: *statement,
            })
            .collect(),
    }
}

fn plan(lanes: Vec<LinkedConcurrentLaneIr>) -> LinkedConcurrentPlanIr {
    LinkedConcurrentPlanIr {
        lanes,
        site: site(),
    }
}

fn statement_lane(source_order: u32, dependencies: &[u32], body: &str) -> LinkedConcurrentLaneIr {
    LinkedConcurrentLaneIr::Statement {
        source_order,
        dependencies: dependencies.to_vec(),
        body: body.to_string(),
        site: site(),
    }
}

fn serial_lane(source_order: u32, dependencies: &[u32], body: &str) -> LinkedConcurrentLaneIr {
    LinkedConcurrentLaneIr::Serial {
        source_order,
        dependencies: dependencies.to_vec(),
        body: body.to_string(),
        site: site(),
    }
}

fn emit(expression: u32) -> LinkedStmtIr {
    LinkedStmtIr::Emit {
        operation: "emit".to_string(),
        value: ExprRefIr { expression },
    }
}

fn assign_count(expression: u32) -> LinkedStmtIr {
    LinkedStmtIr::Assign {
        target: AssignTargetIr::ActorSelfField {
            field: "count".to_string(),
            field_type: integer(),
        },
        value: ExprRefIr { expression },
    }
}

fn number(value: i64) -> LinkedExprIr {
    LinkedExprIr::Literal {
        value: LiteralIr::Number {
            value: serde_json::Number::from(value),
        },
    }
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

#[tokio::test]
async fn f445h_e4r_concurrent_actor_ready_lanes_keep_serial_segments_and_restore_parent() {
    let concurrent = plan(vec![
        serial_lane(0, &[], "lane-0"),
        serial_lane(1, &[], "lane-1"),
    ]);
    let fixture = ActorEvaluatorFixture::new(
        vec![
            block("entry", &[0]),
            block("lane-0", &[1, 2]),
            block("lane-1", &[3, 4, 5]),
        ],
        vec![
            LinkedStmtIr::Concurrent { plan: concurrent },
            emit(0),
            assign_count(1),
            emit(2),
            LinkedStmtIr::Assert {
                condition: ExprRefIr { expression: 5 },
                message: None,
            },
            assign_count(6),
        ],
        vec![
            number(10),
            number(5),
            number(20),
            LinkedExprIr::ActorSelfField {
                field: "count".to_string(),
                field_type: integer(),
            },
            number(5),
            LinkedExprIr::Binary {
                op: BinaryOpIr::Equal,
                left: ExprRefIr { expression: 3 },
                right: ExprRefIr { expression: 4 },
            },
            number(6),
        ],
    );
    let (sink, probe) = GateSink::gated(&[]);
    let (frame, mut heap) = execution_frame(&fixture.actor).await;
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(frame.clone(), &mut heap, &mut env);

    let flow = eval
        .exec_program_executable()
        .await
        .expect("Actor Ready lanes complete");
    drop(eval);

    assert!(matches!(flow, Flow::Continue));
    assert_eq!(
        probe.events(),
        vec![
            GateEvent::Start(10),
            GateEvent::Complete(10),
            GateEvent::Start(20),
            GateEvent::Complete(20),
        ]
    );
    assert_eq!(probe.max_active(), 1);
    assert!(
        frame.has_execution_lease(),
        "parent frame must be restored after all Ready children close"
    );
    assert_eq!(
        frame.read_field("count").expect("restored Actor field"),
        RuntimeValue::Number(6.0),
        "second synchronous segment observes the first committed segment"
    );
    frame.finish(heap).expect("finish restored parent");
}

#[tokio::test]
async fn f445h_e4r_concurrent_actor_pending_lanes_overlap_with_independent_frames_and_parent_restore(
) {
    let concurrent = plan(vec![
        serial_lane(0, &[], "lane-0"),
        serial_lane(1, &[], "lane-1"),
    ]);
    let fixture = ActorEvaluatorFixture::new(
        vec![
            block("entry", &[0]),
            block("lane-0", &[1, 2]),
            block("lane-1", &[3, 4]),
        ],
        vec![
            LinkedStmtIr::Concurrent { plan: concurrent },
            emit(0),
            assign_count(1),
            emit(2),
            assign_count(3),
        ],
        vec![number(10), number(5), number(20), number(6)],
    );
    let (sink, probe) = GateSink::gated(&[10, 20]);
    let (frame, mut heap) = execution_frame(&fixture.actor).await;
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(frame.clone(), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 2);
    assert_eq!(
        probe.max_active(),
        2,
        "actual Pending releases each synchronous segment so waits overlap"
    );
    assert!(
        !frame.has_execution_lease(),
        "outer parent stays suspended while children are open"
    );

    assert!(probe.release(20, Ok(())));
    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(
        !frame.has_execution_lease(),
        "one remaining child keeps the parent gate closed"
    );
    assert!(probe.release(10, Ok(())));
    assert!(matches!(
        execution.await.expect("Actor Pending lanes complete"),
        Flow::Continue
    ));
    drop(eval);

    assert_eq!(
        probe.events(),
        vec![
            GateEvent::Start(10),
            GateEvent::Start(20),
            GateEvent::Complete(20),
            GateEvent::Complete(10),
        ]
    );
    assert!(frame.has_execution_lease());
    assert_eq!(
        frame.read_field("count").expect("restored Actor field"),
        RuntimeValue::Number(5.0),
        "each lane resumes its own frame and commits in gate completion order"
    );
    frame.finish(heap).expect("finish restored parent");
}

#[tokio::test]
async fn f445h_e4r_concurrent_actor_error_abandons_running_and_unstarted_children_without_lease_leak(
) {
    let concurrent = plan(vec![
        statement_lane(0, &[], "winner"),
        statement_lane(1, &[], "running-loser"),
        statement_lane(2, &[1], "unstarted"),
    ]);
    let fixture = ActorEvaluatorFixture::new(
        vec![
            block("entry", &[0]),
            block("winner", &[1]),
            block("running-loser", &[2]),
            block("unstarted", &[3]),
        ],
        vec![
            LinkedStmtIr::Concurrent { plan: concurrent },
            emit(0),
            emit(1),
            emit(2),
        ],
        vec![number(10), number(20), number(30)],
    );
    let (sink, probe) = GateSink::gated(&[10, 20, 30]);
    let (frame, mut heap) = execution_frame(&fixture.actor).await;
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(frame.clone(), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 2);
    assert!(probe.release(10, Err("Actor-winner-error")));
    let error = execution.await.expect_err("Actor winner terminates lanes");
    drop(eval);

    assert!(error.to_string().contains("Actor-winner-error"));
    assert_eq!(probe.starts(), 2);
    assert_eq!(probe.drops(), 1);
    assert!(!probe.events().contains(&GateEvent::Start(30)));
    assert!(!probe.release(20, Ok(())));
    assert!(
        frame.has_execution_lease(),
        "error path closes every child before restoring the parent"
    );
    frame.finish(heap).expect("finish restored error parent");

    assert_eq!(
        execute(&fixture.actor, &fixture.actor.method, b"[9]")
            .await
            .expect("next Actor call acquires without leaked child lease"),
        b"9"
    );
}
