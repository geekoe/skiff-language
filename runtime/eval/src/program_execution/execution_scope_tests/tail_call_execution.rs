use std::sync::Arc;

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
