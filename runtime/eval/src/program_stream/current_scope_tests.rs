use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Poll, Wake, Waker},
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, InstructionSourceSite,
    SyntheticInstructionSiteReason, ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::{
    CancellationToken, DbCapabilityContext, ExecutionControl, ExecutionControlApi,
    ExecutionControlResult, ExecutionScope, ExecutionScopeAccessError, FileSourceStreamContext,
    OwnedExecutionControl, OwnedExecutionControlApi, StreamCancelSignal, StreamInternalItem,
    StreamLifetimeGuard, StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi,
    StreamRuntimeError, StreamRuntimeResult, StreamSink, TimeCapabilityContext,
};
use skiff_runtime_linked_program::{
    BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr, ExternalRefTable, FileDeclarations,
    FileLinkTargets, LinkOverlay, LinkedActorDeclaration, LinkedActorDeclarationOwner,
    LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr,
    LinkedTypeRef, PublicationResourceTable, RuntimeTypeContext, ServiceMeta, ServiceSymbolRef,
    SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use super::*;
use crate::{
    actor_executor::ActorExecutionFrame,
    actor_instance::{
        ActorActivationRequest, ActorExecutorAuthority, ActorIncarnationKey, ActorInstanceFence,
        ActorInstanceHandle, ActorInstanceStore, ActorLogicalKey, ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    assembly_execution::ordinary::tests::test_runtime,
    env::{Env, Flow},
    error::RuntimeError,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram,
};

#[derive(Debug)]
struct ScriptedStreamRuntime {
    outcomes: Mutex<VecDeque<StreamRuntimeResult<StreamPoll>>>,
    deferred: Mutex<Option<tokio::sync::oneshot::Receiver<StreamRuntimeResult<StreamPoll>>>>,
    cancellations: Arc<AtomicUsize>,
    cancel_token_count: Arc<AtomicUsize>,
}

impl ScriptedStreamRuntime {
    fn new(
        outcomes: impl IntoIterator<Item = StreamRuntimeResult<StreamPoll>>,
    ) -> (StreamRuntime, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let cancellations = Arc::new(AtomicUsize::new(0));
        let cancel_token_count = Arc::new(AtomicUsize::new(usize::MAX));
        let runtime = StreamRuntime::new(Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            deferred: Mutex::new(None),
            cancellations: Arc::clone(&cancellations),
            cancel_token_count: Arc::clone(&cancel_token_count),
        });
        (runtime, cancellations, cancel_token_count)
    }

    fn gated() -> (
        StreamRuntime,
        tokio::sync::oneshot::Sender<StreamRuntimeResult<StreamPoll>>,
        Arc<AtomicUsize>,
    ) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let cancellations = Arc::new(AtomicUsize::new(0));
        let runtime = StreamRuntime::new(Self {
            outcomes: Mutex::new(VecDeque::new()),
            deferred: Mutex::new(Some(receiver)),
            cancellations: Arc::clone(&cancellations),
            cancel_token_count: Arc::new(AtomicUsize::new(usize::MAX)),
        });
        (runtime, sender, cancellations)
    }

    fn next_result<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        match self
            .outcomes
            .lock()
            .expect("scripted stream mutex poisoned")
            .pop_front()
        {
            Some(outcome) => Box::pin(std::future::ready(outcome)),
            None => match self
                .deferred
                .lock()
                .expect("scripted deferred stream mutex poisoned")
                .take()
            {
                Some(receiver) => Box::pin(async move {
                    receiver.await.unwrap_or_else(|_| {
                        Err(StreamRuntimeError::decode(
                            "scripted deferred stream sender dropped",
                        ))
                    })
                }),
                None => Box::pin(std::future::pending()),
            },
        }
    }
}

impl StreamRuntimeApi for ScriptedStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        unreachable!("the current-scope fixture only consumes an existing stream")
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        unreachable!("the current-scope fixture only consumes an existing stream")
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        unreachable!("the current-scope fixture only consumes an existing stream")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        unreachable!("the current-scope fixture only consumes an existing stream")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.next_result()
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.cancel_token_count
            .store(cancel_tokens.len(), Ordering::Release);
        self.next_result()
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.next_result()
    }

    fn cancel(&self, _value: &Value) {
        self.cancellations.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
struct ScopedControl {
    root: CancellationToken,
    cancelled: Arc<AtomicBool>,
    scope: ExecutionScope,
}

impl ScopedControl {
    fn new(root: CancellationToken, scope: ExecutionScope) -> Self {
        Self {
            cancelled: root.cancel_flag(),
            root,
            scope,
        }
    }
}

impl ExecutionControlApi for ScopedControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.root.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let scope = self
            .scope
            .derive(local_deadline, site)
            .map_err(ExecutionScopeAccessError::Derive)?;
        Ok(OwnedExecutionControl::new(Self::new(
            self.root.clone(),
            scope,
        )))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.root.is_cancelled() {
            Err(skiff_runtime_capability_context::ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        test_runtime::file_source_stream_context(stream_runtime)
    }
}

impl OwnedExecutionControlApi for ScopedControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.root.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

const ACTOR_FILE: &str = "file:f445h-e4r-stream-actor";

struct ActorFrameFixture {
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
}

impl ActorFrameFixture {
    fn new() -> Self {
        let owner = LinkedActorDeclarationOwner {
            unit: UnitAddr::Service,
            file: skiff_runtime_linked_program::FileAddr::FileIrIdentity(ACTOR_FILE.to_string()),
            actor_symbol: "StreamProbe".to_string(),
        };
        let mut file = empty_file(empty_executable());
        file.file_ir_identity = ACTOR_FILE.to_string();
        file.source_ast_hash = "source:f445h-e4r-stream-actor".to_string();
        file.module_path = "actors".to_string();
        file.actor_declarations = vec![LinkedActorDeclaration {
            actor_type: ServiceSymbolRef {
                module_path: "actors".to_string(),
                symbol: "StreamProbe".to_string(),
            },
            implementation_owner: Some(owner.clone()),
            actor_abi_identity: ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:f445h-e4r-stream"),
            actor_implementation_identity: ActorImplementationIdentity::new(
                "skiff-actor-implementation-v1:sha256:f445h-e4r-stream",
            ),
            actor_name: "StreamProbe".to_string(),
            actor_id_type: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
            fields: Vec::new(),
            public_methods: Vec::new(),
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }];
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/f445h-e4r-stream",
            vec![Arc::new(file)],
            Vec::new(),
            Vec::new(),
            PublicationResourceTable::default(),
            Vec::new(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(ActorActivationRequest {
                fence: ActorInstanceFence {
                    incarnation: ActorIncarnationKey {
                        logical_key: ActorLogicalKey {
                            service_id: "skiff.run/f445h-e4r-stream".to_string(),
                            actor_type_identity: "actors.StreamProbe".to_string(),
                            actor_id_type_identity: "builtin:string".to_string(),
                            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                            actor_id_hash:
                                "sha256:a9d57d9dc2127eaf51681c67636a67bfd14056cf6f4ee552f48d3a8c5a306420"
                                    .to_string(),
                            canonical_actor_id_key_bytes: br#""activation-probe""#.to_vec(),
                        },
                        epoch: 1,
                    },
                    actor_abi_identity: ActorAbiIdentity::new(
                        "skiff-actor-abi-v1:sha256:f445h-e4r-stream",
                    ),
                    actor_implementation_identity: ActorImplementationIdentity::new(
                        "skiff-actor-implementation-v1:sha256:f445h-e4r-stream",
                    ),
                    declaration_owner: owner,
                },
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: b"{}",
                program: program.projection().type_view(),
            })
            .expect("activate fieldless stream Actor");
        Self { store, handle }
    }

    async fn frame(&self) -> (ActorExecutionFrame, RequestHeap) {
        let authority = ActorExecutorAuthority::new();
        let mut lease = self
            .store
            .acquire_execution(&authority, &self.handle)
            .await
            .expect("acquire stream Actor");
        let heap = lease.take_heap();
        (
            ActorExecutionFrame::new(self.store.clone(), self.handle.clone(), lease, Vec::new()),
            heap,
        )
    }
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn empty_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "f445h.e4r.stream.currentScope".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: true,
        body: LinkedExecutableBody::default(),
    }
}

fn body_executable(
    statements: Vec<LinkedStmtIr>,
    expressions: Vec<LinkedExprIr>,
) -> LinkedExecutable {
    LinkedExecutable {
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "body".to_string(),
                statements: (0..statements.len())
                    .map(|statement| StmtRefIr {
                        statement: statement as u32,
                    })
                    .collect(),
            }],
            statements,
            expressions,
        },
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "item".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
        ..empty_executable()
    }
}

fn empty_file(executable: LinkedExecutable) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:f445h-e4r-stream".to_string(),
        source_ast_hash: "source:f445h-e4r-stream".to_string(),
        module_path: "f445h.e4r.stream".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![executable],
        external_refs: ExternalRefTable::default(),
    }
}

fn interpreter_with_file(file: Arc<LinkedFileUnit>) -> Interpreter {
    let program = Arc::new(EvalRuntimeProgram::new(
        "skiff.run/f445h-e4r-stream",
        vec![file],
        Vec::new(),
        Vec::new(),
        PublicationResourceTable::default(),
        Vec::new(),
        Default::default(),
        LinkOverlay::default(),
        RuntimeTypeContext::default(),
    ));
    Interpreter::with_program(program, test_runtime::runtime_factory())
}

fn scoped_context<'a>(
    interpreter: &Interpreter,
    stream_runtime: StreamRuntime,
    execution: ExecutionControl<'a>,
) -> ProgramExecutionContext<'a> {
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: FileSourceStreamContext::new(stream_runtime.clone(), execution.clone()),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            stream_runtime,
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: "skiff.run/f445h-e4r-stream".to_string(),
                display_name: None,
                metadata: Default::default(),
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

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

#[test]
fn f445h_e4r_stream_for_in_materializes_current_local_deadline_owner_before_wait() {
    let executable = empty_executable();
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (stream_runtime, cancellations, cancel_token_count) =
        ScriptedStreamRuntime::new(Vec::new());
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("deadline before now");
    let root = test_runtime::execution_control();
    let owned = root
        .derive_scope(deadline, site())
        .expect("derive current local scope");
    let current_scope = owned.execution_scope().expect("current local scope");
    let context = scoped_context(&interpreter, stream_runtime, owned.borrow());
    let addr = ExecutableAddr::service(0, 0);
    let mut heap = RequestHeap::default();
    let mut env = Env::new();
    let stream_value = json!({"$stream": "f445h-e4r-pending"});
    let future = interpreter.exec_program_stream_for_in(
        context,
        &mut heap,
        &mut env,
        &addr,
        &file,
        &executable,
        0,
        "body",
        stream_value,
        None,
        &[],
    );
    let mut future = Box::pin(future);
    let waker = Waker::from(Arc::new(NoopWake));
    let mut poll_context = std::task::Context::from_waker(&waker);

    let Poll::Ready(Err(RuntimeError::ScopeTerminal(carrier))) =
        future.as_mut().poll(&mut poll_context)
    else {
        panic!("current local deadline must terminate before entering a pending stream wait");
    };
    assert_eq!(
        carrier.effective_deadline().at(),
        deadline,
        "the stream wait must preserve the exact current deadline owner"
    );
    assert!(carrier.is_owned_by(&current_scope));
    drop(future);
    assert_eq!(
        cancellations.load(Ordering::Acquire),
        1,
        "non-End terminal cleanup must initiate locally exactly once"
    );
    assert_eq!(
        cancel_token_count.load(Ordering::Acquire),
        usize::MAX,
        "an already-terminal scope must not enter the stream runtime"
    );
}

#[tokio::test]
async fn f445h_e4r_stream_for_in_natural_end_is_the_only_disarmed_terminal() {
    let executable = empty_executable();
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, cancel_token_count) =
        ScriptedStreamRuntime::new([Ok(StreamPoll::End)]);
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control());
    let result = interpreter
        .exec_program_stream_for_in(
            context,
            &mut RequestHeap::default(),
            &mut Env::new(),
            &ExecutableAddr::service(0, 0),
            &file,
            &executable,
            0,
            "body",
            json!({"$stream": "natural-end"}),
            None,
            &[],
        )
        .await
        .expect("natural End succeeds");
    assert!(matches!(result, Flow::Continue));
    assert_eq!(cancellations.load(Ordering::Acquire), 0);
    assert_eq!(
        cancel_token_count.load(Ordering::Acquire),
        0,
        "current scope is raced directly rather than reduced to a request token"
    );
}

#[tokio::test]
async fn f445h_e4r_stream_for_in_break_initiates_local_cleanup_once() {
    let executable = body_executable(vec![LinkedStmtIr::Break], Vec::new());
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, _) =
        ScriptedStreamRuntime::new([Ok(StreamPoll::Item(json!("item")))]);
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control());
    let result = interpreter
        .exec_program_stream_for_in(
            context,
            &mut RequestHeap::default(),
            &mut Env::for_program_executable(&executable, None, 0).expect("loop env"),
            &ExecutableAddr::service(0, 0),
            &file,
            &executable,
            0,
            "body",
            json!({"$stream": "break"}),
            None,
            &[],
        )
        .await
        .expect("break exits the consumer");
    assert!(matches!(result, Flow::Continue));
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn f445h_e4r_stream_for_in_return_initiates_local_cleanup_once() {
    let executable = body_executable(vec![LinkedStmtIr::Return { value: None }], Vec::new());
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, _) =
        ScriptedStreamRuntime::new([Ok(StreamPoll::Item(json!("item")))]);
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control());
    let result = interpreter
        .exec_program_stream_for_in(
            context,
            &mut RequestHeap::default(),
            &mut Env::for_program_executable(&executable, None, 0).expect("loop env"),
            &ExecutableAddr::service(0, 0),
            &file,
            &executable,
            0,
            "body",
            json!({"$stream": "return"}),
            None,
            &[],
        )
        .await
        .expect("return exits the consumer");
    assert!(matches!(result, Flow::Return(_)));
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn f445h_e4r_stream_for_in_ordinary_error_initiates_local_cleanup_once() {
    let executable = body_executable(
        vec![LinkedStmtIr::Expr {
            value: ExprRefIr { expression: 99 },
        }],
        Vec::new(),
    );
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, _) =
        ScriptedStreamRuntime::new([Ok(StreamPoll::Item(json!("item")))]);
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control());
    let error = interpreter
        .exec_program_stream_for_in(
            context,
            &mut RequestHeap::default(),
            &mut Env::for_program_executable(&executable, None, 0).expect("loop env"),
            &ExecutableAddr::service(0, 0),
            &file,
            &executable,
            0,
            "body",
            json!({"$stream": "ordinary-error"}),
            None,
            &[],
        )
        .await
        .expect_err("invalid body expression is an ordinary evaluator error");
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
}

#[test]
fn f445h_e4r_stream_for_in_future_drop_initiates_cleanup_without_remote_ack() {
    let executable = empty_executable();
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, late_item, cancellations) = ScriptedStreamRuntime::gated();
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control());
    let mut heap = RequestHeap::default();
    let mut env = Env::new();
    let addr = ExecutableAddr::service(0, 0);
    let checkpoint = heap.checkpoint();
    let stats = heap.stats();
    let future = interpreter.exec_program_stream_for_in(
        context,
        &mut heap,
        &mut env,
        &addr,
        &file,
        &executable,
        0,
        "body",
        json!({"$stream": "future-drop"}),
        None,
        &[],
    );
    let mut future = Box::pin(future);
    let waker = Waker::from(Arc::new(NoopWake));
    let mut poll_context = std::task::Context::from_waker(&waker);
    assert!(future.as_mut().poll(&mut poll_context).is_pending());
    drop(future);
    assert_eq!(
        cancellations.load(Ordering::Acquire),
        1,
        "dropping the caller future initiates cleanup synchronously without awaiting an ack"
    );
    let mut late_heap = RequestHeap::default();
    let late_handle = late_heap
        .alloc_array(vec![RuntimeValue::String("late".to_string())])
        .expect("allocate late provider graph");
    assert!(
        late_item
            .send(Ok(StreamPoll::InternalItem(StreamInternalItem::new(
                RuntimeValue::Heap(late_handle),
                late_heap,
            ))))
            .is_err(),
        "a late item and its heap must lose the dropped caller wait"
    );
    assert_eq!(heap.checkpoint(), checkpoint);
    assert_eq!(heap.stats(), stats);

    let (runtime, late_error, error_cancellations) = ScriptedStreamRuntime::gated();
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control());
    let mut error_heap = RequestHeap::default();
    let mut error_env = Env::new();
    let future = interpreter.exec_program_stream_for_in(
        context,
        &mut error_heap,
        &mut error_env,
        &addr,
        &file,
        &executable,
        0,
        "body",
        json!({"$stream": "future-drop-late-error"}),
        None,
        &[],
    );
    let mut future = Box::pin(future);
    assert!(future.as_mut().poll(&mut poll_context).is_pending());
    drop(future);
    assert_eq!(error_cancellations.load(Ordering::Acquire), 1);
    assert!(
        late_error
            .send(Err(StreamRuntimeError::decode("late producer error")))
            .is_err(),
        "a late error must lose the dropped caller wait"
    );
}

#[test]
fn f445h_e4r_stream_for_in_ancestor_cancel_wins_equal_expired_deadline() {
    let executable = empty_executable();
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, _) = ScriptedStreamRuntime::new(Vec::new());
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("expired deadline");
    let root = test_runtime::execution_control();
    let owned = root
        .derive_scope(deadline, site())
        .expect("derive equal terminal fixture");
    root.cancellation_token().cancel();
    let context = scoped_context(&interpreter, runtime, owned.borrow());
    let mut heap = RequestHeap::default();
    let mut env = Env::new();
    let addr = ExecutableAddr::service(0, 0);
    let future = interpreter.exec_program_stream_for_in(
        context,
        &mut heap,
        &mut env,
        &addr,
        &file,
        &executable,
        0,
        "body",
        json!({"$stream": "cancel-deadline-race"}),
        None,
        &[],
    );
    let mut future = Box::pin(future);
    let waker = Waker::from(Arc::new(NoopWake));
    let mut poll_context = std::task::Context::from_waker(&waker);
    assert!(matches!(
        future.as_mut().poll(&mut poll_context),
        Poll::Ready(Err(RuntimeError::Cancelled))
    ));
    drop(future);
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn f445h_e4r_stream_for_in_buffered_ready_keeps_actor_segment() {
    let actor = ActorFrameFixture::new();
    let (frame, mut heap) = actor.frame().await;
    let authority = ActorExecutorAuthority::new();
    let mut competitor = Box::pin(actor.store.acquire_execution(&authority, &actor.handle));
    let waker = Waker::from(Arc::new(NoopWake));
    let mut poll_context = std::task::Context::from_waker(&waker);
    assert!(competitor.as_mut().poll(&mut poll_context).is_pending());

    let executable = body_executable(vec![LinkedStmtIr::Break], Vec::new());
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, cancel_token_count) =
        ScriptedStreamRuntime::new([Ok(StreamPoll::Item(json!("buffered")))]);
    let context = scoped_context(&interpreter, runtime, test_runtime::execution_control())
        .with_actor_execution_frame(frame.clone());
    let flow = interpreter
        .exec_program_stream_for_in(
            context,
            &mut heap,
            &mut Env::for_program_executable(&executable, None, 0).expect("loop env"),
            &ExecutableAddr::service(0, 0),
            &file,
            &executable,
            0,
            "body",
            json!({"$stream": "actor-buffered-ready"}),
            None,
            &[],
        )
        .await
        .expect("buffered stream item and break complete synchronously");
    assert!(matches!(flow, Flow::Continue));
    assert!(
        competitor.as_mut().poll(&mut poll_context).is_pending(),
        "a first-poll Ready stream next must keep the current Actor segment"
    );
    assert_eq!(cancel_token_count.load(Ordering::Acquire), 0);
    assert_eq!(
        cancellations.load(Ordering::Acquire),
        1,
        "break remains a non-End cleanup path"
    );
    drop(competitor);
    frame.finish(heap).expect("finish stream Actor segment");
}

#[test]
fn f445h_e4r_stream_for_in_buffered_ready_then_pending_observes_lease_child_scope() {
    let executable = body_executable(vec![LinkedStmtIr::Continue], Vec::new());
    let file = Arc::new(empty_file(executable.clone()));
    let interpreter = interpreter_with_file(Arc::clone(&file));
    let (runtime, cancellations, cancel_token_count) =
        ScriptedStreamRuntime::new([Ok(StreamPoll::Item(json!("buffered")))]);
    let root = CancellationToken::new();
    let parent = ExecutionScope::request(root.clone(), None);
    let (lease, _completion) = parent.acquire_lease();
    let execution = ExecutionControl::new(ScopedControl::new(root, lease.child_execution_scope()));
    let context = scoped_context(&interpreter, runtime, execution);
    let mut heap = RequestHeap::default();
    let mut env = Env::for_program_executable(&executable, None, 0).expect("loop env");
    let addr = ExecutableAddr::service(0, 0);
    let future = interpreter.exec_program_stream_for_in(
        context,
        &mut heap,
        &mut env,
        &addr,
        &file,
        &executable,
        0,
        "body",
        json!({"$stream": "ready-then-pending"}),
        None,
        &[],
    );
    let mut future = Box::pin(future);
    let waker = Waker::from(Arc::new(NoopWake));
    let mut poll_context = std::task::Context::from_waker(&waker);
    assert!(
        future.as_mut().poll(&mut poll_context).is_pending(),
        "the buffered item is consumed synchronously before the second real Pending wait"
    );
    assert_eq!(cancel_token_count.load(Ordering::Acquire), 0);
    drop(lease);
    assert!(matches!(
        future.as_mut().poll(&mut poll_context),
        Poll::Ready(Err(RuntimeError::Cancelled))
    ));
    drop(future);
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
    assert_eq!(
        parent.lifecycle_snapshot(),
        Default::default(),
        "the lease-child terminal leaves no waiter, timer, or lease state"
    );
}
