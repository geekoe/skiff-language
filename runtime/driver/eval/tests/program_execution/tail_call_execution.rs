use super::super::support::*;
use super::*;

const NON_TAIL_DEPTH_LIMIT: u64 = 128;
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

/// Deep non-tail recursion at the raised limit (128), driven on the profile's
/// configured worker stack (`RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES`: 192 MiB
/// for debug builds, 64 MiB for release). A stack overflow aborts the whole
/// process rather than unwinding, so completing the 128-layer chain on the
/// configured stack is itself the proof that the guard boundary can be reached
/// without native-stack death. The 150-layer probe also proves an over-limit
/// chain is cut off by the structured `programCallDepth` guard before it can
/// grow further.
///
/// `SKIFF_NON_TAIL_DEPTH_STACK_KIB` / `SKIFF_NON_TAIL_DEPTH` override the stack
/// and probe depth for manual before/after stack characterization. The default
/// suite asserts 127 layers pass, 128 layers pass (exact boundary), 129 and 150
/// layers are rejected with current=128, and the interpreter stays healthy.
#[test]
fn runtime_program_non_tail_recursion_deep_chain_hits_raised_guard() {
    let stack_bytes: usize = std::env::var("SKIFF_NON_TAIL_DEPTH_STACK_KIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|kib: usize| kib * 1024)
        .unwrap_or(crate::config::RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES);
    let probe_depth: u64 = std::env::var("SKIFF_NON_TAIL_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(150)
        .max(2);

    run_deep_non_tail_scenarios(stack_bytes, probe_depth);
}

/// Release-only production evidence: 128 non-tail layers complete on 40 MiB,
/// well below the 64 MiB production worker stack (measured minimum ~34 MiB).
/// Debug builds are excluded because their unoptimized evaluator frames are far
/// larger (~1.04 MiB/layer vs ~272 KiB/layer in release) and cannot fit any
/// below-production stack at depth 128; the debug profile is covered by
/// `runtime_program_non_tail_recursion_deep_chain_hits_raised_guard` on the
/// raised 192 MiB worker stack.
#[cfg(not(debug_assertions))]
#[test]
fn runtime_program_non_tail_recursion_128_layers_fit_below_production_stack_release() {
    let stack_bytes: usize = std::env::var("SKIFF_NON_TAIL_DEPTH_STACK_KIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|kib: usize| kib * 1024)
        .unwrap_or(40 * 1024 * 1024);
    let probe_depth: u64 = std::env::var("SKIFF_NON_TAIL_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(150)
        .max(2);

    run_deep_non_tail_scenarios(stack_bytes, probe_depth);
}

fn run_deep_non_tail_scenarios(stack_bytes: usize, probe_depth: u64) {
    let worker = std::thread::Builder::new()
        .name("program-non-tail-deep-chain-small-stack-test".to_string())
        .stack_size(stack_bytes)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("deep-chain test runtime should build");
            runtime.block_on(async move {
                let program = Arc::new(program_with_executable(non_tail_countdown_executable()));
                let interpreter = Interpreter::with_program(program, runtime_factory());

                // 127 layers: comfortably below the raised limit.
                let mut below_frame = test_invocation("svc.main.run");
                set_request_number_args(&mut below_frame, &[("remaining", 127)]);
                let below = execute_test_program_route(&interpreter, &below_frame)
                    .await
                    .expect("a 127-layer non-tail chain must complete at the raised limit");
                assert_eq!(below.as_f64(), Some(127.0));

                // Exact boundary: 128 layers enter, the 129th is rejected.
                let mut limit_frame = test_invocation("svc.main.run");
                set_request_number_args(&mut limit_frame, &[("remaining", NON_TAIL_DEPTH_LIMIT)]);
                let at_limit = execute_test_program_route(&interpreter, &limit_frame)
                    .await
                    .expect("the raised non-tail depth must still be enterable");
                assert_eq!(at_limit.as_f64(), Some(NON_TAIL_DEPTH_LIMIT as f64));

                // The 129th layer must be rejected with the structured guard.
                let mut overflow_frame = test_invocation("svc.main.run");
                set_request_number_args(
                    &mut overflow_frame,
                    &[("remaining", NON_TAIL_DEPTH_LIMIT + 1)],
                );
                let overflow_error = tokio::time::timeout(
                    Duration::from_secs(1),
                    execute_test_program_route(&interpreter, &overflow_frame),
                )
                .await
                .expect("the non-tail recursion guard must terminate promptly")
                .expect_err("the next nested frame must fail at the depth boundary");
                assert_depth_rejection(overflow_error, NON_TAIL_DEPTH_LIMIT);

                // Over-limit probe: 150 layers in the default suite, or an
                // env-chosen depth for stack characterization.
                let mut probe_frame = test_invocation("svc.main.run");
                set_request_number_args(&mut probe_frame, &[("remaining", probe_depth)]);
                if probe_depth <= NON_TAIL_DEPTH_LIMIT {
                    let probe = execute_test_program_route(&interpreter, &probe_frame)
                        .await
                        .expect("probe depth must complete on the restricted stack");
                    assert_eq!(probe.as_f64(), Some(probe_depth as f64));
                } else {
                    let probe_error = tokio::time::timeout(
                        Duration::from_secs(1),
                        execute_test_program_route(&interpreter, &probe_frame),
                    )
                    .await
                    .expect("the non-tail recursion guard must terminate promptly")
                    .expect_err("an over-limit chain must be cut off at the depth boundary");
                    assert_depth_rejection(probe_error, NON_TAIL_DEPTH_LIMIT);
                }

                let mut healthy_frame = test_invocation("svc.main.run");
                set_request_number_args(&mut healthy_frame, &[("remaining", 0)]);
                let healthy = execute_test_program_route(&interpreter, &healthy_frame)
                    .await
                    .expect("the same interpreter/runtime must remain healthy");
                assert_eq!(healthy.as_f64(), Some(0.0));

                eprintln!(
                    "non-tail call depth {} completed on a {}-byte worker stack; \
                     per-layer native stack upper bound <= {} bytes",
                    NON_TAIL_DEPTH_LIMIT,
                    stack_bytes,
                    stack_bytes / (NON_TAIL_DEPTH_LIMIT as usize),
                );
            });
        })
        .expect("deep-chain test worker should spawn");

    worker
        .join()
        .expect("the 128-layer non-tail chain must not overflow the restricted stack");
}

fn assert_depth_rejection(error: crate::eval::error::RuntimeError, limit: u64) {
    let payload = error
        .ordinary_payload()
        .expect("depth exhaustion must remain an ordinary request failure");
    assert_eq!(payload.code, "ResourceLimitExceeded");
    let details = payload.details.as_ref().expect("structured depth details");
    assert_eq!(details["resource"], "programCallDepth");
    assert_eq!(details["limit"], limit);
    assert_eq!(details["current"], limit);
    assert_eq!(details["requestedDelta"], 1);
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

#[tokio::test]
async fn runtime_program_legacy_tail_call_error_catch_rethrow_preserves_exact_exception() {
    let prefix_site = tail_error_prefix_site();
    let tail_site = tail_error_eliminated_site();
    let terminal_site = tail_error_terminal_site();
    let program = Arc::new(program_with_executables_and_std_builtins(vec![
        catch_rethrow_tail_error_executable(prefix_site.clone()),
        tail_countdown_to_error_executable_with_targets(1, 2, tail_site),
        tail_error_executable_at(terminal_site.clone()),
    ]));
    let file = Arc::clone(
        program
            .service_files
            .first()
            .expect("tail error program service file"),
    );
    let executable = file
        .executables
        .first()
        .expect("tail error entry executable")
        .clone();
    let interpreter = Interpreter::with_program(Arc::clone(&program), runtime_factory());
    let frame = test_invocation("svc.main.run");
    let invocation_context = program_invocation_context(&interpreter, &frame);
    let context = invocation_context.execution_context();
    let mut heap = RequestHeap::default();
    let mut env = Env::for_program_executable(&executable, Some(file.module_path.clone()), 0)
        .expect("tail error entry environment");

    let error = tokio::time::timeout(
        Duration::from_secs(30),
        interpreter.exec_program_executable(
            context,
            &mut skiff_runtime_eval::heap_access::HeapAccess::Exclusive(&mut heap),
            &mut env,
            &ExecutableAddr::service(0, 0),
            file.as_ref(),
            &executable,
        ),
    )
    .await
    .expect("the bounded catch/rethrow workload should finish promptly")
    .expect_err("the caught terminal exception should be rethrown");
    let RuntimeError::UserException(exception) = runtime_error_leaf(&error) else {
        panic!("expected a rethrown request-local tail exception, got {error:?}");
    };
    let request = exception.request();

    assert_eq!(
        exception.actual_payload_type(),
        Some(&PlatformBuiltinErrorIdentity::JsonDecode.catch_identity())
    );
    assert_eq!(request.source(), &terminal_site);
    assert_eq!(request.correlation().trace_id, "trace-program");
    assert_eq!(
        request.correlation().error_id,
        "trace-program:local-error:1"
    );
    assert_eq!(
        request.stack(),
        [
            ExceptionStackFrame::Local {
                site: prefix_site.clone(),
            },
            ExceptionStackFrame::Local {
                site: terminal_site,
            },
        ],
        "the real non-tail prefix must remain while all deep tail edges stay eliminated"
    );

    let payload_handle = request
        .local_value()
        .expect("rethrow should preserve the request-local payload")
        .as_heap_handle()
        .expect("DecodeError payload should remain a heap object");
    let target = heap
        .object_field_carrier(payload_handle, "target")
        .expect("DecodeError target should be readable")
        .expect("DecodeError target should exist");
    let message = heap
        .object_field_carrier(payload_handle, "message")
        .expect("DecodeError message should be readable")
        .expect("DecodeError message should exist");
    assert_eq!(
        target.value(),
        &RuntimeValue::String("tail.pressure".to_string())
    );
    assert_eq!(
        message.value(),
        &RuntimeValue::String("terminal".to_string())
    );

    let caught_handle = env
        .get_slot(0)
        .expect("exact catch should store its result")
        .as_heap_handle()
        .expect("catch result should be a heap object");
    let caught_tag = heap
        .object_field_carrier(caught_handle, "tag")
        .expect("catch tag should be readable")
        .expect("catch tag should exist");
    assert_eq!(caught_tag.value(), &RuntimeValue::String("err".to_string()));
    let caught_exception_handle = heap
        .object_field_carrier(caught_handle, "exception")
        .expect("caught exception should be readable")
        .expect("err catch result should carry the exception")
        .as_heap_handle()
        .expect("caught exception should remain a request-local node");
    let rethrow_exception_handle = env
        .get_slot(1)
        .expect("rethrow should load the caught exception")
        .as_heap_handle()
        .expect("rethrow slot should contain the caught exception node");
    assert_eq!(rethrow_exception_handle, caught_exception_handle);
    assert!(matches!(
        heap.get(caught_exception_handle)
            .expect("caught exception handle should resolve"),
        HeapNode::Exception(caught) if caught == request
    ));
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
    tail_countdown_to_error_executable_with_targets(0, 1, test_instruction_site())
}

fn tail_countdown_to_error_executable_with_targets(
    countdown_index: usize,
    error_index: usize,
    tail_site: skiff_artifact_model::InstructionSourceSite,
) -> LinkedExecutable {
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
                        "site": tail_site,
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, countdown_index)
                            ).unwrap()
                        },
                        "args": [{ "expression": 5 }]
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "site": tail_site,
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, error_index)
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
    tail_error_executable_at(test_instruction_site())
}

fn tail_error_executable_at(
    terminal_site: skiff_artifact_model::InstructionSourceSite,
) -> LinkedExecutable {
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
                    "site": terminal_site,
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

fn catch_rethrow_tail_error_executable(
    prefix_site: skiff_artifact_model::InstructionSourceSite,
) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
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
                SlotIr {
                    index: 2,
                    name: "$catch0".to_string(),
                    kind: "temp".to_string(),
                },
            ],
            frame_size: 3,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 0,
                    "value": { "expression": 3 }
                },
                {
                    "kind": "let",
                    "slot": 1,
                    "value": { "expression": 5 }
                },
                {
                    "kind": "rethrow",
                    "exceptionSlot": 1
                }
            ],
            "expressions": [
                {
                    "kind": "literal",
                    "value": { "kind": "number", "value": PRESSURE_HOPS }
                },
                {
                    "kind": "call",
                    "call": {
                        "site": prefix_site,
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, 1)
                            ).unwrap()
                        },
                        "args": [{ "expression": 0 }]
                    }
                },
                { "kind": "loadSlot", "slot": 2 },
                {
                    "kind": "catch",
                    "tryExpression": { "expression": 1 },
                    "catchSlot": 2,
                    "catchType": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    },
                    "body": { "expression": 2 }
                },
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "field",
                    "object": { "expression": 4 },
                    "field": "exception"
                }
            ]
        })),
    }
}

fn tail_error_prefix_site() -> skiff_artifact_model::InstructionSourceSite {
    skiff_artifact_model::InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    }
}

fn tail_error_eliminated_site() -> skiff_artifact_model::InstructionSourceSite {
    skiff_artifact_model::InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}

fn tail_error_terminal_site() -> skiff_artifact_model::InstructionSourceSite {
    skiff_artifact_model::InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}
