use crate::heap_access::HeapAccess;
use skiff_runtime_linked_program::TypeAddr;
use skiff_runtime_model::{
    runtime_value::RuntimeValueCarrier,
    service_error::{
        CatchIdentity, LiteralIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
        NamedUnionOwnerIdentity, NominalTypeIdentity,
    },
};

use super::*;
use crate::env::Env;

const NOMINAL_TYPE_INDEX: u32 = 0;
const UNION_TYPE_INDEX: u32 = 1;
const REPRESENTATION_TYPE_INDEX: u32 = 2;
const TERMINAL_EXECUTABLE_INDEX: u32 = 2;

impl CanonicalTailCallFixture {
    async fn execute_carriers(
        self,
        heap: RequestHeap,
        args: Vec<RuntimeValueCarrier>,
        initial_depth: usize,
    ) -> Result<(RuntimeValueCarrier, RequestHeap), RuntimeError> {
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let entry_addr = self.entry_addr.clone();
        let context = execution_context(&interpreter, self.eval_target)
            .with_program_call_depth_for_test(initial_depth);
        let heap = heap;
        let mut access = HeapAccess::private(heap);
        let value = interpreter
            .call_program_executable_carriers(
                context,
                &mut access,
                &Env::new(),
                &entry_addr,
                &entry_addr,
                &BTreeMap::new(),
                args,
            )
            .await?;
        Ok((value, access.into_owned_heap()))
    }
}

#[tokio::test]
async fn assembly_tail_call_carrier_materialization_matches_ordinary_for_exact_nominal_union_and_representation(
) {
    let nominal_type = TypeRefIr::LocalType {
        type_index: NOMINAL_TYPE_INDEX,
    };
    let mut tail_heap = RequestHeap::default();
    let tail_object = raw_nominal_object(&mut tail_heap);
    let mut ordinary_heap = RequestHeap::default();
    let ordinary_object = raw_nominal_object(&mut ordinary_heap);
    let ((tail, _), (ordinary, _)) = execute_pair(
        nominal_type,
        tail_heap,
        RuntimeValue::Heap(tail_object).into(),
        ordinary_heap,
        RuntimeValue::Heap(ordinary_object).into(),
    )
    .await;
    assert_carrier_parity(
        &tail,
        &ordinary,
        &nominal_identity(NOMINAL_TYPE_INDEX),
        "nominal record",
    );

    let union_type = TypeRefIr::LocalType {
        type_index: UNION_TYPE_INDEX,
    };
    let ((tail, _), (ordinary, _)) = execute_pair(
        union_type,
        RequestHeap::default(),
        RuntimeValue::String("right".to_string()).into(),
        RequestHeap::default(),
        RuntimeValue::String("right".to_string()).into(),
    )
    .await;
    assert_carrier_parity(
        &tail,
        &ordinary,
        &right_union_branch_identity(),
        "named-union branch",
    );
    assert_eq!(tail.value(), &RuntimeValue::String("right".to_string()));

    let representation_type = TypeRefIr::LocalType {
        type_index: REPRESENTATION_TYPE_INDEX,
    };
    let ((tail, _), (ordinary, _)) = execute_pair(
        representation_type,
        RequestHeap::default(),
        RuntimeValue::String("E_DENIED".to_string()).into(),
        RequestHeap::default(),
        RuntimeValue::String("E_DENIED".to_string()).into(),
    )
    .await;
    assert_carrier_parity(
        &tail,
        &ordinary,
        &nominal_identity(REPRESENTATION_TYPE_INDEX),
        "representation",
    );
    assert_eq!(
        tail.value(),
        &RuntimeValue::String("E_DENIED".to_string()),
        "representation materialization must keep the primitive payload"
    );
}

#[tokio::test]
async fn assembly_tail_call_carrier_materialization_matches_ordinary_for_container_elements() {
    let return_type = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::LocalType {
            type_index: UNION_TYPE_INDEX,
        }],
    };
    let mut tail_heap = RequestHeap::default();
    let tail_array = raw_union_array(&mut tail_heap);
    let mut ordinary_heap = RequestHeap::default();
    let ordinary_array = raw_union_array(&mut ordinary_heap);

    let ((tail, tail_heap), (ordinary, ordinary_heap)) = execute_pair(
        return_type,
        tail_heap,
        RuntimeValue::Heap(tail_array).into(),
        ordinary_heap,
        RuntimeValue::Heap(ordinary_array).into(),
    )
    .await;

    assert_eq!(
        tail.catch_identity(),
        ordinary.catch_identity(),
        "the outer Array carrier must have tail/ordinary parity"
    );
    assert_eq!(tail.catch_identity(), None);
    let tail_item = array_item(&tail, &tail_heap);
    let ordinary_item = array_item(&ordinary, &ordinary_heap);
    assert_carrier_parity(
        &tail_item,
        &ordinary_item,
        &right_union_branch_identity(),
        "Array element",
    );
    assert_eq!(
        tail_item.value(),
        &RuntimeValue::String("right".to_string())
    );
}

async fn execute_pair(
    return_type: TypeRefIr,
    tail_heap: RequestHeap,
    tail_arg: RuntimeValueCarrier,
    ordinary_heap: RequestHeap,
    ordinary_arg: RuntimeValueCarrier,
) -> (
    (RuntimeValueCarrier, RequestHeap),
    (RuntimeValueCarrier, RequestHeap),
) {
    let tail_fixture = carrier_fixture(return_type.clone(), 0);
    assert_exact_executable_target(
        tail_fixture.expression(0, 0, 1),
        &ExecutableAddr::package(0, 0, TERMINAL_EXECUTABLE_INDEX as usize),
    );
    let tail = tail_fixture
        .execute_carriers(tail_heap, vec![tail_arg], DEPTH_LIMIT_MINUS_ONE)
        .await
        .expect("eligible carrier call must tail-transfer at the depth limit");

    let ordinary_fixture = carrier_fixture(return_type, 1);
    assert!(
        matches!(
            ordinary_fixture.expression(0, 1, 2),
            LinkedExprIr::ValueBlock { .. }
        ),
        "the ordinary oracle must retain its lexical ValueBlock continuation"
    );
    let ordinary = ordinary_fixture
        .execute_carriers(ordinary_heap, vec![ordinary_arg], 0)
        .await
        .expect("ordinary carrier call must materialize through its nested frame");

    (tail, ordinary)
}

fn carrier_fixture(return_type: TypeRefIr, entry_executable: usize) -> CanonicalTailCallFixture {
    let mut file = FileIrUnit::empty("tail.carrier", "source:tail-carrier");
    file.type_table = carrier_types();
    file.executables = vec![
        carrier_caller("tail.carrier.direct", return_type.clone(), false),
        carrier_caller("tail.carrier.ordinary", return_type.clone(), true),
        carrier_terminal(return_type),
    ];
    CanonicalTailCallFixture::new(vec![file], 0, entry_executable)
}

fn carrier_types() -> Vec<TypeDeclIr> {
    vec![
        TypeDeclIr {
            name: "CarrierFault".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "CarrierChoice".to_string(),
            descriptor: TypeDescriptorIr::Union {
                branches: vec![
                    NamedUnionBranchIr::Literal {
                        value: LiteralIr::String {
                            value: "left".to_string(),
                        },
                    },
                    NamedUnionBranchIr::Literal {
                        value: LiteralIr::String {
                            value: "right".to_string(),
                        },
                    },
                ],
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "CarrierCode".to_string(),
            descriptor: TypeDescriptorIr::Representation {
                representation: TypeRefIr::builtin("string"),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ]
}

fn carrier_caller(symbol: &str, return_type: TypeRefIr, wrapped: bool) -> ExecutableIr {
    let mut expressions = vec![
        ExprIr::LoadSlot { slot: 0 },
        call(
            CallTargetIr::LocalExecutable {
                executable_index: TERMINAL_EXECUTABLE_INDEX,
            },
            vec![expr(0)],
            BTreeMap::new(),
        ),
    ];
    let returned = if wrapped {
        expressions.push(ExprIr::ValueBlock {
            block: "wrapped".to_string(),
            result: expr(1),
        });
        2
    } else {
        1
    };
    let mut body = direct_return_body(expressions, returned);
    if wrapped {
        body.blocks.push(block("wrapped", &[]));
    }
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: vec![param("value", 0, TypeRefIr::builtin("Json"))],
        return_type,
        self_type: None,
        slots: slots(&[("value", SlotKind::Param)]),
        may_suspend: false,
        body,
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    }
}

fn carrier_terminal(return_type: TypeRefIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "tail.carrier.terminal".to_string(),
        type_params: Vec::new(),
        params: vec![param("value", 0, TypeRefIr::builtin("Json"))],
        return_type,
        self_type: None,
        slots: slots(&[("value", SlotKind::Param)]),
        may_suspend: false,
        body: direct_return_body(vec![ExprIr::LoadSlot { slot: 0 }], 0),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    }
}

fn raw_nominal_object(heap: &mut RequestHeap) -> skiff_runtime_model::runtime_value::HeapHandle {
    heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
        "message".to_string(),
        RuntimeValue::String("denied".to_string()),
    )])))
    .expect("raw nominal object should allocate")
}

fn raw_union_array(heap: &mut RequestHeap) -> skiff_runtime_model::runtime_value::HeapHandle {
    heap.alloc_array_carriers(vec![RuntimeValueCarrier::unidentified(
        RuntimeValue::String("right".to_string()),
    )])
    .expect("raw union Array should allocate")
}

fn array_item(value: &RuntimeValueCarrier, heap: &RequestHeap) -> RuntimeValueCarrier {
    let RuntimeValue::Heap(handle) = value.value() else {
        panic!("materialized Array must remain a heap value");
    };
    heap.array_item_carrier(*handle, 0)
        .expect("materialized Array should remain readable")
        .expect("materialized Array should retain its first element")
}

fn assert_carrier_parity(
    tail: &RuntimeValueCarrier,
    ordinary: &RuntimeValueCarrier,
    expected: &CatchIdentity,
    lane: &str,
) {
    assert_eq!(
        tail.value(),
        ordinary.value(),
        "{lane} payload must have tail/ordinary parity"
    );
    assert_eq!(
        tail.catch_identity(),
        ordinary.catch_identity(),
        "{lane} catch identity must have tail/ordinary parity"
    );
    assert_eq!(
        tail.catch_identity(),
        Some(expected),
        "{lane} must retain its exact linked catch identity"
    );
}

fn nominal_identity(type_index: u32) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(local_identity(
        type_index,
    )))
}

fn right_union_branch_identity() -> CatchIdentity {
    CatchIdentity::NamedUnionBranch {
        union: NamedUnionOwnerIdentity::LocalExecution(local_identity(UNION_TYPE_INDEX)),
        branch: NamedUnionBranchIdentity::Literal {
            value: LiteralIdentity::String("right".to_string()),
        },
    }
}

fn local_identity(type_index: u32) -> LocalExecutionTypeIdentity {
    LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: type_index as usize,
        },
        type_arguments: Vec::new(),
    }
}
