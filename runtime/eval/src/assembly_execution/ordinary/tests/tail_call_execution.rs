use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, FileAddr, LinkedCallTarget, LinkedExprIr, UnitAddr,
};
use skiff_runtime_model::runtime_value::{RuntimeObject, RuntimeObjectFields};

use super::*;
use crate::error::RuntimeError;

const DEPTH_LIMIT_MINUS_ONE: usize = 31;
const DEEP_HOPS: i64 = 96;

struct CanonicalTailCallFixture {
    image: Arc<AssemblyExecutionImage>,
    eval_target: RuntimeAssemblyEvalTarget,
    entry_addr: ExecutableAddr,
}

impl CanonicalTailCallFixture {
    fn new(mut files: Vec<FileIrUnit>, entry_file: usize, entry_executable: usize) -> Self {
        for file in &mut files {
            skiff_artifact_identity::assign_file_ir_identity(file)
                .expect("tail-call File IR should receive a canonical identity");
        }

        let mut package = private_package("example.tail-call-assembly-matrix", &files[0]);
        package.files = files.iter().map(file_ref).collect();
        skiff_artifact_identity::assign_package_artifact_identities(&mut package)
            .expect("tail-call package should receive canonical identities");
        let package_ref = package_ref(&package);
        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("assembly:tail-call-matrix"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![package_ref.clone()],
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot {
                    package: package_ref.clone(),
                }],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let image =
            crate::test_support::link_package_fixture(assembly.clone(), vec![(package, files)]);
        let activation =
            activation_context(assembly.assembly_identity, package_ref.package_build_id);
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
            activation: Arc::clone(&activation),
        });
        let request = RequestActivationContext::begin(activation)
            .expect("tail-call request generation should begin");
        let eval_target = RuntimeAssemblyEvalTarget::new(Arc::clone(&image), request, resolver)
            .expect("tail-call image and activation should form an eval target");
        Self {
            image,
            eval_target,
            entry_addr: ExecutableAddr::package(0, entry_file, entry_executable),
        }
    }

    async fn execute(
        self,
        heap: RequestHeap,
        args: Vec<RuntimeValue>,
        initial_depth: usize,
    ) -> Result<(RuntimeValue, RequestHeap), RuntimeError> {
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let context = execution_context(&interpreter, self.eval_target)
            .with_program_call_depth_for_test(initial_depth);
        let mut heap = heap;
        let value = interpreter
            .execute_runtime_assembly_addr(context, &mut heap, &self.entry_addr, args)
            .await?;
        Ok((value, heap))
    }

    fn expression(&self, file: usize, executable: usize, expression: usize) -> &LinkedExprIr {
        &self.image.execution_packages()[0].files()[file].executables[executable]
            .body
            .expressions[expression]
    }
}

#[tokio::test]
async fn assembly_tail_call_direct_branch_uses_shared_trampoline_at_depth_limit() {
    let mut file = FileIrUnit::empty("tail.direct", "source:tail-direct");
    file.executables.push(integer_countdown(
        "tail.direct.count",
        CallTargetIr::LocalExecutable {
            executable_index: 0,
        },
    ));
    let fixture = CanonicalTailCallFixture::new(vec![file], 0, 0);

    assert_exact_executable_target(
        fixture.expression(0, 0, COUNTDOWN_CALL_EXPRESSION),
        &ExecutableAddr::package(0, 0, 0),
    );
    let (value, _) = fixture
        .execute(
            RequestHeap::default(),
            vec![RuntimeValue::Number(DEEP_HOPS as f64)],
            DEPTH_LIMIT_MINUS_ONE,
        )
        .await
        .expect("direct branch tail recursion must replace the active assembly frame");

    assert_eq!(value, RuntimeValue::Number(0.0));
}

#[tokio::test]
async fn assembly_tail_call_same_file_mutual_recursion_uses_shared_trampoline() {
    let mut file = FileIrUnit::empty("tail.mutual", "source:tail-mutual");
    file.executables.push(integer_countdown(
        "tail.mutual.left",
        CallTargetIr::LocalExecutable {
            executable_index: 1,
        },
    ));
    file.executables.push(integer_countdown(
        "tail.mutual.right",
        CallTargetIr::LocalExecutable {
            executable_index: 0,
        },
    ));
    let fixture = CanonicalTailCallFixture::new(vec![file], 0, 0);

    assert_exact_executable_target(
        fixture.expression(0, 0, COUNTDOWN_CALL_EXPRESSION),
        &ExecutableAddr::package(0, 0, 1),
    );
    assert_exact_executable_target(
        fixture.expression(0, 1, COUNTDOWN_CALL_EXPRESSION),
        &ExecutableAddr::package(0, 0, 0),
    );
    let (value, _) = fixture
        .execute(
            RequestHeap::default(),
            vec![RuntimeValue::Number(DEEP_HOPS as f64)],
            DEPTH_LIMIT_MINUS_ONE,
        )
        .await
        .expect("same-file mutual recursion must not push the active assembly depth");

    assert_eq!(value, RuntimeValue::Number(0.0));
}

#[tokio::test]
async fn assembly_tail_call_cross_module_publications_keep_exact_addresses_and_execute() {
    let mut alpha = FileIrUnit::empty("tail.alpha", "source:tail-alpha");
    alpha.executables.push(integer_countdown(
        "tail.alpha.ping",
        CallTargetIr::PublicationExecutable {
            module_path: "tail.beta".to_string(),
            executable_index: 0,
        },
    ));
    let mut beta = FileIrUnit::empty("tail.beta", "source:tail-beta");
    beta.executables.push(integer_countdown(
        "tail.beta.pong",
        CallTargetIr::PublicationExecutable {
            module_path: "tail.alpha".to_string(),
            executable_index: 0,
        },
    ));
    let fixture = CanonicalTailCallFixture::new(vec![alpha, beta], 0, 0);

    assert_exact_executable_target(
        fixture.expression(0, 0, COUNTDOWN_CALL_EXPRESSION),
        &ExecutableAddr::package(0, 1, 0),
    );
    assert_exact_executable_target(
        fixture.expression(1, 0, COUNTDOWN_CALL_EXPRESSION),
        &ExecutableAddr::package(0, 0, 0),
    );
    let (value, _) = fixture
        .execute(
            RequestHeap::default(),
            vec![RuntimeValue::Number(DEEP_HOPS as f64)],
            DEPTH_LIMIT_MINUS_ONE,
        )
        .await
        .expect("cross-module publication recursion must use exact assembly addresses");

    assert_eq!(value, RuntimeValue::Number(0.0));
}

#[tokio::test]
async fn assembly_tail_call_generic_substitution_survives_frame_replacement() {
    let mut file = FileIrUnit::empty("tail.generic", "source:tail-generic");
    file.executables.push(generic_entry());
    file.executables.push(generic_countdown());
    let fixture = CanonicalTailCallFixture::new(vec![file], 0, 0);

    let (value, _) = fixture
        .execute(
            RequestHeap::default(),
            vec![
                RuntimeValue::Number(DEEP_HOPS as f64),
                RuntimeValue::String("generic-value".to_string()),
            ],
            DEPTH_LIMIT_MINUS_ONE,
        )
        .await
        .expect("generic type substitutions must be prepared before the caller frame is replaced");

    assert_eq!(value, RuntimeValue::String("generic-value".to_string()));
}

#[tokio::test]
async fn assembly_tail_call_impl_self_retains_the_explicit_self_carrier() {
    let mut file = FileIrUnit::empty("tail.impl", "source:tail-impl");
    file.executables.push(impl_entry());
    file.executables.push(impl_countdown());
    let fixture = CanonicalTailCallFixture::new(vec![file], 0, 0);

    let (value, _) = fixture
        .execute(
            RequestHeap::default(),
            vec![
                RuntimeValue::String("receiver".to_string()),
                RuntimeValue::Number(DEEP_HOPS as f64),
            ],
            DEPTH_LIMIT_MINUS_ONE,
        )
        .await
        .expect("impl self recursion must retain the prepared self value");

    assert_eq!(value, RuntimeValue::String("receiver".to_string()));
}

#[tokio::test]
async fn assembly_tail_call_arguments_are_left_to_right_once_and_keep_the_heap_carrier() {
    let record_type = phase_record_type();
    let mut file = FileIrUnit::empty("tail.arguments", "source:tail-arguments");
    file.executables.push(argument_entry(record_type.clone()));
    file.executables
        .push(argument_terminal(record_type.clone()));
    file.executables.push(argument_step(record_type));
    let fixture = CanonicalTailCallFixture::new(vec![file], 0, 0);
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "phase".to_string(),
            RuntimeValue::String("start".to_string()),
        )])))
        .expect("argument-order object should allocate");

    let (value, heap) = fixture
        .execute(heap, vec![RuntimeValue::Heap(input)], 0)
        .await
        .expect("ordered single argument evaluation should reach the terminal callable");

    assert_eq!(
        value,
        RuntimeValue::Heap(input),
        "the common record return plan must retain the exact request-heap carrier"
    );
    let HeapNode::Object(object) = heap.get(input).expect("returned argument-order object") else {
        panic!("argument-order result should remain an object");
    };
    assert_eq!(
        object.fields().get("phase"),
        Some(&RuntimeValue::String("second".to_string())),
        "the state machine fails if either argument is reordered or evaluated twice"
    );
}

#[tokio::test]
async fn assembly_tail_call_unequal_return_plan_uses_ordinary_depth_checked_call() {
    let (ordinary_value, _) = unequal_plan_fixture()
        .execute(RequestHeap::default(), Vec::new(), 0)
        .await
        .expect("ordinary fallback must retain the callee result and caller materialization");
    assert_eq!(ordinary_value, RuntimeValue::String("terminal".to_string()));

    let error = unequal_plan_fixture()
        .execute(RequestHeap::default(), Vec::new(), DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect_err("unequal return plans must retain the ordinary nested-call frame");

    assert_program_depth_error(error);
}

#[tokio::test]
async fn assembly_tail_call_value_wrapper_is_a_lexical_barrier() {
    let mut file = FileIrUnit::empty("tail.wrapper", "source:tail-wrapper");
    file.executables.push(value_wrapper_caller());
    file.executables.push(number_terminal());
    let fixture = CanonicalTailCallFixture::new(vec![file], 0, 0);

    assert!(
        matches!(fixture.expression(0, 0, 1), LinkedExprIr::ValueBlock { .. }),
        "Return.value must remain a wrapper rather than the nested exact call"
    );
    let error = fixture
        .execute(RequestHeap::default(), Vec::new(), DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect_err("a lexical result wrapper must retain its ordinary continuation");

    assert_program_depth_error(error);
}

#[tokio::test]
async fn assembly_tail_call_package_direct_target_remains_excluded_and_depth_checked() {
    let fixture = package_direct_fixture();
    let linked_caller = &fixture
        .eval_target
        .execution_projection()
        .image()
        .execution_packages()[0]
        .files()[0]
        .executables[0];
    assert!(matches!(
        linked_caller.body.expressions.get(1),
        Some(LinkedExprIr::Call {
            call: skiff_runtime_linked_program::CallIr {
                target: LinkedCallTarget::PackageDirect { .. },
                ..
            }
        })
    ));

    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.eval_target)
        .with_program_call_depth_for_test(DEPTH_LIMIT_MINUS_ONE);
    let mut heap = RequestHeap::default();
    let input = heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("package-direct input should allocate");
    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(input)],
        )
        .await
        .expect_err("PackageDirect must retain validation and ordinary call depth");

    assert_program_depth_error(error);
}

const COUNTDOWN_CALL_EXPRESSION: usize = 7;

fn integer_countdown(symbol: &str, target: CallTargetIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: vec![param("remaining", 0, TypeRefIr::builtin("integer"))],
        return_type: TypeRefIr::builtin("integer"),
        self_type: None,
        slots: slots(&[("remaining", SlotKind::Param)]),
        may_suspend: false,
        body: countdown_body(
            target,
            Vec::new(),
            ExprIr::Literal {
                value: number_literal(0),
            },
        ),
        source_span: None,
    }
}

fn countdown_body(
    target: CallTargetIr,
    mut carried_args: Vec<ExprIr>,
    base_value: ExprIr,
) -> ExecutableBody {
    let mut expressions = vec![
        ExprIr::LoadSlot { slot: 0 },
        ExprIr::Literal {
            value: number_literal(0),
        },
        ExprIr::Binary {
            op: BinaryOpIr::LessThanOrEqual,
            left: expr(0),
            right: expr(1),
        },
        base_value,
        ExprIr::LoadSlot { slot: 0 },
        ExprIr::Literal {
            value: number_literal(1),
        },
        ExprIr::Binary {
            op: BinaryOpIr::Subtract,
            left: expr(4),
            right: expr(5),
        },
    ];
    let carried_start = expressions.len();
    expressions.append(&mut carried_args);
    let mut call_args = vec![expr(6)];
    call_args.extend((carried_start..expressions.len()).map(expr));
    expressions.push(call(target, call_args, BTreeMap::new()));
    ExecutableBody {
        blocks: vec![
            block("entry", &[0]),
            block("done", &[1]),
            block("recurse", &[2]),
        ],
        statements: vec![
            StmtIr::If {
                condition: expr(2),
                then_block: "done".to_string(),
                else_block: Some("recurse".to_string()),
            },
            StmtIr::Return {
                value: Some(expr(3)),
            },
            StmtIr::Return {
                value: Some(expr(expressions.len() - 1)),
            },
        ],
        expressions,
    }
}

fn generic_entry() -> ExecutableIr {
    let params = vec![
        param("remaining", 0, TypeRefIr::builtin("integer")),
        param("value", 1, TypeRefIr::builtin("string")),
    ];
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.generic.entry".to_string(),
        type_params: Vec::new(),
        params,
        return_type: TypeRefIr::builtin("string"),
        self_type: None,
        slots: slots(&[("remaining", SlotKind::Param), ("value", SlotKind::Param)]),
        may_suspend: false,
        body: direct_return_body(
            vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::LoadSlot { slot: 1 },
                call(
                    CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    vec![expr(0), expr(1)],
                    BTreeMap::from([("T".to_string(), TypeRefIr::builtin("string"))]),
                ),
            ],
            2,
        ),
        source_span: None,
    }
}

fn generic_countdown() -> ExecutableIr {
    let generic = TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    let mut body = countdown_body(
        CallTargetIr::LocalExecutable {
            executable_index: 1,
        },
        vec![ExprIr::LoadSlot { slot: 1 }],
        ExprIr::LoadSlot { slot: 1 },
    );
    let ExprIr::Call { call } = body.expressions.last_mut().expect("generic countdown call") else {
        panic!("generic countdown should end in a call expression");
    };
    call.type_args.insert("T".to_string(), generic.clone());
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.generic.bounce".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![
            param("remaining", 0, TypeRefIr::builtin("integer")),
            param("value", 1, generic.clone()),
        ],
        return_type: generic,
        self_type: None,
        slots: slots(&[("remaining", SlotKind::Param), ("value", SlotKind::Param)]),
        may_suspend: false,
        body,
        source_span: None,
    }
}

fn impl_entry() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.impl.entry".to_string(),
        type_params: Vec::new(),
        params: vec![
            param("receiver", 0, TypeRefIr::builtin("string")),
            param("remaining", 1, TypeRefIr::builtin("integer")),
        ],
        return_type: TypeRefIr::builtin("string"),
        self_type: None,
        slots: slots(&[
            ("receiver", SlotKind::Param),
            ("remaining", SlotKind::Param),
        ]),
        may_suspend: false,
        body: direct_return_body(
            vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::LoadSlot { slot: 1 },
                call(
                    CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    vec![expr(0), expr(1)],
                    BTreeMap::new(),
                ),
            ],
            2,
        ),
        source_span: None,
    }
}

fn impl_countdown() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::ImplMethod,
        symbol: "tail.impl.retry".to_string(),
        type_params: Vec::new(),
        params: vec![
            param("self", 0, TypeRefIr::builtin("string")),
            param("remaining", 1, TypeRefIr::builtin("integer")),
        ],
        return_type: TypeRefIr::builtin("string"),
        self_type: Some(TypeRefIr::builtin("string")),
        slots: slots(&[
            ("self", SlotKind::SelfValue),
            ("remaining", SlotKind::Param),
        ]),
        may_suspend: false,
        body: countdown_body_for_slot_one(),
        source_span: None,
    }
}

fn countdown_body_for_slot_one() -> ExecutableBody {
    let expressions = vec![
        ExprIr::LoadSlot { slot: 1 },
        ExprIr::Literal {
            value: number_literal(0),
        },
        ExprIr::Binary {
            op: BinaryOpIr::LessThanOrEqual,
            left: expr(0),
            right: expr(1),
        },
        ExprIr::LoadSlot { slot: 0 },
        ExprIr::LoadSlot { slot: 0 },
        ExprIr::LoadSlot { slot: 1 },
        ExprIr::Literal {
            value: number_literal(1),
        },
        ExprIr::Binary {
            op: BinaryOpIr::Subtract,
            left: expr(5),
            right: expr(6),
        },
        call(
            CallTargetIr::LocalExecutable {
                executable_index: 1,
            },
            vec![expr(4), expr(7)],
            BTreeMap::new(),
        ),
    ];
    ExecutableBody {
        blocks: vec![
            block("entry", &[0]),
            block("done", &[1]),
            block("recurse", &[2]),
        ],
        statements: vec![
            StmtIr::If {
                condition: expr(2),
                then_block: "done".to_string(),
                else_block: Some("recurse".to_string()),
            },
            StmtIr::Return {
                value: Some(expr(3)),
            },
            StmtIr::Return {
                value: Some(expr(8)),
            },
        ],
        expressions,
    }
}

fn phase_record_type() -> TypeRefIr {
    TypeRefIr::Record {
        fields: BTreeMap::from([("phase".to_string(), TypeRefIr::builtin("string"))]),
    }
}

fn argument_entry(record_type: TypeRefIr) -> ExecutableIr {
    let params = vec![param("state", 0, record_type.clone())];
    let expressions = vec![
        ExprIr::LoadSlot { slot: 0 },
        string_expression("start"),
        string_expression("first"),
        call(
            CallTargetIr::LocalExecutable {
                executable_index: 2,
            },
            vec![expr(0), expr(1), expr(2)],
            BTreeMap::new(),
        ),
        ExprIr::LoadSlot { slot: 0 },
        string_expression("first"),
        string_expression("second"),
        call(
            CallTargetIr::LocalExecutable {
                executable_index: 2,
            },
            vec![expr(4), expr(5), expr(6)],
            BTreeMap::new(),
        ),
        call(
            CallTargetIr::LocalExecutable {
                executable_index: 1,
            },
            vec![expr(3), expr(7)],
            BTreeMap::new(),
        ),
    ];
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.arguments.entry".to_string(),
        type_params: Vec::new(),
        params,
        return_type: record_type,
        self_type: None,
        slots: slots(&[("state", SlotKind::Param)]),
        may_suspend: false,
        body: direct_return_body(expressions, 8),
        source_span: None,
    }
}

fn argument_terminal(record_type: TypeRefIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.arguments.terminal".to_string(),
        type_params: Vec::new(),
        params: vec![
            param("first", 0, record_type.clone()),
            param("second", 1, record_type.clone()),
        ],
        return_type: record_type,
        self_type: None,
        slots: slots(&[("first", SlotKind::Param), ("second", SlotKind::Param)]),
        may_suspend: false,
        body: direct_return_body(vec![ExprIr::LoadSlot { slot: 1 }], 0),
        source_span: None,
    }
}

fn argument_step(record_type: TypeRefIr) -> ExecutableIr {
    let expressions = vec![
        ExprIr::LoadSlot { slot: 0 },
        ExprIr::Field {
            object: expr(0),
            field: "phase".to_string(),
        },
        ExprIr::LoadSlot { slot: 1 },
        ExprIr::Binary {
            op: BinaryOpIr::Equal,
            left: expr(1),
            right: expr(2),
        },
        ExprIr::LoadSlot { slot: 0 },
        ExprIr::LoadSlot { slot: 2 },
        ExprIr::LoadSlot { slot: 0 },
    ];
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.arguments.step".to_string(),
        type_params: Vec::new(),
        params: vec![
            param("state", 0, record_type.clone()),
            param("expected", 1, TypeRefIr::builtin("string")),
            param("next", 2, TypeRefIr::builtin("string")),
        ],
        return_type: record_type,
        self_type: None,
        slots: slots(&[
            ("state", SlotKind::Param),
            ("expected", SlotKind::Param),
            ("next", SlotKind::Param),
        ]),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![block("entry", &[0, 1, 2])],
            statements: vec![
                StmtIr::Assert {
                    condition: expr(3),
                    message: None,
                },
                StmtIr::Assign {
                    target: AssignTargetIr::Field {
                        object: expr(4),
                        field: "phase".to_string(),
                    },
                    value: expr(5),
                },
                StmtIr::Return {
                    value: Some(expr(6)),
                },
            ],
            expressions,
        },
        source_span: None,
    }
}

fn unequal_plan_caller() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.plan.caller".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: direct_return_body(
            vec![call(
                CallTargetIr::LocalExecutable {
                    executable_index: 1,
                },
                Vec::new(),
                BTreeMap::new(),
            )],
            0,
        ),
        source_span: None,
    }
}

fn unequal_plan_fixture() -> CanonicalTailCallFixture {
    let mut file = FileIrUnit::empty("tail.plan", "source:tail-plan");
    file.executables.push(unequal_plan_caller());
    file.executables.push(string_terminal());
    CanonicalTailCallFixture::new(vec![file], 0, 0)
}

fn string_terminal() -> ExecutableIr {
    terminal(
        "tail.plan.terminal",
        TypeRefIr::builtin("string"),
        string_expression("terminal"),
    )
}

fn value_wrapper_caller() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.wrapper.caller".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("number"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![block("entry", &[0]), block("wrapped", &[])],
            statements: vec![StmtIr::Return {
                value: Some(expr(1)),
            }],
            expressions: vec![
                call(
                    CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    Vec::new(),
                    BTreeMap::new(),
                ),
                ExprIr::ValueBlock {
                    block: "wrapped".to_string(),
                    result: expr(0),
                },
            ],
        },
        source_span: None,
    }
}

fn number_terminal() -> ExecutableIr {
    terminal(
        "tail.wrapper.terminal",
        TypeRefIr::builtin("number"),
        ExprIr::Literal {
            value: number_literal(1),
        },
    )
}

fn terminal(symbol: &str, return_type: TypeRefIr, value: ExprIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type,
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: direct_return_body(vec![value], 0),
        source_span: None,
    }
}

fn direct_return_body(expressions: Vec<ExprIr>, value: usize) -> ExecutableBody {
    ExecutableBody {
        blocks: vec![block("entry", &[0])],
        statements: vec![StmtIr::Return {
            value: Some(expr(value)),
        }],
        expressions,
    }
}

fn param(name: &str, slot: u32, ty: TypeRefIr) -> ParamIr {
    ParamIr {
        name: name.to_string(),
        slot,
        ty,
    }
}

fn slots(entries: &[(&str, SlotKind)]) -> SlotLayout {
    SlotLayout {
        slots: entries
            .iter()
            .enumerate()
            .map(|(index, (name, kind))| SlotIr {
                index: index as u32,
                name: (*name).to_string(),
                kind: *kind,
            })
            .collect(),
        frame_size: entries.len() as u32,
    }
}

fn block(label: &str, statements: &[u32]) -> BlockIr {
    BlockIr {
        label: label.to_string(),
        statements: statements
            .iter()
            .copied()
            .map(|statement| StmtRefIr { statement })
            .collect(),
    }
}

fn expr(expression: usize) -> ExprRefIr {
    ExprRefIr {
        expression: expression as u32,
    }
}

fn call(
    target: CallTargetIr,
    args: Vec<ExprRefIr>,
    type_args: BTreeMap<String, TypeRefIr>,
) -> ExprIr {
    ExprIr::Call {
        call: skiff_artifact_model::CallIr {
            target,
            site: test_instruction_site(),
            args,
            type_args,
            metadata: BTreeMap::new(),
        },
    }
}

fn number_literal(value: i64) -> LiteralIr {
    LiteralIr::Number {
        value: serde_json::Number::from(value),
    }
}

fn string_expression(value: &str) -> ExprIr {
    ExprIr::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn assert_exact_executable_target(expression: &LinkedExprIr, expected: &ExecutableAddr) {
    let LinkedExprIr::Call { call } = expression else {
        panic!("tail return expression should remain a linked call");
    };
    let LinkedCallTarget::Executable { addr } = &call.target else {
        panic!(
            "eligible canonical tail target should be exact: {:?}",
            call.target
        );
    };
    assert_eq!(addr, expected);
    assert!(matches!(
        (&addr.unit, &addr.file),
        (UnitAddr::Package(0), FileAddr::LoadedFileIndex(_))
    ));
}

fn assert_program_depth_error(error: RuntimeError) {
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded {
            ref resource,
            limit: 32,
            current: 32,
            requested_delta: 1,
            ..
        } if resource == "programCallDepth"
    ));
}
