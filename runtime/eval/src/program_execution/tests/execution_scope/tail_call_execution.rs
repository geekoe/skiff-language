use std::sync::Arc;

use skiff_artifact_model::{SourcePosition, SourceSpanRef};
use skiff_runtime_capability_context::{
    ExecutionControl as CapabilityExecutionControl, OwnedExecutionControlApi,
};
use skiff_runtime_linked_program::{
    BinaryOpIr, BlockIr, CallIr, ExecutableAddr, ExecutableKind, ExprRefIr, ExternalRefTable,
    FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedCallTarget, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr, LinkedTypeRef, ParamIr,
    PublicationResourceTable, RuntimeTypeContext, SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr,
    UnitAddr,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::*;
use crate::{
    env::{Env, Flow},
    error::Result,
    EvalRuntimeProgram, Interpreter,
};

struct TailCallFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    caller: ExecutableAddr,
}

struct TailPressureFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    entry: ExecutableAddr,
}

struct TailEntryCheckpointFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    entry: ExecutableAddr,
    previous_site: InstructionSourceSite,
    current_site: InstructionSourceSite,
}

#[derive(Clone)]
struct ScheduledBudgetControl {
    scope: ExecutionScope,
    cancellation: CancellationToken,
    cancelled: Arc<AtomicBool>,
    instruction_units: Arc<AtomicU64>,
    polls: Arc<AtomicU64>,
    fail_on_poll: u64,
}

impl ScheduledBudgetControl {
    fn new(scope: ExecutionScope, cancellation: CancellationToken, fail_on_poll: u64) -> Self {
        Self {
            scope,
            cancelled: cancellation.cancel_flag(),
            cancellation,
            instruction_units: Arc::new(AtomicU64::new(0)),
            polls: Arc::new(AtomicU64::new(0)),
            fail_on_poll,
        }
    }
}

impl ExecutionControlApi for ScheduledBudgetControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
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
        let scope = self.scope.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl::new(Self {
            scope,
            cancellation: self.cancellation.clone(),
            cancelled: Arc::clone(&self.cancelled),
            instruction_units: Arc::clone(&self.instruction_units),
            polls: Arc::clone(&self.polls),
            fail_on_poll: self.fail_on_poll,
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.instruction_units.fetch_add(units, Ordering::Relaxed);
        Ok(())
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        let poll = self.polls.fetch_add(1, Ordering::Relaxed) + 1;
        if poll == self.fail_on_poll {
            Err(ExecutionControlError::BudgetExceeded(
                ExecutionBudgetFailure {
                    reason: ExecutionBudgetReason::InstructionLimitExceeded,
                    instruction_count: self.instruction_units.load(Ordering::Relaxed),
                    limit: Some(self.instruction_units.load(Ordering::Relaxed) - 1),
                    elapsed_ms: 1.0,
                },
            ))
        } else {
            Ok(())
        }
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        test_runtime::file_source_stream_context(stream_runtime)
    }
}

impl OwnedExecutionControlApi for ScheduledBudgetControl {
    fn borrow(&self) -> CapabilityExecutionControl<'_> {
        CapabilityExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        ExecutionControlApi::execution_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

impl TailEntryCheckpointFixture {
    fn new() -> Self {
        let file_id = "file:e1-tail-entry-checkpoint".to_string();
        let entry = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file_id.clone()),
            executable: 0,
        };
        let middle = ExecutableAddr {
            executable: 1,
            ..entry.clone()
        };
        let terminal = ExecutableAddr {
            executable: 2,
            ..entry.clone()
        };
        let previous_site = source_site(201);
        let current_site = source_site(202);
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: file_id,
            source_ast_hash: "source:e1-tail-entry-checkpoint".to_string(),
            module_path: "e1.tailEntryCheckpoint".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![
                tail_forwarder("entry", middle, previous_site.clone()),
                tail_forwarder("middle", terminal, current_site.clone()),
                executable(
                    "terminal",
                    vec![LinkedExprIr::Literal {
                        value: skiff_artifact_model::LiteralIr::Null,
                    }],
                ),
            ],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/e1-tail-entry-checkpoint",
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
            entry,
            previous_site,
            current_site,
        }
    }

    async fn execute(&self, context: ProgramExecutionContext<'static>) -> Result<crate::env::Flow> {
        let mut heap = RequestHeap::default();
        let mut env = Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )?;
        self.interpreter
            .exec_program_executable(
                context,
                &mut heap,
                &mut env,
                &self.entry,
                &self.file,
                &self.file.executables[0],
            )
            .await
    }
}

impl TailPressureFixture {
    fn new() -> Self {
        let file_id = "file:v1-tail-pressure".to_string();
        let entry = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file_id.clone()),
            executable: 0,
        };
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: file_id,
            source_ast_hash: "source:v1-tail-pressure".to_string(),
            module_path: "v1.tailPressure".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![tail_countdown_executable(entry.clone())],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/v1-tail-pressure",
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
            entry,
        }
    }

    async fn execute(self, context: ProgramExecutionContext<'static>, hops: u64) -> Result<Flow> {
        let mut heap = RequestHeap::default();
        let executable = &self.file.executables[0];
        let mut env =
            Env::for_program_executable(executable, Some(self.file.module_path.clone()), 0)?;
        env.declare_program_parameter(executable, "remaining", RuntimeValue::Number(hops as f64))?;
        env.declare_program_parameter(executable, "accumulator", RuntimeValue::Number(0.0))?;
        self.interpreter
            .exec_program_executable(
                context,
                &mut heap,
                &mut env,
                &self.entry,
                &self.file,
                executable,
            )
            .await
    }
}

impl TailCallFixture {
    fn new() -> Self {
        Self::with_equivalent_return_plans(true)
    }

    fn with_equivalent_return_plans(equivalent: bool) -> Self {
        let file_id = "file:r1-tail-call-execution".to_string();
        let caller = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file_id.clone()),
            executable: 0,
        };
        let callee = ExecutableAddr {
            executable: 1,
            ..caller.clone()
        };
        let site = site();
        let mut caller_executable = executable(
            "caller",
            vec![LinkedExprIr::Call {
                call: CallIr {
                    target: LinkedCallTarget::Executable {
                        addr: callee.clone(),
                    },
                    site,
                    args: Vec::new(),
                    type_args: Default::default(),
                    metadata: Default::default(),
                    actor_metadata: None,
                },
            }],
        );
        if !equivalent {
            caller_executable.return_type = Some(LinkedTypeRef::Native {
                name: "Json".to_string(),
                args: Vec::new(),
            });
        }
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: file_id,
            source_ast_hash: "source:r1-tail-call-execution".to_string(),
            module_path: "r1.tailCall".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![
                caller_executable,
                executable(
                    "callee",
                    vec![LinkedExprIr::Literal {
                        value: skiff_artifact_model::LiteralIr::Null,
                    }],
                ),
            ],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/r1-tail-call-execution",
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
            caller,
        }
    }

    async fn execute_callable(
        &self,
        context: ProgramExecutionContext<'static>,
    ) -> Result<crate::env::Flow> {
        let mut heap = RequestHeap::default();
        let mut env = Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )?;
        self.interpreter
            .exec_program_executable(
                context,
                &mut heap,
                &mut env,
                &self.caller,
                &self.file,
                &self.file.executables[0],
            )
            .await
    }

    async fn execute_barrier(
        &self,
        context: ProgramExecutionContext<'static>,
    ) -> Result<crate::env::Flow> {
        let mut heap = RequestHeap::default();
        let mut env = Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )?;
        self.interpreter
            .exec_program_block(
                context,
                &mut heap,
                &mut env,
                &self.caller,
                &self.file,
                &self.file.executables[0],
                "entry",
            )
            .await
    }
}

fn executable(symbol: &str, expressions: Vec<LinkedExprIr>) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions,
        },
    }
}

fn tail_forwarder(
    symbol: &str,
    target: ExecutableAddr,
    call_site: InstructionSourceSite,
) -> LinkedExecutable {
    executable(
        symbol,
        vec![LinkedExprIr::Call {
            call: CallIr {
                target: LinkedCallTarget::Executable { addr: target },
                site: call_site,
                args: Vec::new(),
                type_args: Default::default(),
                metadata: Default::default(),
                actor_metadata: None,
            },
        }],
    )
}

fn source_site(line: u32) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 901,
            start: SourcePosition::new(line, 3),
            end: SourcePosition::new(line, 19),
        },
    }
}

fn tail_countdown_executable(target: ExecutableAddr) -> LinkedExecutable {
    let remaining = ExprRefIr { expression: 0 };
    let zero = ExprRefIr { expression: 1 };
    let condition = ExprRefIr { expression: 2 };
    let accumulator = ExprRefIr { expression: 3 };
    let next_remaining = ExprRefIr { expression: 6 };
    let next_accumulator = ExprRefIr { expression: 8 };
    let call = ExprRefIr { expression: 9 };
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "countdown".to_string(),
        type_params: Vec::new(),
        params: vec![
            ParamIr {
                name: "remaining".to_string(),
                slot: 0,
                ty: LinkedTypeRef::Native {
                    name: "Json".to_string(),
                    args: Vec::new(),
                },
            },
            ParamIr {
                name: "accumulator".to_string(),
                slot: 1,
                ty: LinkedTypeRef::Native {
                    name: "Json".to_string(),
                    args: Vec::new(),
                },
            },
        ],
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "remaining".to_string(),
                    kind: "param".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "accumulator".to_string(),
                    kind: "param".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: LinkedExecutableBody {
            blocks: vec![
                BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }],
                },
                BlockIr {
                    label: "done".to_string(),
                    statements: vec![StmtRefIr { statement: 1 }],
                },
                BlockIr {
                    label: "recurse".to_string(),
                    statements: vec![StmtRefIr { statement: 2 }],
                },
            ],
            statements: vec![
                LinkedStmtIr::If {
                    condition,
                    then_block: "done".to_string(),
                    else_block: Some("recurse".to_string()),
                },
                LinkedStmtIr::Return {
                    value: Some(accumulator),
                },
                LinkedStmtIr::Return { value: Some(call) },
            ],
            expressions: vec![
                LinkedExprIr::LoadSlot { slot: 0 },
                number(0),
                LinkedExprIr::Binary {
                    op: BinaryOpIr::LessThanOrEqual,
                    left: remaining,
                    right: zero,
                },
                LinkedExprIr::LoadSlot { slot: 1 },
                LinkedExprIr::LoadSlot { slot: 0 },
                number(1),
                LinkedExprIr::Binary {
                    op: BinaryOpIr::Subtract,
                    left: ExprRefIr { expression: 4 },
                    right: ExprRefIr { expression: 5 },
                },
                LinkedExprIr::LoadSlot { slot: 1 },
                LinkedExprIr::Binary {
                    op: BinaryOpIr::Add,
                    left: ExprRefIr { expression: 7 },
                    right: ExprRefIr { expression: 5 },
                },
                LinkedExprIr::Call {
                    call: CallIr {
                        target: LinkedCallTarget::Executable { addr: target },
                        site: site(),
                        args: vec![next_remaining, next_accumulator],
                        type_args: Default::default(),
                        metadata: Default::default(),
                        actor_metadata: None,
                    },
                },
            ],
        },
    }
}

fn number(value: i64) -> LinkedExprIr {
    LinkedExprIr::Literal {
        value: skiff_artifact_model::LiteralIr::Number {
            value: serde_json::Number::from(value),
        },
    }
}

fn available_context() -> ProgramExecutionContext<'static> {
    let (cancellation, scope) = root_scope(None);
    context(ScopeAwareControl::available(scope, cancellation.token()))
}

fn counted_context() -> (ProgramExecutionContext<'static>, Arc<AtomicU64>) {
    let (cancellation, scope) = root_scope(None);
    let control = ScopeAwareControl::available(scope, cancellation.token());
    let instruction_units = Arc::clone(&control.instruction_units);
    (context(control), instruction_units)
}

fn max_depth_context() -> ProgramExecutionContext<'static> {
    let (cancellation, scope) = root_scope(None);
    context(ScopeAwareControl::available(scope, cancellation.token()))
        .with_program_call_depth_for_test(super::super::MAX_PROGRAM_CALL_DEPTH)
}

fn scheduled_budget_context(control: ScheduledBudgetControl) -> ProgramExecutionContext<'static> {
    let execution = CapabilityExecutionControl::new(control);
    let runtime_factory = test_runtime::runtime_factory();
    let stream_runtime = runtime_factory.stream_runtime();
    let test_effect_doubles =
        runtime_factory.reusable_test_effect_doubles(HashMap::new(), &stream_runtime, false);
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context_with_trace("trace:e1-entry-checkpoint");
    let request = test_runtime::request_context_with_trace("trace:e1-entry-checkpoint");
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(stream_runtime.clone()),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            HttpRuntimeOptions::explicit(false),
            stream_runtime,
            test_effect_doubles.clone(),
        ),
        test_effect_doubles,
        actor: actor.clone(),
        request,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

#[tokio::test]
async fn tail_call_entry_checkpoint_attributes_the_current_edge_exactly_once() {
    const FAIL_ON_POLL: u64 = 11;

    let fixture = TailEntryCheckpointFixture::new();
    let (cancellation, scope) = root_scope(None);
    let control = ScheduledBudgetControl::new(scope, cancellation.token(), FAIL_ON_POLL);
    let instruction_units = Arc::clone(&control.instruction_units);
    let polls = Arc::clone(&control.polls);
    let error = fixture
        .execute(scheduled_budget_context(control))
        .await
        .expect_err("the terminal entry checkpoint must cross the scheduled budget");

    assert_eq!(
        polls.load(Ordering::Relaxed),
        FAIL_ON_POLL,
        "both entry preparations and tail transfer polls must succeed before the terminal entry checkpoint fails"
    );
    assert_eq!(
        instruction_units.load(Ordering::Relaxed),
        FAIL_ON_POLL,
        "the failed entry checkpoint must account its unit after both transfer units"
    );
    let request = crate::exceptions::user_exception_for_catch(&error)
        .expect("entry checkpoint budget failure should materialize a request exception")
        .request();
    assert_eq!(request.source(), &fixture.current_site);
    assert_eq!(
        request.stack(),
        [
            skiff_runtime_model::service_error::ExceptionStackFrame::Local {
                site: fixture.current_site.clone(),
            }
        ],
        "the current tail edge must be promoted exactly once"
    );
    assert_ne!(request.source(), &fixture.previous_site);
}

#[tokio::test]
async fn tail_call_shared_trampoline_does_not_push_non_tail_depth() {
    let fixture = TailCallFixture::new();
    let flow = fixture
        .execute_callable(max_depth_context())
        .await
        .expect("tail transfer must replace the active frame");
    assert!(matches!(flow, crate::env::Flow::Return(_)));
}

#[tokio::test]
async fn tail_call_lexical_barrier_falls_back_to_an_ordinary_call() {
    let fixture = TailCallFixture::new();
    assert!(
        fixture.execute_barrier(max_depth_context()).await.is_err(),
        "barrier evaluation must retain the ordinary nested-call depth push"
    );
}

#[tokio::test]
async fn tail_call_transfer_accounts_like_the_corresponding_ordinary_call() {
    let tail_fixture = TailCallFixture::new();
    let (tail_context, tail_units) = counted_context();
    tail_fixture
        .execute_callable(tail_context)
        .await
        .expect("eligible tail call should complete");

    let ordinary_fixture = TailCallFixture::with_equivalent_return_plans(false);
    let (ordinary_context, ordinary_units) = counted_context();
    ordinary_fixture
        .execute_callable(ordinary_context)
        .await
        .expect("return-plan mismatch should execute as an ordinary call");

    assert_eq!(
        tail_units.load(Ordering::Relaxed),
        ordinary_units.load(Ordering::Relaxed),
        "an eliminated tail transfer must preserve ordinary per-call instruction accounting"
    );
}

#[test]
fn tail_call_completes_100000_hops_on_one_mib_tokio_worker() {
    const HOPS: u64 = 100_000;
    let fixture = TailPressureFixture::new();
    let context = available_context();
    let worker = std::thread::Builder::new()
        .name("tail-pressure-one-mib".to_string())
        .stack_size(1024 * 1024)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("one-worker Tokio runtime should build");
            runtime.block_on(async {
                let flow = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::spawn(async move { fixture.execute(context, HOPS).await }),
                )
                .await
                .expect("100,000 tail hops should finish promptly")
                .expect("1 MiB worker task must not panic")
                .expect("100,000 tail hops should execute");
                let Flow::Return(value) = flow else {
                    panic!("tail pressure fixture must return");
                };
                assert_eq!(value.into_value(), RuntimeValue::Number(HOPS as f64));
            });
        })
        .expect("1 MiB Tokio worker should spawn");
    worker
        .join()
        .expect("1 MiB Tokio worker must not overflow or panic");
}
