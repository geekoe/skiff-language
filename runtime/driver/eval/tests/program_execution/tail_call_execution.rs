use super::*;

const NON_TAIL_DEPTH_LIMIT: u64 = 32;
const PRESSURE_HOPS: u64 = 100_000;

#[test]
fn runtime_program_non_tail_recursion_fails_at_guard_and_stays_healthy() {
    let worker = std::thread::Builder::new()
        .name("program-non-tail-recursion-limit-test".to_string())
        .stack_size(crate::config::RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("recursion test runtime should build");
            runtime.block_on(async {
                let program = Arc::new(program_with_executable(non_tail_countdown_executable()));
                let interpreter = Interpreter::with_program(program, runtime_factory());

                let mut limit_frame = test_invocation("svc.main.run");
                set_request_number_args(&mut limit_frame, &[("remaining", NON_TAIL_DEPTH_LIMIT)]);
                let value = execute_test_program_route(&interpreter, &limit_frame)
                    .await
                    .expect("the configured non-tail depth must still be enterable");
                assert_eq!(value.as_f64(), Some(NON_TAIL_DEPTH_LIMIT as f64));

                let mut overflow_frame = test_invocation("svc.main.run");
                set_request_number_args(
                    &mut overflow_frame,
                    &[("remaining", NON_TAIL_DEPTH_LIMIT + 1)],
                );
                let error = tokio::time::timeout(
                    Duration::from_secs(1),
                    execute_test_program_route(&interpreter, &overflow_frame),
                )
                .await
                .expect("the non-tail recursion guard must terminate promptly")
                .expect_err("the next nested frame must fail at the depth boundary");
                let payload = error
                    .ordinary_payload()
                    .expect("depth exhaustion must remain an ordinary request failure");
                assert_eq!(payload.code, "ResourceLimitExceeded");
                let details = payload.details.as_ref().expect("structured depth details");
                assert_eq!(details["resource"], "programCallDepth");
                assert_eq!(details["limit"], NON_TAIL_DEPTH_LIMIT);
                assert_eq!(details["current"], NON_TAIL_DEPTH_LIMIT);
                assert_eq!(details["requestedDelta"], 1);

                let mut healthy_frame = test_invocation("svc.main.run");
                set_request_number_args(&mut healthy_frame, &[("remaining", 0)]);
                let healthy = execute_test_program_route(&interpreter, &healthy_frame)
                    .await
                    .expect("the same interpreter/runtime must remain healthy");
                assert_eq!(healthy.as_f64(), Some(0.0));
            });
        })
        .expect("recursion test worker should spawn");

    worker
        .join()
        .expect("non-tail recursion must not abort the runtime process");
}

#[tokio::test]
async fn runtime_program_legacy_tail_call_executes_direct_and_mutual_chains() {
    let direct = Interpreter::with_program(
        Arc::new(program_with_executable(tail_countdown_executable("run", 0))),
        runtime_factory(),
    );
    let mut direct_frame = test_invocation("svc.main.run");
    set_request_countdown_args(&mut direct_frame, 512);
    let direct_value = execute_test_program_route(&direct, &direct_frame)
        .await
        .expect("legacy direct tail recursion should complete");
    assert_eq!(direct_value.as_f64(), Some(512.0));

    let mutual = Interpreter::with_program(
        Arc::new(program_with_executables(vec![
            tail_countdown_executable("run", 1),
            tail_countdown_executable("mutual", 0),
        ])),
        runtime_factory(),
    );
    let mut mutual_frame = test_invocation("svc.main.run");
    set_request_countdown_args(&mut mutual_frame, 513);
    let mutual_value = execute_test_program_route(&mutual, &mutual_frame)
        .await
        .expect("legacy mutual tail recursion should complete");
    assert_eq!(mutual_value.as_f64(), Some(513.0));
}

#[tokio::test]
async fn runtime_program_legacy_tail_call_accounts_for_every_finite_hop() {
    let zero = tail_countdown_instruction_count(0).await;
    let one = tail_countdown_instruction_count(1).await;
    let twenty = tail_countdown_instruction_count(20).await;
    let units_per_hop = one
        .checked_sub(zero)
        .expect("one hop must consume more instructions than the base case");

    assert!(units_per_hop > 0, "tail hops must not be accounting-free");
    assert_eq!(
        twenty,
        zero + 20 * units_per_hop,
        "finite legacy tail transfers must preserve exact per-hop accounting"
    );
}

#[tokio::test]
async fn runtime_program_legacy_tail_call_infinite_loop_hits_instruction_limit() {
    let interpreter = Interpreter::with_program(
        Arc::new(program_with_executable(infinite_tail_executable())),
        runtime_factory(),
    );
    let mut frame = test_invocation("svc.main.run");
    set_instruction_budget(&mut frame, 64);

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("instruction accounting must terminate infinite tail recursion")
    .expect_err("infinite tail recursion must exhaust the instruction budget");
    let payload = error
        .ordinary_payload()
        .expect("instruction exhaustion must remain an ordinary request failure");
    assert_eq!(payload.code, "TimeoutError");
    let details = payload.details.as_ref().expect("structured budget details");
    assert_eq!(details["reason"], "instructionLimitExceeded");
    assert_ne!(details["resource"], "programCallDepth");
}

#[tokio::test]
async fn runtime_program_legacy_tail_call_error_stack_is_bounded_after_100000_hops() {
    let program = program_with_executables_and_std_builtins(vec![
        tail_countdown_to_error_executable(),
        tail_error_executable(),
    ]);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_number_args(&mut frame, &[("remaining", PRESSURE_HOPS)]);
    let error = tokio::time::timeout(
        Duration::from_secs(30),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("the bounded diagnostic workload should finish promptly")
    .expect_err("the terminal tail target should throw");
    let RuntimeError::UserException(exception) = runtime_error_leaf(&error) else {
        panic!("expected a request-local tail terminal exception, got {error:?}");
    };
    let request = exception.request();
    assert_eq!(request.source(), &test_instruction_site());
    assert_eq!(
        request.stack(),
        [ExceptionStackFrame::Local {
            site: test_instruction_site(),
        }],
        "eliminated tail edges must not accumulate diagnostic frames"
    );
}

async fn tail_countdown_instruction_count(hops: u64) -> u64 {
    let interpreter = Interpreter::with_program(
        Arc::new(program_with_executable(tail_countdown_executable("run", 0))),
        runtime_factory(),
    );
    let mut frame = test_invocation("svc.main.run");
    set_request_countdown_args(&mut frame, hops);
    set_instruction_budget(&mut frame, 1_000_000);
    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("finite accounting fixture should complete");
    assert_eq!(value.as_f64(), Some(hops as f64));
    frame.execution_budget.stats_snapshot().instruction_count
}

fn set_instruction_budget(frame: &mut ProgramTestInvocation, limit: u64) {
    frame.execution_budget = Arc::new(crate::execution_budget::ExecutionBudget::new(
        crate::execution_budget::ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(limit),
            poll_interval: 1,
        },
        None,
    ));
}

fn set_request_countdown_args(frame: &mut ProgramTestInvocation, remaining: u64) {
    set_request_number_args(frame, &[("remaining", remaining), ("accumulator", 0)]);
}

fn set_request_number_args(frame: &mut ProgramTestInvocation, fields: &[(&str, u64)]) {
    let descriptor_fields = fields
        .iter()
        .map(|(name, _)| {
            (
                (*name).to_string(),
                json!({ "kind": "builtin", "name": "Json", "args": [] }),
            )
        })
        .collect::<serde_json::Map<String, Value>>();
    let runtime_fields = fields
        .iter()
        .map(|(name, value)| ((*name).to_string(), RuntimeValue::Number(*value as f64)))
        .collect::<RuntimeObjectFields>();
    let mut heap = RequestHeap::default();
    let args_handle = heap
        .alloc_object(RuntimeObject::unshaped(runtime_fields))
        .expect("tail-call args record should allocate");
    frame.request.payload_bytes = encode_payload(
        &RuntimeValue::Heap(args_handle),
        &json!({ "kind": "record", "fields": descriptor_fields }),
        &heap,
    )
    .expect("tail-call args payload should encode");
}

fn countdown_params(include_accumulator: bool) -> (Vec<ParamIr>, SlotLayoutIr) {
    let mut params = vec![ParamIr {
        name: "remaining".to_string(),
        slot: 0,
        ty: linked_builtin_type("Json"),
    }];
    let mut slots = vec![SlotIr {
        index: 0,
        name: "remaining".to_string(),
        kind: "param".to_string(),
    }];
    if include_accumulator {
        params.push(ParamIr {
            name: "accumulator".to_string(),
            slot: 1,
            ty: linked_builtin_type("Json"),
        });
        slots.push(SlotIr {
            index: 1,
            name: "accumulator".to_string(),
            kind: "param".to_string(),
        });
    }
    (
        params,
        SlotLayoutIr {
            frame_size: slots.len(),
            slots,
        },
    )
}

fn tail_countdown_executable(symbol: &str, target_index: usize) -> LinkedExecutable {
    let (params, slots) = countdown_params(true);
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params,
        return_type: None,
        self_type: None,
        slots,
        may_suspend: false,
        body: countdown_body(target_index, true, false),
    }
}

fn non_tail_countdown_executable() -> LinkedExecutable {
    let (params, slots) = countdown_params(false);
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params,
        return_type: None,
        self_type: None,
        slots,
        may_suspend: false,
        body: countdown_body(0, false, true),
    }
}

fn countdown_body(
    target_index: usize,
    include_accumulator: bool,
    wrap_recursive_result: bool,
) -> LinkedExecutableBody {
    let next_accumulator = if include_accumulator {
        json!({
            "kind": "binary",
            "op": "add",
            "left": { "expression": 7 },
            "right": { "expression": 5 }
        })
    } else {
        json!({ "kind": "literal", "value": { "kind": "number", "value": 0 } })
    };
    let args = if include_accumulator {
        json!([{ "expression": 6 }, { "expression": 8 }])
    } else {
        json!([{ "expression": 6 }])
    };
    let recursive_result = if wrap_recursive_result {
        json!({
            "kind": "binary",
            "op": "add",
            "left": { "expression": 5 },
            "right": { "expression": 9 }
        })
    } else {
        json!({
            "kind": "call",
            "call": {
                "site": test_instruction_site(),
                "target": {
                    "kind": "executable",
                    "addr": serde_json::to_value(
                        ExecutableAddr::service(0, target_index)
                    ).unwrap()
                },
                "args": args
            }
        })
    };
    let recursive_call = if wrap_recursive_result {
        json!({
            "kind": "call",
            "call": {
                "site": test_instruction_site(),
                "target": {
                    "kind": "executable",
                    "addr": serde_json::to_value(
                        ExecutableAddr::service(0, target_index)
                    ).unwrap()
                },
                "args": args
            }
        })
    } else {
        recursive_result.clone()
    };
    let base_ref = if include_accumulator { 3 } else { 1 };
    let recursive_ref = if wrap_recursive_result { 10 } else { 9 };

    executable_body(json!({
        "blocks": [
            { "label": "entry", "statements": [{ "statement": 0 }] },
            { "label": "done", "statements": [{ "statement": 1 }] },
            { "label": "recurse", "statements": [{ "statement": 2 }] }
        ],
        "statements": [
            {
                "kind": "if",
                "condition": { "expression": 2 },
                "thenBlock": "done",
                "elseBlock": "recurse"
            },
            { "kind": "return", "value": { "expression": base_ref } },
            { "kind": "return", "value": { "expression": recursive_ref } }
        ],
        "expressions": [
            { "kind": "loadSlot", "slot": 0 },
            { "kind": "literal", "value": { "kind": "number", "value": 0 } },
            {
                "kind": "binary",
                "op": "lessThanOrEqual",
                "left": { "expression": 0 },
                "right": { "expression": 1 }
            },
            { "kind": "loadSlot", "slot": 1 },
            { "kind": "loadSlot", "slot": 0 },
            { "kind": "literal", "value": { "kind": "number", "value": 1 } },
            {
                "kind": "binary",
                "op": "subtract",
                "left": { "expression": 4 },
                "right": { "expression": 5 }
            },
            { "kind": "loadSlot", "slot": 1 },
            next_accumulator,
            recursive_call,
            recursive_result
        ]
    }))
}

fn infinite_tail_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                { "label": "entry", "statements": [{ "statement": 0 }] }
            ],
            "statements": [
                { "kind": "return", "value": { "expression": 0 } }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, 0)
                            ).unwrap()
                        },
                        "args": []
                    }
                }
            ]
        })),
    }
}

fn tail_countdown_to_error_executable() -> LinkedExecutable {
    let (params, slots) = countdown_params(false);
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params,
        return_type: None,
        self_type: None,
        slots,
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                { "label": "entry", "statements": [{ "statement": 0 }] },
                { "label": "fail", "statements": [{ "statement": 1 }] },
                { "label": "recurse", "statements": [{ "statement": 2 }] }
            ],
            "statements": [
                {
                    "kind": "if",
                    "condition": { "expression": 2 },
                    "thenBlock": "fail",
                    "elseBlock": "recurse"
                },
                { "kind": "return", "value": { "expression": 7 } },
                { "kind": "return", "value": { "expression": 6 } }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "number", "value": 0 } },
                {
                    "kind": "binary",
                    "op": "lessThanOrEqual",
                    "left": { "expression": 0 },
                    "right": { "expression": 1 }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "number", "value": 1 } },
                {
                    "kind": "binary",
                    "op": "subtract",
                    "left": { "expression": 3 },
                    "right": { "expression": 4 }
                },
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, 0)
                            ).unwrap()
                        },
                        "args": [{ "expression": 5 }]
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, 1)
                            ).unwrap()
                        },
                        "args": []
                    }
                }
            ]
        })),
    }
}

fn tail_error_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "tailError".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                { "label": "entry", "statements": [{ "statement": 0 }] }
            ],
            "statements": [
                {
                    "kind": "throw",
                    "site": test_instruction_site(),
                    "value": { "expression": 2 },
                    "payloadType": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    }
                }
            ],
            "expressions": [
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "tail.pressure" }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "terminal" }
                },
                {
                    "kind": "construct",
                    "typeRef": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    },
                    "fields": {
                        "target": { "expression": 0 },
                        "message": { "expression": 1 }
                    }
                }
            ]
        })),
    }
}
