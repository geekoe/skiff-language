use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Wake, Waker},
};

use serde_json::Value;
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::{
    CancellationToken, StreamCancelSignal, StreamInternalItem, StreamRuntimeError,
    StreamRuntimeResult, StreamSink, StreamSinkApi,
};
use skiff_runtime_linked_program::{
    BlockIr, ExecutableAddr, ExecutableKind, ExternalRefTable, FileAddr, FileDeclarations,
    FileLinkTargets, LinkOverlay, LinkedConcurrentLaneIr, LinkedConcurrentPlanIr, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr, LiteralIr,
    PublicationResourceTable, RuntimeTypeContext, ServiceMeta, SlotIr, SlotLayoutIr, SourceMapDto,
    StmtRefIr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
};
use tokio::sync::oneshot;

use super::*;
use crate::{
    actor_executor_test_runtime as test_runtime,
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram,
};

const FILE_ID: &str = "file:f445h-e4r-concurrent";

struct EvaluatorFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    addr: ExecutableAddr,
}

impl EvaluatorFixture {
    fn new(body: LinkedExecutableBody, slots: SlotLayoutIr) -> Self {
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: FILE_ID.to_string(),
            source_ast_hash: "source:f445h-e4r-concurrent".to_string(),
            module_path: "concurrent".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![LinkedExecutable {
                kind: ExecutableKind::Function,
                symbol: "concurrentEvaluator".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                self_type: None,
                slots,
                may_suspend: true,
                body,
            }],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/f445h-e4r-concurrent",
            vec![Arc::clone(&file)],
            Vec::new(),
            PublicationResourceTable::default(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        Self {
            interpreter: Interpreter::with_program(program, test_runtime::runtime_factory()),
            file,
            addr: ExecutableAddr {
                unit: UnitAddr::Service,
                file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
                executable: 0,
            },
        }
    }

    fn env(&self) -> Env {
        Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )
        .expect("concurrent evaluator env")
    }

    fn eval<'a>(
        &'a self,
        context: ProgramExecutionContext<'static>,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context,
            heap,
            env,
            &self.addr,
            &self.file,
            &self.file.executables[0],
        )
        .expect("concurrent evaluator context")
    }
}

fn program_context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(
            interpreter.stream_runtime.clone(),
        ),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: "skiff.run/f445h-e4r-concurrent".to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: "1.0.0".to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: Default::default(),
        }),
        actor: actor.clone(),
        spawn: actor,
        outbound: test_runtime::outbound_context(),
        request_heap_limits: RequestHeapLimits::default(),
    })
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
            .expect("gate senders lock")
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
        self.state.events.lock().expect("gate events lock").clone()
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

    fn id(item: StreamInternalItem) -> StreamRuntimeResult<i64> {
        let (value, _) = item.into_parts();
        match value {
            RuntimeValue::Number(value) => Ok(value as i64),
            other => Err(StreamRuntimeError::decode(format!(
                "gate sink expected a numeric lane id, got {other:?}"
            ))),
        }
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
                .expect("gate events lock")
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
        let id = match Self::id(item) {
            Ok(id) => id,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let receiver = self
            .state
            .receivers
            .lock()
            .expect("gate receivers lock")
            .get_mut(&id)
            .and_then(VecDeque::pop_front);
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        self.state
            .events
            .lock()
            .expect("gate events lock")
            .push(GateEvent::Start(id));
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut active = ActiveSend::new(Arc::clone(&state), id);
            let result = match receiver {
                Some(receiver) => receiver
                    .await
                    .map_err(|_| StreamRuntimeError::decode("lane gate sender dropped"))?
                    .map_err(StreamRuntimeError::decode),
                None => Ok(()),
            };
            active.completed = true;
            state
                .events
                .lock()
                .expect("gate events lock")
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

fn fixture(
    blocks: Vec<BlockIr>,
    statements: Vec<LinkedStmtIr>,
    expressions: Vec<LinkedExprIr>,
    slots: SlotLayoutIr,
) -> EvaluatorFixture {
    EvaluatorFixture::new(
        LinkedExecutableBody {
            blocks,
            statements,
            expressions,
        },
        slots,
    )
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

fn statement_plan(lanes: Vec<LinkedConcurrentLaneIr>) -> LinkedConcurrentPlanIr {
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

fn tail_lane(source_order: u32, dependencies: &[u32], expression: u32) -> LinkedConcurrentLaneIr {
    LinkedConcurrentLaneIr::Tail {
        source_order,
        dependencies: dependencies.to_vec(),
        tail: ExprRefIr { expression },
        site: site(),
    }
}

fn emit(expression: u32) -> LinkedStmtIr {
    LinkedStmtIr::Emit {
        operation: "emit".to_string(),
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

fn assert_number(carrier: RuntimeValueCarrier, expected: f64) {
    assert_eq!(carrier.into_value(), RuntimeValue::Number(expected));
}

#[tokio::test]
async fn f445h_e4r_concurrent_statement_root_executes_direct_lanes_with_pending_overlap() {
    let plan = statement_plan(vec![
        statement_lane(0, &[], "lane-0"),
        statement_lane(1, &[], "lane-1"),
    ]);
    let fixture = fixture(
        vec![
            block("entry", &[0]),
            block("lane-0", &[1]),
            block("lane-1", &[2]),
        ],
        vec![LinkedStmtIr::Concurrent { plan }, emit(0), emit(1)],
        vec![number(10), number(20)],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[10, 20]);
    let mut heap = RequestHeap::default();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let context = program_context(&fixture.interpreter);
    let mut eval = fixture.eval(context, &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 2);
    assert_eq!(probe.max_active(), 2);
    assert_eq!(
        &probe.events()[..2],
        &[GateEvent::Start(10), GateEvent::Start(20)]
    );

    assert!(probe.release(10, Ok(())));
    assert!(probe.release(20, Ok(())));
    let flow = execution.await.expect("concurrent statement completes");
    assert!(matches!(flow, Flow::Continue));
    assert_eq!(
        probe.events(),
        vec![
            GateEvent::Start(10),
            GateEvent::Start(20),
            GateEvent::Complete(10),
            GateEvent::Complete(20),
        ]
    );
}

#[tokio::test]
async fn f445h_e4r_concurrent_serial_dependency_gates_and_runs_the_complete_block() {
    let plan = statement_plan(vec![
        statement_lane(0, &[], "lane-0"),
        serial_lane(1, &[0], "serial-1"),
    ]);
    let fixture = fixture(
        vec![
            block("entry", &[0]),
            block("lane-0", &[1]),
            block("serial-1", &[2, 3]),
        ],
        vec![LinkedStmtIr::Concurrent { plan }, emit(0), emit(1), emit(2)],
        vec![number(10), number(20), number(21)],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[10, 20, 21]);
    let mut heap = RequestHeap::default();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(program_context(&fixture.interpreter), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 1, "dependent serial lane must not start");
    assert!(probe.release(10, Ok(())));
    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 2, "serial block starts after dependency");
    assert!(probe.release(20, Ok(())));
    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(
        probe.starts(),
        3,
        "serial block executes its second statement"
    );
    assert!(probe.release(21, Ok(())));

    assert!(matches!(
        execution.await.expect("serial dependency completes"),
        Flow::Continue
    ));
    assert_eq!(
        probe.events(),
        vec![
            GateEvent::Start(10),
            GateEvent::Complete(10),
            GateEvent::Start(20),
            GateEvent::Complete(20),
            GateEvent::Start(21),
            GateEvent::Complete(21),
        ]
    );
}

#[tokio::test]
async fn f445h_e4r_concurrent_value_tail_waits_for_fence_and_hands_heap_value_to_parent() {
    let plan = statement_plan(vec![
        statement_lane(0, &[], "lane-0"),
        tail_lane(1, &[0], 2),
    ]);
    let fixture = fixture(
        vec![block("entry", &[0]), block("lane-0", &[1])],
        vec![
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 3 }),
            },
            emit(0),
        ],
        vec![
            number(10),
            LinkedExprIr::Literal {
                value: LiteralIr::String {
                    value: "tail".to_string(),
                },
            },
            LinkedExprIr::ArrayLiteral {
                items: vec![ExprRefIr { expression: 1 }],
            },
            LinkedExprIr::ConcurrentValue { plan },
        ],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[10]);
    let mut heap = RequestHeap::default();
    let before = heap.stats();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(program_context(&fixture.interpreter), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 1);
    assert!(probe.release(10, Ok(())));
    let Flow::Return(value) = execution.await.expect("value concurrent completes") else {
        panic!("value concurrent must return its tail carrier");
    };
    drop(eval);

    let items = runtime_array_item_carriers(&value, &heap)
        .expect("tail array lookup")
        .expect("parent heap owns tail array");
    assert_eq!(items[0].value(), &RuntimeValue::String("tail".to_string()));
    assert!(heap.stats().node_count > before.node_count);
    assert_eq!(
        probe.events(),
        vec![GateEvent::Start(10), GateEvent::Complete(10)]
    );
}

#[tokio::test]
async fn f445h_e4r_concurrent_same_turn_errors_choose_source_order() {
    let plan = statement_plan(vec![
        statement_lane(0, &[], "lane-0"),
        statement_lane(1, &[], "lane-1"),
    ]);
    let fixture = fixture(
        vec![
            block("entry", &[0]),
            block("lane-0", &[1]),
            block("lane-1", &[2]),
        ],
        vec![LinkedStmtIr::Concurrent { plan }, emit(0), emit(1)],
        vec![number(10), number(20)],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[10, 20]);
    let mut heap = RequestHeap::default();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(program_context(&fixture.interpreter), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(probe.release(20, Err("lane-1-error")));
    assert!(probe.release(10, Err("lane-0-error")));
    let error = execution.await.expect_err("both lanes fail in one turn");

    assert!(error.to_string().contains("lane-0-error"));
    assert!(!error.to_string().contains("lane-1-error"));
    assert_eq!(
        &probe.events()[..2],
        &[GateEvent::Start(10), GateEvent::Start(20)]
    );
}

#[tokio::test]
async fn f445h_e4r_concurrent_outer_terminal_wins_over_same_turn_lane_completion() {
    let plan = statement_plan(vec![statement_lane(0, &[], "lane-0")]);
    let fixture = fixture(
        vec![block("entry", &[0]), block("lane-0", &[1])],
        vec![LinkedStmtIr::Concurrent { plan }, emit(0)],
        vec![number(10)],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[10]);
    let mut heap = RequestHeap::default();
    let before = heap.stats();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let context = program_context(&fixture.interpreter);
    let cancellation = context.execution().cancellation_token();
    let mut eval = fixture.eval(context, &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(probe.release(10, Ok(())));
    cancellation.cancel();
    let error = execution
        .await
        .expect_err("outer cancellation must beat lane completion");

    assert!(error.to_string().to_ascii_lowercase().contains("cancel"));
    assert_eq!(eval.heap.stats(), before);
    assert_eq!(
        probe.events(),
        vec![GateEvent::Start(10), GateEvent::Complete(10)]
    );
}

#[tokio::test]
async fn f445h_e4r_concurrent_winner_stops_unstarted_lane_and_drops_running_loser() {
    let plan = statement_plan(vec![
        statement_lane(0, &[], "winner"),
        statement_lane(1, &[], "running-loser"),
        statement_lane(2, &[1], "unstarted"),
    ]);
    let fixture = fixture(
        vec![
            block("entry", &[0]),
            block("winner", &[1]),
            block("running-loser", &[2]),
            block("unstarted", &[3]),
        ],
        vec![LinkedStmtIr::Concurrent { plan }, emit(0), emit(1), emit(2)],
        vec![number(10), number(20), number(30)],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[10, 20, 30]);
    let mut heap = RequestHeap::default();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    let mut eval = fixture.eval(program_context(&fixture.interpreter), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(probe.starts(), 2);
    assert!(probe.release(10, Err("winner-error")));
    let error = execution.await.expect_err("winner terminates scheduler");

    assert!(error.to_string().contains("winner-error"));
    assert_eq!(probe.starts(), 2, "dependent lane must remain unstarted");
    assert_eq!(probe.drops(), 1, "running loser future must be dropped");
    assert!(!probe.release(20, Ok(())), "loser receiver is already gone");
    assert!(probe.events().contains(&GateEvent::Dropped(20)));
    assert!(!probe.events().contains(&GateEvent::Start(30)));
}

#[tokio::test]
async fn f445h_e4r_concurrent_loser_late_heap_write_isolated_and_outer_scope_restored() {
    let plan = statement_plan(vec![
        statement_lane(0, &[], "winner"),
        serial_lane(1, &[], "late-loser"),
    ]);
    let fixture = fixture(
        vec![
            block("entry", &[0]),
            block("winner", &[1]),
            block("late-loser", &[2, 3]),
        ],
        vec![
            LinkedStmtIr::Concurrent { plan },
            emit(0),
            emit(1),
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 3 },
            },
        ],
        vec![
            number(10),
            number(20),
            number(99),
            LinkedExprIr::ArrayLiteral {
                items: vec![ExprRefIr { expression: 2 }],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "outer".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let (sink, probe) = GateSink::gated(&[10, 20]);
    let mut heap = RequestHeap::default();
    let before = heap.stats();
    let mut env = fixture.env();
    env.stream_sink = Some(sink);
    env.declare_binding("outer", Some(0), RuntimeValue::Number(7.0))
        .expect("outer binding");
    let context = program_context(&fixture.interpreter);
    let outer_scope = context.execution().execution_scope().expect("outer scope");
    let outer_lifecycle = outer_scope.lifecycle_snapshot();
    let mut eval = fixture.eval(context.clone(), &mut heap, &mut env);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(probe.release(10, Err("winner-before-late-write")));
    execution.await.expect_err("winner rejects late loser");
    drop(eval);

    assert!(!probe.release(20, Ok(())), "late result cannot be accepted");
    assert_number(env.get_slot(0).expect("outer slot"), 7.0);
    assert_eq!(heap.stats(), before, "lane-local heap write cannot leak");
    assert_eq!(outer_scope.nesting(), 0);
    assert_eq!(
        context
            .execution()
            .execution_scope()
            .expect("restored scope")
            .lifecycle_snapshot(),
        outer_lifecycle
    );
    assert!(probe.events().contains(&GateEvent::Dropped(20)));
}

#[tokio::test]
async fn f445h_e4r_concurrent_malformed_and_noncontinue_lanes_fail_closed_without_fallback() {
    let malformed = statement_plan(vec![statement_lane(0, &[], "malformed")]);
    let malformed_fixture = fixture(
        vec![block("entry", &[0]), block("malformed", &[1, 2])],
        vec![
            LinkedStmtIr::Concurrent { plan: malformed },
            emit(0),
            emit(1),
        ],
        vec![number(10), number(20)],
        SlotLayoutIr::default(),
    );
    let (sink, probe) = GateSink::gated(&[]);
    let mut heap = RequestHeap::default();
    let mut env = malformed_fixture.env();
    env.stream_sink = Some(sink);
    let error = malformed_fixture
        .eval(
            program_context(&malformed_fixture.interpreter),
            &mut heap,
            &mut env,
        )
        .exec_program_executable()
        .await
        .expect_err("malformed linked plan");
    assert!(error.to_string().contains("exactly one direct statement"));
    assert_eq!(
        probe.starts(),
        0,
        "malformed plan must not run sequentially"
    );

    let noncontinue = statement_plan(vec![statement_lane(0, &[], "returning")]);
    let noncontinue_fixture = fixture(
        vec![block("entry", &[0]), block("returning", &[1])],
        vec![
            LinkedStmtIr::Concurrent { plan: noncontinue },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            },
        ],
        vec![number(42)],
        SlotLayoutIr::default(),
    );
    let mut heap = RequestHeap::default();
    let mut env = noncontinue_fixture.env();
    let error = noncontinue_fixture
        .eval(
            program_context(&noncontinue_fixture.interpreter),
            &mut heap,
            &mut env,
        )
        .exec_program_executable()
        .await
        .expect_err("return flow cannot escape a concurrent lane");
    assert!(error.to_string().contains("forbidden return flow"));
}
