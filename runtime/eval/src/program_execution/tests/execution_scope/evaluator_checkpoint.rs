use crate::heap_access::HeapAccess;
use std::sync::Arc;

use skiff_runtime_linked_program::{
    AssignTargetIr, BinaryOpIr, BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr,
    ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr, PublicationResourceTable,
    RuntimeTypeContext, SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr, UnitAddr,
};
use skiff_runtime_model::request_heap::RequestHeap;
use skiff_runtime_model::runtime_value::RuntimeValue;

use super::*;
use crate::{env::Env, EvalRuntimeProgram, Interpreter};

struct LinkedCheckpointFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    addr: ExecutableAddr,
}

impl LinkedCheckpointFixture {
    fn new(expressions: Vec<LinkedExprIr>, statements: Vec<LinkedStmtIr>) -> Self {
        let entry = BlockIr {
            label: "entry".to_string(),
            statements: (0..statements.len())
                .map(|statement| StmtRefIr {
                    statement: statement as u32,
                })
                .collect(),
        };
        Self::with_body(
            expressions,
            statements,
            vec![entry],
            SlotLayoutIr::default(),
        )
    }

    fn with_body(
        expressions: Vec<LinkedExprIr>,
        statements: Vec<LinkedStmtIr>,
        blocks: Vec<BlockIr>,
        slots: SlotLayoutIr,
    ) -> Self {
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:f445h-e4r-spine-checkpoint".to_string(),
            source_ast_hash: "source:f445h-e4r-spine-checkpoint".to_string(),
            module_path: "f445h.e4r.spine".to_string(),
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
                symbol: "checkpoint".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                self_type: None,
                slots,
                may_suspend: false,
                body: LinkedExecutableBody {
                    blocks,
                    statements,
                    expressions,
                },
            }],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/f445h-e4r-spine",
            vec![Arc::clone(&file)],
            Vec::new(),
            PublicationResourceTable::default(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        let interpreter = Interpreter::with_program(program, test_runtime::runtime_factory());
        let addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file.file_ir_identity.clone()),
            executable: 0,
        };
        Self {
            interpreter,
            file,
            addr,
        }
    }

    async fn execute(
        &self,
        context: ProgramExecutionContext<'static>,
        control: &ScopeAwareControl,
    ) -> Result<crate::env::Flow, RuntimeError> {
        let heap = RequestHeap::default();
        let mut env = Env::for_program_executable(&self.file.executables[0], None, 0)
            .expect("fixture slot layout must be installable");
        let result = self
            .interpreter
            .exec_program_executable(
                context,
                &mut HeapAccess::private(heap),
                &mut env,
                &self.addr,
                &self.file,
                &self.file.executables[0],
            )
            .await;
        assert!(
            control.instruction_units.load(Ordering::Relaxed) > 0,
            "real evaluator must account instruction units before terminating"
        );
        result
    }
}

#[tokio::test]
async fn f445h_e4r_spine_scripted_clock_terminates_pure_cpu_for_loop() {
    let item_count = 16;
    let expressions = (0..item_count)
        .map(|_| LinkedExprIr::Literal {
            value: skiff_artifact_model::LiteralIr::Null,
        })
        .chain(std::iter::once(LinkedExprIr::ArrayLiteral {
            items: (0..item_count)
                .map(|expression| ExprRefIr { expression })
                .collect(),
        }))
        .collect();
    let fixture = LinkedCheckpointFixture::with_body(
        expressions,
        vec![
            LinkedStmtIr::ForIn {
                item_slot: 0,
                item_type: None,
                value_slot: None,
                iterable: ExprRefIr {
                    expression: item_count,
                },
                body: "loop".to_string(),
            },
            LinkedStmtIr::Continue,
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            },
            BlockIr {
                label: "loop".to_string(),
                statements: vec![StmtRefIr { statement: 1 }],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "item".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("local scope");
    let control = ScopeAwareControl::available(scope, cancellation.token());
    let context =
        context(control.clone()).with_execution_clock(ExecutionClock::new(ScriptedClock::new(
            vec![
                base,
                base,
                base,
                base,
                base,
                base + Duration::from_millis(1),
            ],
            Arc::new(AtomicU64::new(0)),
        )));

    let error = fixture
        .execute(context, &control)
        .await
        .expect_err("loop checkpoints must observe the current scope deadline");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
}

#[tokio::test]
async fn while_false_condition_skips_body() {
    let fixture = LinkedCheckpointFixture::with_body(
        vec![
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(1.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Bool { value: false },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(1.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::Add,
                left: ExprRefIr { expression: 3 },
                right: ExprRefIr { expression: 4 },
            },
        ],
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::While {
                condition: ExprRefIr { expression: 1 },
                body: "loop".to_string(),
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 2 }),
            },
            LinkedStmtIr::Assign {
                target: AssignTargetIr::Slot { slot: 0 },
                value: ExprRefIr { expression: 5 },
            },
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![
                    StmtRefIr { statement: 0 },
                    StmtRefIr { statement: 1 },
                    StmtRefIr { statement: 2 },
                ],
            },
            BlockIr {
                label: "loop".to_string(),
                statements: vec![StmtRefIr { statement: 3 }],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "counter".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let flow = fixture
        .execute(context(control.clone()), &control)
        .await
        .expect("false while condition returns without executing the body");
    match flow {
        crate::env::Flow::Return(value) => {
            assert_eq!(value.into_value(), RuntimeValue::Number(1.0));
        }
        other => panic!("expected return after while, got {other:?}"),
    }
}

#[tokio::test]
async fn while_reevaluates_condition_each_iteration() {
    let fixture = LinkedCheckpointFixture::with_body(
        vec![
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(0.0).expect("finite literal"),
                },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(3.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::LessThan,
                left: ExprRefIr { expression: 1 },
                right: ExprRefIr { expression: 2 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(1.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::Add,
                left: ExprRefIr { expression: 4 },
                right: ExprRefIr { expression: 5 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
        ],
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::While {
                condition: ExprRefIr { expression: 3 },
                body: "loop".to_string(),
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 7 }),
            },
            LinkedStmtIr::Assign {
                target: AssignTargetIr::Slot { slot: 0 },
                value: ExprRefIr { expression: 6 },
            },
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![
                    StmtRefIr { statement: 0 },
                    StmtRefIr { statement: 1 },
                    StmtRefIr { statement: 2 },
                ],
            },
            BlockIr {
                label: "loop".to_string(),
                statements: vec![StmtRefIr { statement: 3 }],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "counter".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let flow = fixture
        .execute(context(control.clone()), &control)
        .await
        .expect("bounded while executes");
    match flow {
        crate::env::Flow::Return(value) => {
            assert_eq!(value.into_value(), RuntimeValue::Number(3.0));
        }
        other => panic!("expected return after while, got {other:?}"),
    }
}

#[tokio::test]
async fn while_break_exits_loop_and_execution_continues_after() {
    let fixture = LinkedCheckpointFixture::with_body(
        vec![
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(0.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Bool { value: true },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(1.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::Add,
                left: ExprRefIr { expression: 2 },
                right: ExprRefIr { expression: 3 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(2.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::GreaterThanOrEqual,
                left: ExprRefIr { expression: 5 },
                right: ExprRefIr { expression: 6 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
        ],
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::While {
                condition: ExprRefIr { expression: 1 },
                body: "loop".to_string(),
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 8 }),
            },
            LinkedStmtIr::Assign {
                target: AssignTargetIr::Slot { slot: 0 },
                value: ExprRefIr { expression: 4 },
            },
            LinkedStmtIr::If {
                condition: ExprRefIr { expression: 7 },
                then_block: "exit".to_string(),
                else_block: None,
            },
            LinkedStmtIr::Break,
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![
                    StmtRefIr { statement: 0 },
                    StmtRefIr { statement: 1 },
                    StmtRefIr { statement: 2 },
                ],
            },
            BlockIr {
                label: "loop".to_string(),
                statements: vec![StmtRefIr { statement: 3 }, StmtRefIr { statement: 4 }],
            },
            BlockIr {
                label: "exit".to_string(),
                statements: vec![StmtRefIr { statement: 5 }],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "counter".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let flow = fixture
        .execute(context(control.clone()), &control)
        .await
        .expect("break exits the while loop");
    match flow {
        crate::env::Flow::Return(value) => {
            assert_eq!(value.into_value(), RuntimeValue::Number(2.0));
        }
        other => panic!("expected return after while break, got {other:?}"),
    }
}

#[tokio::test]
async fn while_continue_skips_remaining_body_statements() {
    let fixture = LinkedCheckpointFixture::with_body(
        vec![
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(0.0).expect("finite literal"),
                },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(3.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::LessThan,
                left: ExprRefIr { expression: 1 },
                right: ExprRefIr { expression: 2 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(1.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::Add,
                left: ExprRefIr { expression: 4 },
                right: ExprRefIr { expression: 5 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Number {
                    value: serde_json::Number::from_f64(10.0).expect("finite literal"),
                },
            },
            LinkedExprIr::Binary {
                op: BinaryOpIr::Add,
                left: ExprRefIr { expression: 7 },
                right: ExprRefIr { expression: 8 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
        ],
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::While {
                condition: ExprRefIr { expression: 3 },
                body: "loop".to_string(),
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 10 }),
            },
            LinkedStmtIr::Assign {
                target: AssignTargetIr::Slot { slot: 0 },
                value: ExprRefIr { expression: 6 },
            },
            LinkedStmtIr::Continue,
            LinkedStmtIr::Assign {
                target: AssignTargetIr::Slot { slot: 0 },
                value: ExprRefIr { expression: 9 },
            },
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![
                    StmtRefIr { statement: 0 },
                    StmtRefIr { statement: 1 },
                    StmtRefIr { statement: 2 },
                ],
            },
            BlockIr {
                label: "loop".to_string(),
                statements: vec![
                    StmtRefIr { statement: 3 },
                    StmtRefIr { statement: 4 },
                    StmtRefIr { statement: 5 },
                ],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "counter".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let flow = fixture
        .execute(context(control.clone()), &control)
        .await
        .expect("continue skips the rest of the while body");
    match flow {
        crate::env::Flow::Return(value) => {
            assert_eq!(value.into_value(), RuntimeValue::Number(3.0));
        }
        other => panic!("expected return after while continue, got {other:?}"),
    }
}

#[tokio::test]
async fn while_condition_and_backedge_checkpoints_observe_scope_deadline() {
    let fixture = LinkedCheckpointFixture::with_body(
        vec![LinkedExprIr::Literal {
            value: skiff_artifact_model::LiteralIr::Bool { value: true },
        }],
        vec![
            LinkedStmtIr::While {
                condition: ExprRefIr { expression: 0 },
                body: "loop".to_string(),
            },
            LinkedStmtIr::Continue,
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            },
            BlockIr {
                label: "loop".to_string(),
                statements: vec![StmtRefIr { statement: 1 }],
            },
        ],
        SlotLayoutIr::default(),
    );
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("local scope");
    let control = ScopeAwareControl::available(scope, cancellation.token());
    let context =
        context(control.clone()).with_execution_clock(ExecutionClock::new(ScriptedClock::new(
            vec![
                base,
                base,
                base,
                base,
                base,
                base,
                base,
                base + Duration::from_millis(1),
            ],
            Arc::new(AtomicU64::new(0)),
        )));

    let error = fixture
        .execute(context, &control)
        .await
        .expect_err("while checkpoints must observe the current scope deadline");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
}

#[tokio::test]
async fn f445h_e4r_spine_scripted_clock_terminates_generated_array_chunk() {
    let item_count = 12;
    let expressions = (0..item_count)
        .map(|_| LinkedExprIr::Literal {
            value: skiff_artifact_model::LiteralIr::Null,
        })
        .chain(std::iter::once(LinkedExprIr::ArrayLiteral {
            items: (0..item_count)
                .map(|expression| ExprRefIr { expression })
                .collect(),
        }))
        .collect();
    let fixture = LinkedCheckpointFixture::new(
        expressions,
        vec![LinkedStmtIr::Expr {
            value: ExprRefIr {
                expression: item_count,
            },
        }],
    );
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(1), site())
        .expect("local scope");
    let control = ScopeAwareControl::available(scope, cancellation.token());
    let context =
        context(control.clone()).with_execution_clock(ExecutionClock::new(ScriptedClock::new(
            vec![base, base, base + Duration::from_millis(1)],
            Arc::new(AtomicU64::new(0)),
        )));

    let error = fixture
        .execute(context, &control)
        .await
        .expect_err("generated array checkpoints must observe the current scope deadline");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
}

#[tokio::test]
async fn f445h_e4r_spine_checkpoint_instruction_count_replaces_legacy_accounting() {
    let fixture = LinkedCheckpointFixture::new(
        vec![
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Null,
            },
            LinkedExprIr::Literal {
                value: skiff_artifact_model::LiteralIr::Null,
            },
            LinkedExprIr::ArrayLiteral {
                items: vec![ExprRefIr { expression: 0 }, ExprRefIr { expression: 1 }],
            },
        ],
        vec![LinkedStmtIr::Expr {
            value: ExprRefIr { expression: 2 },
        }],
    );
    let (cancellation, scope) = root_scope(None);
    let control = ScopeAwareControl::available(scope, cancellation.token());
    fixture
        .execute(context(control.clone()), &control)
        .await
        .expect("bounded array executes");
    assert_eq!(
        control.instruction_units.load(Ordering::Relaxed),
        6,
        "executable + block + statement + array + two literal units"
    );
}

#[test]
fn f445h_e4r_spine_shared_test_control_exposes_current_and_derived_scope() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let control = test_runtime::execution_control_with_deadline(Some(deadline));
    let current = control.execution_scope().expect("current request scope");
    assert_eq!(current.nesting(), 0);
    assert_eq!(
        current.effective_deadline().map(|deadline| deadline.at()),
        Some(deadline)
    );
    assert_eq!(
        current
            .effective_deadline()
            .expect("request deadline")
            .source(),
        &ExecutionDeadlineSource::Request,
        "the current fixture scope retains the request as deadline owner"
    );
    assert!(!current.cancellation_signals().is_cancelled());

    let child = control
        .derive_scope(deadline - Duration::from_secs(1), site())
        .expect("derive child scope");
    let child_scope = child.execution_scope().expect("derived current scope");
    assert_eq!(child_scope.nesting(), 1);
    assert_eq!(
        child_scope
            .effective_deadline()
            .map(|deadline| deadline.at()),
        Some(deadline - Duration::from_secs(1))
    );
    assert_eq!(
        child_scope
            .effective_deadline()
            .expect("derived deadline")
            .source(),
        &ExecutionDeadlineSource::Scope { site: site() },
        "the derived fixture scope retains the local timeout site as owner"
    );
    assert!(!child_scope.cancellation_signals().is_cancelled());

    control.cancel_flag().store(true, Ordering::Release);
    assert!(current.cancellation_signals().is_cancelled());
    assert!(
        child_scope.cancellation_signals().is_cancelled(),
        "derived current scope inherits the fixture's request cancellation"
    );
}
