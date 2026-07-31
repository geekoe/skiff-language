use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

use serde_json::json;
use skiff_artifact_model::{
    LiteralIr, SourcePosition, SourceSpanRef, SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_program::{
    anonymous_type_decl, BlockIr, CallIr, ExecutableAddr, ExecutableKind, ExprRefIr,
    ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedCallTarget,
    LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr,
    LinkedTypeDescriptor, LinkedTypeRef, NativeTarget, PackageSymbolKey, PublicationResourceTable,
    RuntimeTypeContext, SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr, TypeAddr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueCarrier},
    service_error::{ExceptionStackFrame, PlatformBuiltinErrorIdentity, RequestException},
};

use super::*;
use crate::{
    capabilities::{HttpRuntimeOptions, TimeCapabilityContext},
    env::{Env, Flow},
    error::unwrap_diagnostic_source_context,
    exceptions::user_exception_for_catch,
    runtime_ops::runtime_to_wire,
    EvalRuntimeProgram, Interpreter,
};

const TRACE_ID: &str = "trace-f445h-e4r-timeout";
const TIMEOUT_FILE_ID: &str = "file:f445h-e4r-timeout";

struct LinkedTimeoutFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    addr: ExecutableAddr,
}

struct TimeoutRun {
    result: Result<Flow, RuntimeError>,
    heap: RequestHeap,
}

impl LinkedTimeoutFixture {
    fn new(expressions: Vec<LinkedExprIr>, statements: Vec<LinkedStmtIr>) -> Self {
        Self::with_body(
            expressions,
            statements,
            vec![BlockIr {
                label: "entry".to_string(),
                statements: Vec::new(),
            }],
            SlotLayoutIr::default(),
        )
    }

    fn with_body(
        expressions: Vec<LinkedExprIr>,
        statements: Vec<LinkedStmtIr>,
        blocks: Vec<BlockIr>,
        slots: SlotLayoutIr,
    ) -> Self {
        Self::with_body_and_additional_executables(
            expressions,
            statements,
            blocks,
            slots,
            Vec::new(),
        )
    }

    fn with_body_and_additional_executables(
        expressions: Vec<LinkedExprIr>,
        statements: Vec<LinkedStmtIr>,
        mut blocks: Vec<BlockIr>,
        slots: SlotLayoutIr,
        mut additional_executables: Vec<LinkedExecutable>,
    ) -> Self {
        let entry = blocks
            .iter_mut()
            .find(|block| block.label == "entry")
            .expect("timeout fixture needs an entry block");
        if entry.statements.is_empty() {
            entry.statements = (0..statements.len())
                .map(|statement| StmtRefIr {
                    statement: statement as u32,
                })
                .collect();
        }
        let mut executables = vec![LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "timeout".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots,
            may_suspend: true,
            body: LinkedExecutableBody {
                blocks,
                statements,
                expressions,
            },
        }];
        executables.append(&mut additional_executables);
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: TIMEOUT_FILE_ID.to_string(),
            source_ast_hash: "source:f445h-e4r-timeout".to_string(),
            module_path: "f445h.e4r.timeout".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables,
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/f445h-e4r-timeout",
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

    async fn execute(&self, context: ProgramExecutionContext<'static>) -> TimeoutRun {
        let mut heap = RequestHeap::default();
        let mut env = Env::for_program_executable(&self.file.executables[0], None, 0)
            .expect("timeout fixture slot layout");
        let result = self
            .interpreter
            .exec_program_executable(
                context,
                &mut heap,
                &mut env,
                &self.addr,
                &self.file,
                &self.file.executables[0],
            )
            .await;
        TimeoutRun { result, heap }
    }

    fn with_std_duration(mut self) -> Self {
        let duration = anonymous_type_decl(
            "std.time.Duration",
            LinkedTypeDescriptor::Alias {
                target: LinkedTypeRef::Native {
                    name: "integer".to_string(),
                    args: Vec::new(),
                },
            },
        );
        let std_file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:f445h-e4r-timeout-std".to_string(),
            source_ast_hash: "source:f445h-e4r-timeout-std".to_string(),
            module_path: "std".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: vec![duration.clone()],
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        });
        let mut overlay = LinkOverlay::default();
        overlay
            .package_slots_by_id
            .insert("skiff.run/std".to_string(), 0);
        overlay
            .package_slots_by_dependency_ref
            .insert("std".to_string(), 0);
        let addr = TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        };
        let mut types = RuntimeTypeContext::default();
        types.descriptors.insert(addr.clone(), duration);
        types
            .exported_types
            .insert_package(PackageSymbolKey::new(0, "std.time.Duration"), addr);
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/f445h-e4r-timeout",
            vec![Arc::clone(&self.file)],
            vec![crate::test_support::runtime_execution_package_fixture(
                "skiff.run/std",
                0,
                vec![std_file],
                PublicationResourceTable::default(),
            )],
            PublicationResourceTable::default(),
            Default::default(),
            overlay,
            types,
        ));
        self.interpreter = Interpreter::with_program(program, test_runtime::runtime_factory());
        self
    }
}

fn traced_context(control: ScopeAwareControl) -> ProgramExecutionContext<'static> {
    let execution = ExecutionControl::new(control);
    let runtime_factory = test_runtime::runtime_factory();
    let stream_runtime = runtime_factory.stream_runtime();
    let test_effect_doubles =
        runtime_factory.reusable_test_effect_doubles(HashMap::new(), &stream_runtime, false);
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context_with_trace(TRACE_ID);
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
        spawn: actor,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

fn context_with_clock(
    control: ScopeAwareControl,
    clock: impl EvalMonotonicClock + 'static,
) -> ProgramExecutionContext<'static> {
    traced_context(control).with_execution_clock(ExecutionClock::new(clock))
}

fn fixed_clock(base: Instant) -> ScriptedClock {
    ScriptedClock::new(vec![base], Arc::new(AtomicU64::new(0)))
}

fn crossing_clock(base: Instant, deadline: Instant, crossing_call: usize) -> ScriptedClock {
    let mut values = vec![base; crossing_call.saturating_sub(1)];
    values.push(deadline);
    ScriptedClock::new(values, Arc::new(AtomicU64::new(0)))
}

fn timeout_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "TimeoutError".to_string(),
        args: Vec::new(),
    }
}

fn source_site(line: u32) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 445,
            start: SourcePosition::new(line, 2),
            end: SourcePosition::new(line, 12),
        },
    }
}

fn null_expr() -> LinkedExprIr {
    LinkedExprIr::Literal {
        value: LiteralIr::Null,
    }
}

fn string_expr(value: &str) -> LinkedExprIr {
    LinkedExprIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn return_statement(expression: u32) -> LinkedStmtIr {
    LinkedStmtIr::Return {
        value: Some(ExprRefIr { expression }),
    }
}

fn returned_value(flow: &Flow) -> &RuntimeValueCarrier {
    let Flow::Return(value) = flow else {
        panic!("timeout fixture must return a value, got {flow:?}");
    };
    value
}

fn caught_exception<'a>(flow: &Flow, heap: &'a RequestHeap) -> &'a RequestException {
    let RuntimeValue::Heap(caught) = returned_value(flow).value() else {
        panic!("ordinary catch result must be an object");
    };
    let HeapNode::Object(caught) = heap.get(*caught).expect("ordinary catch result") else {
        panic!("ordinary catch result must stay an object");
    };
    assert_eq!(
        caught.fields().get("tag"),
        Some(&RuntimeValue::String("err".to_string()))
    );
    let RuntimeValue::Heap(exception) = caught
        .fields()
        .get("exception")
        .expect("ordinary catch retains exception")
    else {
        panic!("ordinary catch exception must be heap-owned");
    };
    let HeapNode::Exception(exception) = heap.get(*exception).expect("caught exception") else {
        panic!("ordinary catch must retain RequestException");
    };
    exception
}

fn uncaught_exception(error: &RuntimeError) -> &RequestException {
    user_exception_for_catch(error)
        .unwrap_or_else(|| {
            panic!("timeout owner must materialize a request-local exception, got {error:?}")
        })
        .request()
}

fn assert_timeout_exception(
    exception: &RequestException,
    heap: &RequestHeap,
    wrapper_site: &InstructionSourceSite,
    deadline_site: &InstructionSourceSite,
    nesting: u32,
) {
    assert_eq!(
        exception.local_catch_identity(),
        Some(&PlatformBuiltinErrorIdentity::Timeout.catch_identity())
    );
    assert_eq!(exception.source(), wrapper_site);
    assert_eq!(
        exception.stack(),
        &[ExceptionStackFrame::Local {
            site: wrapper_site.clone(),
        }]
    );
    assert_eq!(exception.correlation().trace_id, TRACE_ID);
    assert_eq!(
        exception.correlation().error_id,
        format!("{TRACE_ID}:local-error:1")
    );
    let payload = exception
        .local_value()
        .expect("local timeout exception must retain its payload");
    assert_eq!(
        payload.catch_identity(),
        Some(&PlatformBuiltinErrorIdentity::Timeout.catch_identity())
    );
    assert_eq!(
        runtime_to_wire(payload.value(), heap).expect("timeout payload remains JSON-compatible"),
        json!({
            "reason": "deadlineExceeded",
            "deadlineSource": "scope",
            "deadlineNesting": nesting,
            "deadlineSite": serde_json::to_value(deadline_site)
                .expect("deadline site is serializable"),
        })
    );
}

fn assert_parent_restored(control: &ScopeAwareControl) {
    let scope =
        ExecutionControlApi::execution_scope(control).expect("parent scope remains available");
    assert_eq!(scope.nesting(), 0);
    assert_eq!(scope.lifecycle_snapshot(), Default::default());
}

fn timeout_exact_local_call(executable: usize, site: InstructionSourceSite) -> LinkedExprIr {
    LinkedExprIr::Call {
        call: CallIr {
            target: LinkedCallTarget::Executable {
                addr: ExecutableAddr {
                    unit: UnitAddr::Service,
                    file: FileAddr::FileIrIdentity(TIMEOUT_FILE_ID.to_string()),
                    executable,
                },
            },
            site,
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
            actor_metadata: None,
        },
    }
}

fn timeout_terminal_executable(value: &str) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "timeoutBarrierTerminal".to_string(),
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
            statements: vec![return_statement(0)],
            expressions: vec![string_expr(value)],
        },
    }
}

fn assert_program_call_depth_error(error: &RuntimeError) {
    assert!(matches!(
        unwrap_diagnostic_source_context(error),
        RuntimeError::ResourceLimitExceeded {
            resource,
            limit: 32,
            current: 32,
            requested_delta: 1,
            ..
        } if resource == "programCallDepth"
    ));
}

#[tokio::test]
async fn f445h_e4r_tail_call_negative_timeout_keeps_ordinary_depth_and_deadline_owner() {
    let base = Instant::now();
    let wrapper_site = source_site(9);
    let call_site = source_site(8);
    let terminal_addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(TIMEOUT_FILE_ID.to_string()),
        executable: 1,
    };
    let fixture = LinkedTimeoutFixture::with_body_and_additional_executables(
        vec![
            timeout_exact_local_call(1, call_site),
            LinkedExprIr::LoadSlot { slot: 0 },
        ],
        vec![
            LinkedStmtIr::Timeout {
                duration_ms: 5,
                body: "timed".to_string(),
                site: wrapper_site.clone(),
            },
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            return_statement(1),
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            },
            BlockIr {
                label: "timed".to_string(),
                statements: vec![StmtRefIr { statement: 1 }, StmtRefIr { statement: 2 }],
            },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "result".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
        vec![timeout_terminal_executable("ordinary-result")],
    );
    let LinkedExprIr::Call { call } = &fixture.file.executables[0].body.expressions[0] else {
        panic!("timeout barrier probe must remain an exact local call");
    };
    assert!(matches!(
        &call.target,
        LinkedCallTarget::Executable { addr } if addr == &terminal_addr
    ));

    let (normal_cancellation, normal_root) = root_scope(None);
    let normal_control = ScopeAwareControl::available(normal_root, normal_cancellation.token());
    let normal = fixture
        .execute(context_with_clock(
            normal_control.clone(),
            fixed_clock(base),
        ))
        .await
        .result
        .expect("ordinary exact call returns through the timeout owner");
    assert_eq!(
        returned_value(&normal).value(),
        &RuntimeValue::String("ordinary-result".to_string())
    );
    assert_parent_restored(&normal_control);

    let (depth_cancellation, depth_root) = root_scope(None);
    let depth_control = ScopeAwareControl::available(depth_root, depth_cancellation.token());
    let depth_run = fixture
        .execute(
            context_with_clock(depth_control.clone(), fixed_clock(base))
                .with_program_call_depth_for_test(super::super::MAX_PROGRAM_CALL_DEPTH),
        )
        .await;
    let depth_error = depth_run
        .result
        .expect_err("timeout body must use an ordinary depth-checked local call");
    assert_program_call_depth_error(&depth_error);
    assert_parent_restored(&depth_control);

    let deadline = base + Duration::from_millis(5);
    let deadline_calls = Arc::new(AtomicU64::new(0));
    let (deadline_cancellation, deadline_root) = root_scope(None);
    let deadline_control =
        ScopeAwareControl::available(deadline_root, deadline_cancellation.token());
    let deadline_run = fixture
        .execute(context_with_clock(
            deadline_control.clone(),
            ScriptedClock::new(
                {
                    let mut values = vec![base; 9];
                    values.push(deadline);
                    values
                },
                Arc::clone(&deadline_calls),
            ),
        ))
        .await;
    let deadline_error = deadline_run
        .result
        .expect_err("deadline after the local call must remain owned by the timeout wrapper");
    assert_timeout_exception(
        uncaught_exception(&deadline_error),
        &deadline_run.heap,
        &wrapper_site,
        &wrapper_site,
        1,
    );
    assert_eq!(
        deadline_calls.load(Ordering::Relaxed),
        10,
        "the deadline must cross on the timed body's post-call continuation"
    );
    assert_parent_restored(&deadline_control);
}

#[tokio::test]
async fn f445h_e4r_timeout_statement_normal_return_max_duration_and_parent_restore() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::with_body(
        vec![string_expr("after")],
        vec![
            LinkedStmtIr::Timeout {
                duration_ms: u64::MAX,
                body: "normal".to_string(),
                site: source_site(10),
            },
            return_statement(0),
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            },
            BlockIr {
                label: "normal".to_string(),
                statements: Vec::new(),
            },
        ],
        SlotLayoutIr::default(),
    );
    let run = fixture
        .execute(context_with_clock(control.clone(), fixed_clock(base)))
        .await;
    let flow = run.result.expect("normal timeout body continues");
    assert_eq!(
        returned_value(&flow).value(),
        &RuntimeValue::String("after".to_string())
    );
    assert_parent_restored(&control);

    let (return_cancellation, return_root) = root_scope(None);
    let return_control = ScopeAwareControl::available(return_root, return_cancellation.token());
    let return_fixture = LinkedTimeoutFixture::with_body(
        vec![string_expr("inside"), string_expr("unreachable")],
        vec![
            LinkedStmtIr::Timeout {
                duration_ms: u64::MAX,
                body: "returning".to_string(),
                site: source_site(11),
            },
            return_statement(1),
            return_statement(0),
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            },
            BlockIr {
                label: "returning".to_string(),
                statements: vec![StmtRefIr { statement: 2 }],
            },
        ],
        SlotLayoutIr::default(),
    );
    let returned = return_fixture
        .execute(context_with_clock(
            return_control.clone(),
            fixed_clock(base),
        ))
        .await
        .result
        .expect("return flow passes through timeout");
    assert_eq!(
        returned_value(&returned).value(),
        &RuntimeValue::String("inside".to_string())
    );
    assert_parent_restored(&return_control);
}

#[tokio::test]
async fn f445h_e4r_timeout_expression_value_uses_child_and_restores_parent() {
    let base = Instant::now();
    let calls = Arc::new(AtomicU64::new(0));
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::new(
        vec![
            string_expr("value"),
            LinkedExprIr::Timeout {
                duration_ms: u64::MAX,
                value: ExprRefIr { expression: 0 },
                site: source_site(20),
            },
        ],
        vec![return_statement(1)],
    );
    let run = fixture
        .execute(context_with_clock(
            control.clone(),
            ScriptedClock::new(vec![base], Arc::clone(&calls)),
        ))
        .await;
    let flow = run.result.expect("timeout expression returns its value");
    assert_eq!(
        returned_value(&flow).value(),
        &RuntimeValue::String("value".to_string())
    );
    assert!(
        calls.load(Ordering::Relaxed) >= 6,
        "real root expression, timeout derivation, and child expression all consume the scripted clock"
    );
    assert_parent_restored(&control);
}

#[tokio::test]
async fn f445h_e4r_timeout_local_owner_inner_catch_misses_outer_catch_hits_and_continues() {
    let base = Instant::now();
    let deadline = base + Duration::from_millis(5);
    let wrapper_site = source_site(30);
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::with_body(
        vec![
            null_expr(),
            LinkedExprIr::Catch {
                try_expression: ExprRefIr { expression: 0 },
                catch_slot: 0,
                catch_type: timeout_type(),
                body: ExprRefIr { expression: 0 },
            },
            LinkedExprIr::Timeout {
                duration_ms: 5,
                value: ExprRefIr { expression: 1 },
                site: wrapper_site.clone(),
            },
            LinkedExprIr::Catch {
                try_expression: ExprRefIr { expression: 2 },
                catch_slot: 0,
                catch_type: timeout_type(),
                body: ExprRefIr { expression: 0 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
        ],
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 3 },
            },
            return_statement(4),
        ],
        vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
        }],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "caught".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
    );
    let run = fixture
        .execute(context_with_clock(
            control.clone(),
            crossing_clock(base, deadline, 7),
        ))
        .await;
    let flow = run
        .result
        .expect("outer ordinary catch must handle the owned timeout");
    let exception = caught_exception(&flow, &run.heap);
    assert_timeout_exception(exception, &run.heap, &wrapper_site, &wrapper_site, 1);
    assert_parent_restored(&control);
}

fn nested_timeout_fixture(
    outer_duration_ms: u64,
    inner_duration_ms: u64,
    outer_site: InstructionSourceSite,
    inner_site: InstructionSourceSite,
) -> LinkedTimeoutFixture {
    LinkedTimeoutFixture::new(
        vec![
            null_expr(),
            LinkedExprIr::Timeout {
                duration_ms: inner_duration_ms,
                value: ExprRefIr { expression: 0 },
                site: inner_site,
            },
            LinkedExprIr::Timeout {
                duration_ms: outer_duration_ms,
                value: ExprRefIr { expression: 1 },
                site: outer_site,
            },
        ],
        vec![return_statement(2)],
    )
}

#[tokio::test]
async fn f445h_e4r_timeout_nested_inner_earlier_materializes_inner_only() {
    let base = Instant::now();
    let inner_deadline = base + Duration::from_millis(5);
    let outer_site = source_site(40);
    let inner_site = source_site(41);
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let run = nested_timeout_fixture(10, 5, outer_site, inner_site.clone())
        .execute(context_with_clock(
            control.clone(),
            crossing_clock(base, inner_deadline, 8),
        ))
        .await;
    let error = run
        .result
        .expect_err("inner deadline must leave as a user timeout");
    assert_timeout_exception(
        uncaught_exception(&error),
        &run.heap,
        &inner_site,
        &inner_site,
        2,
    );
    assert_parent_restored(&control);
}

#[tokio::test]
async fn f445h_e4r_timeout_nested_outer_earlier_passes_inner_and_materializes_outer() {
    let base = Instant::now();
    let outer_deadline = base + Duration::from_millis(5);
    let outer_site = source_site(50);
    let inner_site = source_site(51);
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let run = nested_timeout_fixture(5, 10, outer_site.clone(), inner_site)
        .execute(context_with_clock(
            control.clone(),
            crossing_clock(base, outer_deadline, 8),
        ))
        .await;
    let error = run
        .result
        .expect_err("outer deadline must materialize after crossing inner");
    assert_timeout_exception(
        uncaught_exception(&error),
        &run.heap,
        &outer_site,
        &outer_site,
        1,
    );
    assert_parent_restored(&control);
}

#[tokio::test]
async fn f445h_e4r_timeout_equal_absolute_deadline_materializes_outer_only() {
    let base = Instant::now();
    let deadline = base + Duration::from_millis(5);
    let outer_site = source_site(60);
    let inner_site = source_site(61);
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let run = nested_timeout_fixture(5, 5, outer_site.clone(), inner_site)
        .execute(context_with_clock(
            control.clone(),
            crossing_clock(base, deadline, 8),
        ))
        .await;
    let error = run
        .result
        .expect_err("equal deadline must be owned by the outer wrapper");
    assert_timeout_exception(
        uncaught_exception(&error),
        &run.heap,
        &outer_site,
        &outer_site,
        1,
    );
    assert_parent_restored(&control);
}

#[tokio::test]
async fn f445h_e4r_timeout_inherited_request_deadline_is_not_extended_materialized_or_caught() {
    let base = Instant::now();
    let request_deadline = base + Duration::from_millis(5);
    let wrapper_site = source_site(70);
    let (cancellation, root) = root_scope(Some(request_deadline));
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::new(
        vec![
            null_expr(),
            LinkedExprIr::Timeout {
                duration_ms: 10,
                value: ExprRefIr { expression: 0 },
                site: wrapper_site,
            },
            LinkedExprIr::Catch {
                try_expression: ExprRefIr { expression: 1 },
                catch_slot: 0,
                catch_type: timeout_type(),
                body: ExprRefIr { expression: 0 },
            },
        ],
        vec![return_statement(2)],
    );
    let run = fixture
        .execute(context_with_clock(
            control.clone(),
            crossing_clock(base, request_deadline, 7),
        ))
        .await;
    let error = run
        .result
        .expect_err("request-owned terminal must cross the local catch");
    assert!(user_exception_for_catch(&error).is_none());
    assert!(error.ordinary_catch_projection().is_none());
    let terminal = error
        .scope_terminal()
        .expect("inherited request deadline stays an internal carrier")
        .terminal();
    let ExecutionScopeTerminal::InheritedDeadlineExceeded(deadline) = terminal else {
        panic!("request-owned deadline must remain inherited, got {terminal:?}");
    };
    assert_eq!(deadline.at(), request_deadline);
    assert_eq!(deadline.source(), &ExecutionDeadlineSource::Request);
    assert_eq!(deadline.nesting(), 0);
    assert_parent_restored(&control);
}

#[derive(Clone)]
struct CancelAtCallClock {
    base: Instant,
    deadline: Instant,
    cancel_at: u64,
    calls: Arc<AtomicU64>,
    cancellation: CancellationSource,
}

impl EvalMonotonicClock for CancelAtCallClock {
    fn now(&self) -> Instant {
        let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if call == self.cancel_at {
            self.cancellation.cancel();
            self.deadline
        } else {
            self.base
        }
    }
}

#[tokio::test]
async fn f445h_e4r_timeout_ancestor_cancel_wins_same_poll_and_lifecycle_returns_zero() {
    let base = Instant::now();
    let deadline = base + Duration::from_millis(5);
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::new(
        vec![
            null_expr(),
            LinkedExprIr::Timeout {
                duration_ms: 5,
                value: ExprRefIr { expression: 0 },
                site: source_site(80),
            },
        ],
        vec![return_statement(1)],
    );
    let run = fixture
        .execute(context_with_clock(
            control.clone(),
            CancelAtCallClock {
                base,
                deadline,
                cancel_at: 6,
                calls: Arc::new(AtomicU64::new(0)),
                cancellation,
            },
        ))
        .await;
    assert!(matches!(run.result, Err(RuntimeError::Cancelled)));
    assert_parent_restored(&control);
}

#[tokio::test]
async fn f445h_e4r_timeout_zero_millis_statement_and_expression_use_real_root_arms() {
    let base = Instant::now();
    let statement_site = source_site(90);
    let (cancellation, root) = root_scope(None);
    let statement_control = ScopeAwareControl::available(root, cancellation.token());
    let statement = LinkedTimeoutFixture::with_body(
        vec![null_expr()],
        vec![
            LinkedStmtIr::Timeout {
                duration_ms: 0,
                body: "child".to_string(),
                site: statement_site.clone(),
            },
            return_statement(0),
        ],
        vec![
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            },
            BlockIr {
                label: "child".to_string(),
                statements: Vec::new(),
            },
        ],
        SlotLayoutIr::default(),
    )
    .execute(context_with_clock(
        statement_control.clone(),
        fixed_clock(base),
    ))
    .await;
    let statement_error = statement
        .result
        .expect_err("zero-millisecond statement times out at child block entry");
    assert_timeout_exception(
        uncaught_exception(&statement_error),
        &statement.heap,
        &statement_site,
        &statement_site,
        1,
    );
    assert_parent_restored(&statement_control);

    let expression_site = source_site(91);
    let (expression_cancellation, expression_root) = root_scope(None);
    let expression_control =
        ScopeAwareControl::available(expression_root, expression_cancellation.token());
    let expression = LinkedTimeoutFixture::new(
        vec![
            null_expr(),
            LinkedExprIr::Timeout {
                duration_ms: 0,
                value: ExprRefIr { expression: 0 },
                site: expression_site.clone(),
            },
        ],
        vec![return_statement(1)],
    )
    .execute(context_with_clock(
        expression_control.clone(),
        fixed_clock(base),
    ))
    .await;
    let expression_error = expression
        .result
        .expect_err("zero-millisecond expression times out at child expression entry");
    assert_timeout_exception(
        uncaught_exception(&expression_error),
        &expression.heap,
        &expression_site,
        &expression_site,
        1,
    );
    assert_parent_restored(&expression_control);
}

#[tokio::test]
async fn f445h_e4r_timeout_ordinary_catch_rethrow_preserves_materialized_owner() {
    let base = Instant::now();
    let owner_site = source_site(100);
    let transparent_site = source_site(101);
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::with_body(
        vec![
            null_expr(),
            LinkedExprIr::Timeout {
                duration_ms: 0,
                value: ExprRefIr { expression: 0 },
                site: owner_site.clone(),
            },
            LinkedExprIr::Catch {
                try_expression: ExprRefIr { expression: 1 },
                catch_slot: 0,
                catch_type: timeout_type(),
                body: ExprRefIr { expression: 0 },
            },
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Field {
                object: ExprRefIr { expression: 3 },
                field: "exception".to_string(),
            },
            LinkedExprIr::Rethrow { exception_slot: 1 },
            LinkedExprIr::Timeout {
                duration_ms: u64::MAX,
                value: ExprRefIr { expression: 5 },
                site: transparent_site,
            },
        ],
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Let {
                slot: 1,
                value: ExprRefIr { expression: 4 },
            },
            return_statement(6),
        ],
        vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![
                StmtRefIr { statement: 0 },
                StmtRefIr { statement: 1 },
                StmtRefIr { statement: 2 },
            ],
        }],
        SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "caught".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "exception".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
    );
    let run = fixture
        .execute(context_with_clock(control.clone(), fixed_clock(base)))
        .await;
    let error = run
        .result
        .expect_err("rethrow crosses a later timeout wrapper unchanged");
    assert_timeout_exception(
        uncaught_exception(&error),
        &run.heap,
        &owner_site,
        &owner_site,
        1,
    );
    assert_parent_restored(&control);
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
async fn f445h_e4r_timeout_future_drop_keeps_parent_scope_and_zero_lifecycle() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let control = ScopeAwareControl::available(root, cancellation.token());
    let fixture = LinkedTimeoutFixture::new(
        vec![
            LinkedExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(86_400_000_u64),
                },
            },
            LinkedExprIr::Call {
                call: CallIr {
                    target: LinkedCallTarget::Native {
                        target: NativeTarget {
                            namespace: "std.time".to_string(),
                            symbol: "sleep".to_string(),
                            binding_key: Some("std.time.sleep".to_string()),
                            metadata: BTreeMap::new(),
                        },
                    },
                    site: InstructionSourceSite::Synthetic {
                        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
                    },
                    args: vec![ExprRefIr { expression: 0 }],
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                    actor_metadata: None,
                },
            },
            LinkedExprIr::Timeout {
                duration_ms: u64::MAX,
                value: ExprRefIr { expression: 1 },
                site: source_site(110),
            },
        ],
        vec![return_statement(2)],
    )
    .with_std_duration();
    let mut future =
        Box::pin(fixture.execute(context_with_clock(control.clone(), fixed_clock(base))));
    match first_poll(future.as_mut()) {
        Poll::Pending => {}
        Poll::Ready(run) => panic!(
            "long native wait must expose a droppable real evaluator future, got {:?}",
            run.result
        ),
    }
    drop(future);
    assert_parent_restored(&control);
}
