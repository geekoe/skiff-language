use super::intrinsic_dispatch::*;
use super::*;
use crate::control::VmLifecycleSite;
use crate::{VmCompletion, VmOwnedValues, VmTerminalCause, VmTerminalEscrow};

const PHASE_3_RETHROW_SOURCE: &str = r#"
type Leaf {
  marker: number,
}

function innerThrow(leaf: Leaf) -> void {
  throw leaf
}

function run(seed: number) -> number {
  final leaf = Leaf { marker: seed }
  final inner = catch<Leaf>(innerThrow(leaf))
  if inner.tag == "err" {
    final exception = inner.exception
    final outer = catch<Leaf>(rethrow exception)
    if outer.tag == "err" {
      return 2
    }
    return 11
  }
  return 12
}
"#;

const ARRAY_TRANSACTION_SOURCE: &str = r#"
type Item {
  value: number,
}

function run(seed: number) -> number {
  final items = [Item { value: seed }, Item { value: seed + 1 }]
  return items[0].value
}
"#;

const NOT_EQUAL_TRANSACTION_SOURCE: &str = r#"
function run(seed: number) -> number {
  if seed != 2 {
    return 1
  }
  return 0
}
"#;

const WRITABLE_TRANSACTION_SOURCE: &str = r#"
type Leaf {
  marker: number,
}

type Payload {
  child: Leaf,
}

function run(seed: number) -> number {
  final original = Payload { child: Leaf { marker: seed } }
  var mutated = original
  mutated.child = Leaf { marker: 2 }
  return original.child.marker + mutated.child.marker
}
"#;

const THREE_OWNER_TRANSACTION_SOURCE: &str = r#"
type Leaf {
  marker: number,
}

type Trio {
  first: Leaf,
  second: Leaf,
  third: Leaf,
}

function run(seed: number) -> number {
  final trio = Trio {
    first: Leaf { marker: seed },
    second: Leaf { marker: seed + 1 },
    third: Leaf { marker: seed + 2 },
  }
  return trio.first.marker
}
"#;

fn array_transaction_fixture() -> &'static ObservationFixture {
    static FIXTURE: OnceLock<ObservationFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        ObservationFixture::build_number_parameter(
            "example.com/fiber-array-transactions",
            ARRAY_TRANSACTION_SOURCE,
        )
    })
}

fn writable_transaction_fixture() -> &'static ObservationFixture {
    static FIXTURE: OnceLock<ObservationFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        ObservationFixture::build_number_parameter(
            "example.com/fiber-writable-transactions",
            WRITABLE_TRANSACTION_SOURCE,
        )
    })
}

fn three_owner_transaction_fixture() -> &'static ObservationFixture {
    static FIXTURE: OnceLock<ObservationFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        ObservationFixture::build_number_parameter(
            "example.com/fiber-three-owner-transactions",
            THREE_OWNER_TRANSACTION_SOURCE,
        )
    })
}

fn start_number_fixture(fixture: &ObservationFixture) -> VmFiber {
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]))
}

fn finish_admitted_fixture(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap) {
    for _ in 0..10_000 {
        match fiber.dispatch_one(heap).expect("admitted fixture dispatch") {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Complete(values) => {
                let mut escrow = values.into_terminal_escrow();
                escrow
                    .release_all(heap)
                    .expect("returned values release through their exact plans");
                return;
            }
            DispatchOutcome::Handoff(_) => panic!("unary admitted fixture must not hand off"),
            DispatchOutcome::Throw(_) => panic!("admitted fixture must not throw"),
        }
    }
    panic!("admitted fixture did not complete within the step cap");
}

fn dispatch_error(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap, message: &str) -> VmError {
    match fiber.dispatch_one(heap) {
        Err(error) => error,
        Ok(_) => panic!("{message}"),
    }
}

fn current_instruction(fiber: &VmFiber) -> (FunctionIndex, InstructionIndex, Opcode) {
    let frame = fiber.current_frame().expect("current transaction frame");
    let instruction = fiber
        .function(frame.function())
        .expect("current transaction function")
        .instructions()
        .get(frame.instruction().get() as usize)
        .expect("current transaction instruction");
    (frame.function(), frame.instruction(), instruction.opcode())
}

fn assert_direct_roots_match_owner_counts(
    source: &impl VmRootSource,
    heap: &IntrinsicDispatchHeap,
) {
    let mut roots = IntrinsicRootHandles::default();
    source.visit_roots(&mut roots).unwrap();
    let mut occurrences = BTreeMap::<u64, usize>::new();
    for handle in roots.0 {
        *occurrences.entry(handle).or_default() += 1;
    }
    for (handle, count) in occurrences {
        assert!(
            heap.owner_count(handle) >= count,
            "direct root {handle} has {count} external occurrences but only {} total owners",
            heap.owner_count(handle)
        );
    }
}

fn take_terminal_completion(fiber: &mut VmFiber, heap: &mut IntrinsicDispatchHeap) -> VmCompletion {
    let mut budget = ResumeBudget;
    let VmControl::Complete(completion) = fiber.run_segment(heap, &mut budget) else {
        panic!("terminal transaction must return one linear completion")
    };
    completion
}

fn drain_terminal_completion(completion: VmCompletion, heap: &mut IntrinsicDispatchHeap) {
    assert_direct_roots_match_owner_counts(&completion, heap);
    let (cause, mut escrow) = completion.into_terminal();
    if let Some(mut cause) = cause {
        cause.release_all(heap).unwrap();
    }
    escrow
        .release_all(heap)
        .expect("terminal transaction cleanup resumes from the failed owner");
    assert!(
        heap.entries.is_empty(),
        "terminal cleanup left {:?}",
        heap.debug_inventory()
    );
}

#[test]
fn operand_consume_release_retries_first_middle_and_last_from_exact_live_prefix() {
    for release_offset in 1..=3 {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = start_number_fixture(three_owner_transaction_fixture());
        for _ in 0..10_000 {
            let (function, instruction, opcode) = current_instruction(&fiber);
            if opcode == Opcode::NewRecord {
                let decoded = fiber
                    .function(function)
                    .unwrap()
                    .instructions()
                    .get(instruction.get() as usize)
                    .unwrap();
                if fiber
                    .operand_usize(decoded, 1, function, instruction)
                    .unwrap()
                    == 3
                {
                    break;
                }
            }
            assert!(matches!(
                fiber.dispatch_one(&mut heap).unwrap(),
                DispatchOutcome::Continue
            ));
        }
        let (function, instruction, opcode) = current_instruction(&fiber);
        assert_eq!(opcode, Opcode::NewRecord);
        let (_, source_start, owners) = fiber.borrow_operands(3).unwrap();
        assert!(owners
            .iter()
            .all(|owner| owner.as_request_heap_ref().is_some()));
        let (reservation, reserved) = fiber
            .reserve_operand_consume(function, instruction, opcode, 3, 1)
            .unwrap();
        assert!(reserved == owners);
        heap.fail_release_at = Some(heap.release_attempts + release_offset);

        let error = {
            let mut lifecycle = LifecycleExecutor::new(&mut heap);
            fiber
                .release_reserved_sources_reverse(&mut lifecycle, &reservation, 0, 3)
                .expect_err("selected source release must fail")
        };

        assert!(matches!(
            error,
            VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            })
        ));
        assert_eq!(fiber.state(), VmFiberState::Terminal);
        let failed_ordinal = 3 - release_offset;
        for ordinal in 0..3 {
            assert_eq!(
                fiber.live_values[source_start + ordinal],
                ordinal <= failed_ordinal,
                "only the reverse-order successful suffix may be cleared"
            );
        }
        let mut roots = IntrinsicRootHandles::default();
        fiber.visit_roots(&mut roots).unwrap();
        for (ordinal, owner) in owners.iter().enumerate() {
            let handle = owner.as_request_heap_ref().unwrap().get();
            let expected = usize::from(ordinal <= failed_ordinal);
            assert_eq!(
                roots.0.iter().filter(|root| **root == handle).count(),
                expected
            );
            assert_eq!(heap.owner_count(handle), expected);
        }
        assert_direct_roots_match_owner_counts(&fiber, &heap);

        heap.fail_release_at = None;
        let completion = take_terminal_completion(&mut fiber, &mut heap);
        drain_terminal_completion(completion, &mut heap);
    }
}

#[test]
fn admitted_equal_release_failure_retains_the_owned_operand() {
    let fixture = ObservationFixture::build_number_parameter(
        "example.com/fiber-equal-owner-transaction",
        PHASE_3_RETHROW_SOURCE,
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    fiber.set_error_correlation(ErrorCorrelation {
        trace_id: "equal-transaction-trace".to_string(),
        error_id: "equal-transaction-error".to_string(),
    });
    let mut heap = IntrinsicDispatchHeap::default();
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::Equal, 1);
    let frame = fiber.current_frame().unwrap().clone();
    let start = frame.operand_base() + frame.operand_height() - 2;
    let operands = [fiber.values[start], fiber.values[start + 1]];
    let owned = operands
        .iter()
        .copied()
        .find(|value| value.as_request_heap_ref().is_some())
        .expect("admitted catch tag comparison has one owned string");
    heap.fail_release_at = Some(heap.release_attempts + 1);

    let error = dispatch_error(
        &mut fiber,
        &mut heap,
        "owned equality operand release must fail",
    );

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(fiber.state(), VmFiberState::Terminal);
    assert_eq!(
        fiber.current_frame().unwrap().instruction(),
        frame.instruction()
    );
    assert_eq!(
        fiber.current_frame().unwrap().operand_height(),
        frame.operand_height()
    );
    let owned_handle = owned.as_request_heap_ref().unwrap().get();
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert_eq!(
        roots.0.iter().filter(|root| **root == owned_handle).count(),
        1
    );

    heap.fail_release_at = None;
    let completion = take_terminal_completion(&mut fiber, &mut heap);
    drain_terminal_completion(completion, &mut heap);
}

#[test]
fn admitted_not_equal_uses_the_same_reserved_transaction_path_for_scalar_operands() {
    // Equal above proves the owner-backed physical release path, and the
    // three-owner helper test proves first/middle/last partial failure. This
    // numeric source exists only to pin real compiler/admission reachability
    // for NotEqual, which shares the exact production handler.
    let fixture = ObservationFixture::build_number_parameter(
        "example.com/fiber-not-equal-transaction",
        NOT_EQUAL_TRANSACTION_SOURCE,
    );
    let mut fiber = start_number_fixture(&fixture);
    let mut heap = IntrinsicDispatchHeap::default();
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::NotEqual, 1);
    let frame = fiber.current_frame().unwrap().clone();

    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));

    let next = fiber.current_frame().unwrap();
    assert_ne!(next.instruction(), frame.instruction());
    assert_eq!(next.operand_height(), frame.operand_height() - 1);
    let result_index = next.operand_base() + next.operand_height() - 1;
    assert_eq!(fiber.values[result_index].as_bool(), Some(true));
    finish_admitted_fixture(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn admitted_get_dense_field_share_and_source_release_failures_preserve_both_sides() {
    for fail_release in [false, true] {
        let mut heap = IntrinsicDispatchHeap::default();
        let fixture = ObservationFixture::build_number_parameter(
            if fail_release {
                "example.com/fiber-get-dense-release"
            } else {
                "example.com/fiber-get-dense-share"
            },
            PHASE_3_RETHROW_SOURCE,
        );
        let observer = BytecodeExecutionObserver::new(
            Arc::new(RecordingSink::default()),
            observation_correlation(),
        );
        let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
        fiber.set_error_correlation(ErrorCorrelation {
            trace_id: "get-dense-transaction-trace".to_string(),
            error_id: "get-dense-transaction-error".to_string(),
        });
        drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::GetDenseField, 1);
        let frame = fiber.current_frame().unwrap().clone();
        let source_index = frame.operand_base() + frame.operand_height() - 1;
        let source = fiber.values[source_index];
        if fail_release {
            heap.fail_release_at = Some(heap.release_attempts + 1);
        } else {
            heap.fail_share_at = Some(heap.share_attempts + 1);
        }

        let error = dispatch_error(
            &mut fiber,
            &mut heap,
            "selected dense-field lifecycle boundary must fail",
        );

        assert!(fiber.live_values[source_index]);
        assert!(fiber.values[source_index] == source);
        assert_eq!(
            fiber.current_frame().unwrap().instruction(),
            frame.instruction()
        );
        if fail_release {
            assert_eq!(fiber.state(), VmFiberState::Terminal);
            assert!(matches!(
                error,
                VmError::Heap(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::ReleaseSnapshot,
                    ..
                })
            ));
            assert_eq!(fiber.terminal_escrow.len(), 1);
            heap.fail_release_at = None;
            let completion = take_terminal_completion(&mut fiber, &mut heap);
            let (cause, mut escrow) = completion.into_terminal();
            if let Some(mut cause) = cause {
                cause.release_all(&mut heap).unwrap();
            }
            escrow.release_all(&mut heap).unwrap();
            assert!(heap.entries.is_empty());
        } else {
            assert_eq!(fiber.state(), VmFiberState::Runnable);
            assert!(matches!(
                error,
                VmError::Heap(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::SnapshotShare,
                    ..
                })
            ));
            heap.fail_share_at = None;
            finish_admitted_fixture(&mut fiber, &mut heap);
            assert!(heap.entries.is_empty());
        }
    }
}

#[test]
fn admitted_array_builder_transfer_and_adoption_failures_leave_the_source_window_retryable() {
    for fail_push_after_changed_transfer in [false, true] {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = start_number_fixture(array_transaction_fixture());
        drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::ArrayBuilderPush, 1);
        let frame = fiber.current_frame().unwrap().clone();
        let source_start = frame.operand_base() + frame.operand_height() - 2;
        let builder = fiber.values[source_start];
        let item = fiber.values[source_start + 1];
        if fail_push_after_changed_transfer {
            heap.change_transfer_at = Some(heap.transfer_attempts + 1);
            heap.fail_array_push_at = Some(heap.array_push_attempts + 1);
        } else {
            heap.fail_transfer_at = Some(heap.transfer_attempts + 1);
        }

        let error = dispatch_error(
            &mut fiber,
            &mut heap,
            "selected builder ownership boundary must fail",
        );

        assert_eq!(fiber.state(), VmFiberState::Runnable);
        assert_eq!(
            fiber.current_frame().unwrap().instruction(),
            frame.instruction()
        );
        assert_eq!(
            fiber.current_frame().unwrap().operand_height(),
            frame.operand_height()
        );
        assert!(fiber.live_values[source_start]);
        assert!(fiber.live_values[source_start + 1]);
        assert!(fiber.values[source_start] == builder);
        assert_eq!(
            fiber.values[source_start + 1].as_request_heap_ref(),
            item.as_request_heap_ref()
        );
        if fail_push_after_changed_transfer {
            assert!(matches!(
                error,
                VmError::Heap(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::ArrayPushOwned,
                    ..
                })
            ));
            assert_ne!(fiber.values[source_start + 1].flags(), item.flags());
            assert_eq!(heap.array_len(&builder).unwrap(), 0);
            heap.fail_array_push_at = None;
        } else {
            assert!(matches!(
                error,
                VmError::Heap(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::TransferOwner,
                    ..
                })
            ));
            assert!(fiber.values[source_start + 1] == item);
            heap.fail_transfer_at = None;
        }

        finish_admitted_fixture(&mut fiber, &mut heap);
        assert!(heap.entries.is_empty());
    }
}

#[test]
fn admitted_freeze_array_validation_failure_keeps_the_builder_owner_in_place() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = start_number_fixture(array_transaction_fixture());
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::FreezeArray, 1);
    let frame = fiber.current_frame().unwrap().clone();
    let source_index = frame.operand_base() + frame.operand_height() - 1;
    let array = fiber.values[source_index];
    let handle = array.as_request_heap_ref().unwrap().get();
    heap.fail_validate_handle = Some(handle);

    let error = dispatch_error(
        &mut fiber,
        &mut heap,
        "freeze validation must fail before its infallible carry commit",
    );

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ValidateLive,
            ..
        })
    ));
    assert_eq!(fiber.state(), VmFiberState::Runnable);
    assert_eq!(
        fiber.current_frame().unwrap().instruction(),
        frame.instruction()
    );
    assert_eq!(
        fiber.current_frame().unwrap().operand_height(),
        frame.operand_height()
    );
    assert!(fiber.live_values[source_index]);
    assert!(fiber.values[source_index] == array);

    heap.fail_validate_handle = None;
    finish_admitted_fixture(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn admitted_array_get_share_failure_is_retryable_and_release_failure_escrows_the_result() {
    for fail_release in [false, true] {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = start_number_fixture(array_transaction_fixture());
        drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::ArrayGet, 1);
        let frame = fiber.current_frame().unwrap().clone();
        let source_start = frame.operand_base() + frame.operand_height() - 2;
        let array = fiber.values[source_start];
        let index = fiber.values[source_start + 1];
        if fail_release {
            heap.fail_release_at = Some(heap.release_attempts + 1);
        } else {
            heap.fail_share_at = Some(heap.share_attempts + 1);
        }

        let error = dispatch_error(
            &mut fiber,
            &mut heap,
            "selected array-get ownership boundary must fail",
        );

        assert_eq!(
            fiber.current_frame().unwrap().instruction(),
            frame.instruction()
        );
        assert_eq!(
            fiber.current_frame().unwrap().operand_height(),
            frame.operand_height()
        );
        assert!(fiber.live_values[source_start]);
        assert!(fiber.values[source_start] == array);
        if fail_release {
            assert_eq!(fiber.state(), VmFiberState::Terminal);
            assert!(!fiber.live_values[source_start + 1]);
            assert_eq!(fiber.terminal_escrow.len(), 1);
            assert!(matches!(
                error,
                VmError::Heap(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::ReleaseSnapshot,
                    ..
                })
            ));
            heap.fail_release_at = None;
            let completion = take_terminal_completion(&mut fiber, &mut heap);
            drain_terminal_completion(completion, &mut heap);
        } else {
            assert_eq!(fiber.state(), VmFiberState::Runnable);
            assert!(fiber.live_values[source_start + 1]);
            assert!(fiber.values[source_start + 1] == index);
            assert!(fiber.terminal_escrow.is_empty());
            assert!(matches!(
                error,
                VmError::Heap(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::SnapshotShare,
                    ..
                })
            ));
            heap.fail_share_at = None;
            finish_admitted_fixture(&mut fiber, &mut heap);
            assert!(heap.entries.is_empty());
        }
    }
}

fn writable_root_state(fiber: &VmFiber) -> (usize, ValueSlot, usize, ValueSlot) {
    let frame = fiber.current_frame().unwrap().clone();
    let decoded = fiber
        .function(frame.function())
        .unwrap()
        .instructions()
        .get(frame.instruction().get() as usize)
        .unwrap();
    assert_eq!(decoded.opcode(), Opcode::SetWritablePath);
    let LinkedInstructionTarget::FrameSlot(root_slot) = fiber
        .resolved_target(frame.function(), frame.instruction(), decoded, 0)
        .unwrap()
    else {
        panic!("SetWritablePath carries its exact writable root")
    };
    let slot_count = fiber
        .function(frame.function())
        .unwrap()
        .frame()
        .slot_types()
        .len();
    let root_index = VmFiber::slot_index(&frame, slot_count, root_slot, frame.function()).unwrap();
    let rhs_index = frame.operand_base() + frame.operand_height() - 1;
    (
        root_index,
        fiber.values[root_index],
        rhs_index,
        fiber.values[rhs_index],
    )
}

#[test]
fn admitted_writable_path_preflight_transfer_and_commit_failures_return_every_owner_for_retry() {
    for failure in ["prepare", "transfer", "commit"] {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = start_number_fixture(writable_transaction_fixture());
        drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::SetWritablePath, 1);
        let frame = fiber.current_frame().unwrap().clone();
        let (root_index, root, rhs_index, rhs) = writable_root_state(&fiber);
        match failure {
            "prepare" => heap.fail_writable_prepare_at = Some(heap.writable_prepare_attempts + 1),
            "transfer" => heap.fail_transfer_at = Some(heap.transfer_attempts + 1),
            "commit" => {
                heap.change_transfer_at = Some(heap.transfer_attempts + 1);
                heap.fail_writable_commit_at = Some(heap.writable_commit_attempts + 1);
            }
            _ => unreachable!(),
        }

        let error = dispatch_error(
            &mut fiber,
            &mut heap,
            "selected writable-path boundary must fail before adoption",
        );

        assert_eq!(fiber.state(), VmFiberState::Runnable);
        assert_eq!(
            fiber.current_frame().unwrap().instruction(),
            frame.instruction()
        );
        assert_eq!(
            fiber.current_frame().unwrap().operand_height(),
            frame.operand_height()
        );
        assert!(fiber.live_values[root_index]);
        assert!(fiber.values[root_index] == root);
        assert!(fiber.live_values[rhs_index]);
        assert_eq!(
            fiber.values[rhs_index].as_request_heap_ref(),
            rhs.as_request_heap_ref()
        );
        if failure == "commit" {
            assert_ne!(fiber.values[rhs_index].flags(), rhs.flags());
        } else {
            assert!(fiber.values[rhs_index] == rhs);
        }
        assert!(matches!(error, VmError::Heap(_)));

        heap.fail_writable_prepare_at = None;
        heap.fail_transfer_at = None;
        heap.fail_writable_commit_at = None;
        finish_admitted_fixture(&mut fiber, &mut heap);
        assert!(heap.entries.is_empty());
    }
}

#[test]
fn admitted_writable_path_cow_release_failure_retains_old_and_replacement_roots() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = start_number_fixture(writable_transaction_fixture());
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::SetWritablePath, 1);
    let frame = fiber.current_frame().unwrap().clone();
    let (root_index, root, rhs_index, _) = writable_root_state(&fiber);
    let old_handle = root.as_request_heap_ref().unwrap().get();
    assert_eq!(heap.owner_count(old_handle), 2, "source alias forces COW");
    heap.fail_release_at = Some(heap.release_attempts + 1);

    let error = dispatch_error(
        &mut fiber,
        &mut heap,
        "old writable root release must fail after COW adoption",
    );

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(fiber.state(), VmFiberState::Terminal);
    assert_eq!(
        fiber.current_frame().unwrap().instruction(),
        frame.instruction()
    );
    assert!(fiber.live_values[root_index]);
    assert!(fiber.values[root_index] == root);
    assert!(
        !fiber.live_values[rhs_index],
        "heap adopted the transferred RHS"
    );
    assert_eq!(fiber.terminal_escrow.len(), 1);
    let replacement = fiber.terminal_escrow[0].value;
    let replacement_handle = replacement.as_request_heap_ref().unwrap().get();
    assert_ne!(replacement_handle, old_handle);
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|handle| **handle == old_handle)
            .count(),
        2
    );
    assert_eq!(heap.owner_count(old_handle), 2);
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|handle| **handle == replacement_handle)
            .count(),
        1
    );
    assert_eq!(heap.owner_count(replacement_handle), 1);

    heap.fail_release_at = None;
    let completion = take_terminal_completion(&mut fiber, &mut heap);
    drain_terminal_completion(completion, &mut heap);
}

#[test]
fn intrinsic_dispatch_read_failure_keeps_all_arguments_rooted_for_safe_retry() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);

    for _ in 0..10_000 {
        if current_intrinsic(&fiber).is_some_and(|(key, _)| key == "receiver:bytes.toUtf8String@1")
        {
            break;
        }
        assert!(matches!(
            fiber.dispatch_one(&mut heap).unwrap(),
            DispatchOutcome::Continue
        ));
    }
    let frame = fiber.current_frame().unwrap().clone();
    let input_index = frame.operand_base() + frame.operand_height() - 1;
    let input = fiber.values[input_index];
    let handle = input.as_request_heap_ref().unwrap().get();
    assert_eq!(heap.owner_count(handle), 2, "slot plus borrowed operand");
    heap.fail_bytes_read = true;

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("bytes payload read failure must fail the intrinsic"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::RepresentationPayload,
            ..
        })
    ));
    assert_eq!(heap.owner_count(handle), 2);
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0.iter().filter(|root| **root == handle).count(), 2);

    heap.fail_bytes_read = false;
    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn intrinsic_result_allocation_failure_is_terminal_after_argument_release() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    drive_to_intrinsic_key(&mut fiber, &mut heap, "core.bytes.fromUtf8");
    let frame = fiber.current_frame().unwrap().clone();
    let instruction = frame.instruction();
    let input_index = frame.operand_base() + frame.operand_height() - 1;
    assert!(fiber.live_values[input_index]);
    let height = frame.operand_height();
    heap.fail_typed_bytes_at = Some(heap.typed_bytes_allocations + 1);

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("post-release result allocation failure must terminalize the dispatch"),
    };

    assert_eq!(fiber.state(), VmFiberState::Terminal);
    assert_eq!(fiber.current_frame().unwrap().instruction(), instruction);
    assert_eq!(fiber.current_frame().unwrap().operand_height(), height - 1);
    assert!(!fiber.live_values[input_index]);
    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRepresentation,
            ..
        })
    ));
    let allocations_after_failure = heap.typed_bytes_allocations;
    let mut budget = ResumeBudget;
    let VmControl::Complete(completion) = fiber.run_segment(&mut heap, &mut budget) else {
        panic!("the terminal fiber returns one linear completion")
    };
    assert!(matches!(
        completion.failure(),
        Some(VmError::FiberNotRunnable {
            state: VmFiberState::Terminal
        })
    ));
    assert_eq!(heap.typed_bytes_allocations, allocations_after_failure);
    let (Some(mut cause), mut escrow) = completion.into_terminal() else {
        panic!("the failed completion retains its diagnostic")
    };
    heap.fail_typed_bytes_at = None;
    cause.release_all(&mut heap).unwrap();
    escrow
        .release_all(&mut heap)
        .expect("terminal cleanup drains all non-argument frame owners");
    assert!(heap.entries.is_empty());
}

#[test]
fn intrinsic_dispatch_partial_release_is_terminal_and_exact_cleanup_drains_the_prefix() {
    for release_offset in [1, 2] {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = intrinsic_fiber(&mut heap);
        drive_to_owned_concat(&mut fiber, &mut heap);

        let frame = fiber.current_frame().unwrap().clone();
        let argument_start = frame.operand_base() + frame.operand_height() - 2;
        let arguments = [
            fiber.values[argument_start],
            fiber.values[argument_start + 1],
        ];
        let height = frame.operand_height();
        heap.fail_release_at = Some(heap.release_attempts + release_offset);

        let error = match fiber.dispatch_one(&mut heap) {
            Err(error) => error,
            Ok(_) => panic!("the injected intrinsic argument release must fail the dispatch"),
        };
        assert!(matches!(
            error,
            VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            })
        ));
        assert_eq!(fiber.state(), VmFiberState::Terminal);
        assert_eq!(
            fiber.current_frame().unwrap().operand_height(),
            height - (release_offset - 1),
            "only a successfully released top suffix may leave the frame height"
        );
        assert!(fiber.live_values[argument_start]);
        assert!(fiber.values[argument_start] == arguments[0]);
        assert_eq!(
            fiber.live_values[argument_start + 1],
            release_offset == 1,
            "the top argument is cleared only when its release succeeded"
        );
        let mut roots = IntrinsicRootHandles::default();
        fiber.visit_roots(&mut roots).unwrap();
        assert!(arguments[0]
            .as_request_heap_ref()
            .is_some_and(|handle| roots.0.contains(&handle.get())));
        let releases_after_failure = heap.release_attempts;
        let mut budget = ResumeBudget;
        let VmControl::Complete(completion) = fiber.run_segment(&mut heap, &mut budget) else {
            panic!("the terminal fiber returns one linear completion")
        };
        assert!(matches!(
            completion.failure(),
            Some(VmError::FiberNotRunnable {
                state: VmFiberState::Terminal
            })
        ));
        assert_eq!(heap.release_attempts, releases_after_failure);

        let (Some(mut cause), mut escrow) = completion.into_terminal() else {
            panic!("the failed completion retains its diagnostic")
        };
        heap.fail_release_at = None;
        cause.release_all(&mut heap).unwrap();
        escrow
            .release_all(&mut heap)
            .expect("terminal cleanup resumes from the remaining prefix");
        assert!(heap.entries.is_empty());
    }
}

#[test]
fn terminal_escrow_retries_first_middle_and_last_release_without_replaying_a_prefix() {
    for release_offset in 1..=3 {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = intrinsic_fiber(&mut heap);
        drive_to_owned_concat(&mut fiber, &mut heap);
        let source = live_string_owner(&fiber, &heap);
        let handle = source.as_request_heap_ref().unwrap().get();
        let baseline_owners = heap.owner_count(handle);
        let values = (0..3)
            .map(|_| {
                heap.snapshot_share(&source)
                    .expect("share terminal test owner")
            })
            .collect::<Vec<_>>();
        let site = VmLifecycleSite {
            function: fiber.current_frame().unwrap().function(),
            instruction: fiber.current_frame().unwrap().instruction(),
            opcode: Opcode::Return,
        };
        let plans = vec![Some(intrinsic_snapshot_plan()); values.len()];
        let mut escrow =
            VmTerminalEscrow::from_slots(Arc::clone(fiber.entry.image()), values, plans, site);
        assert_eq!(escrow.root_count(), 3);
        heap.fail_release_at = Some(heap.release_attempts + release_offset);

        let error = escrow
            .release_all(&mut heap)
            .expect_err("the selected escrow release must fail");

        assert!(matches!(
            error,
            VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            })
        ));
        let remaining = 4 - release_offset;
        assert_eq!(escrow.root_count(), remaining);
        assert_eq!(heap.owner_count(handle), baseline_owners + remaining);
        let mut roots = IntrinsicRootHandles::default();
        escrow.visit_roots(&mut roots).unwrap();
        assert_eq!(
            roots
                .0
                .iter()
                .filter(|candidate| **candidate == handle)
                .count(),
            remaining
        );

        heap.fail_release_at = None;
        escrow
            .release_all(&mut heap)
            .expect("retry drains only the failing owner and its prefix");
        assert!(escrow.is_empty());
        assert_eq!(heap.owner_count(handle), baseline_owners);
        drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
        assert!(heap.entries.is_empty());
    }
}

#[test]
fn abandoned_completion_moves_result_owner_into_exact_terminal_escrow() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    drive_to_owned_concat(&mut fiber, &mut heap);
    let source = live_string_owner(&fiber, &heap);
    let handle = source.as_request_heap_ref().unwrap().get();
    let baseline_owners = heap.owner_count(handle);
    let result = heap
        .snapshot_share(&source)
        .expect("share abandoned result");
    let values = VmOwnedValues::new_exact(
        Arc::clone(fiber.entry.image()),
        Box::new([result]),
        Box::new([intrinsic_snapshot_plan()]),
    );

    let completion = VmCompletion::returned(
        values,
        VmTerminalEscrow::empty(Arc::clone(fiber.entry.image())),
    );
    let (primary, mut escrow) = completion.into_terminal();

    assert!(primary.is_none());
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(heap.owner_count(handle), baseline_owners + 1);
    escrow
        .release_all(&mut heap)
        .expect("abandoned result uses its direct exact type plan");
    assert_eq!(heap.owner_count(handle), baseline_owners);
    drive_intrinsic_fiber_to_completion(&mut fiber, &mut heap);
    assert!(heap.entries.is_empty());
}

#[test]
fn uncaught_throw_moves_one_exact_owner_into_terminal_cause_and_retries_release() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-terminal-cause",
        OWNED_THROW_RESUME_SOURCE,
    );
    let mut heap = IntrinsicDispatchHeap::default();
    let (fiber, completed) = origin_throw_completion(&fixture, &mut heap);
    let diagnostic = completed
        .thrown_diagnostic()
        .expect("uncaught completion exposes only rootless throw metadata")
        .clone();

    let (Some(mut cause), escrow) = completed.into_terminal() else {
        panic!("uncaught Throw yields one owner-aware terminal cause")
    };

    assert!(escrow.is_empty());
    assert_eq!(cause.root_count(), 1);
    assert_eq!(cause.unresolved_count(), 0);
    assert_eq!(cause.thrown(), Some(&diagnostic));
    let mut fiber_roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut fiber_roots).unwrap();
    assert!(fiber_roots.0.is_empty());
    let mut cause_roots = IntrinsicRootHandles::default();
    cause.visit_roots(&mut cause_roots).unwrap();
    assert_eq!(cause_roots.0.len(), 1);
    assert_eq!(heap_owner_total(&heap), 1);

    heap.fail_release_at = Some(heap.release_attempts + 1);
    let error = cause
        .release_all(&mut heap)
        .expect_err("the injected terminal-cause release must fail");
    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(cause.root_count(), 1);
    assert_eq!(cause.thrown(), Some(&diagnostic));
    assert_eq!(heap_owner_total(&heap), 1);

    heap.fail_release_at = None;
    cause
        .release_all(&mut heap)
        .expect("retry consumes the exact same owner once");
    assert_eq!(cause.root_count(), 0);
    assert_eq!(cause.thrown(), Some(&diagnostic));
    assert!(heap.entries.is_empty());
}

#[test]
fn completion_is_single_use_and_repeated_run_cannot_reexpose_the_throw_owner() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-single-use-completion",
        OWNED_THROW_RESUME_SOURCE,
    );
    let mut heap = IntrinsicDispatchHeap::default();
    let (mut fiber, completion) = origin_throw_completion(&fixture, &mut heap);
    let first_diagnostic = completion.thrown_diagnostic().unwrap().clone();
    let cloned_diagnostic = first_diagnostic.clone();

    let mut budget = ResumeBudget;
    let VmControl::Complete(repeated) = fiber.run_segment(&mut heap, &mut budget) else {
        panic!("a completed fiber returns only a rootless terminal diagnostic")
    };
    assert!(matches!(
        repeated.failure(),
        Some(VmError::FiberNotRunnable {
            state: VmFiberState::Terminal
        })
    ));
    let mut repeated_roots = IntrinsicRootHandles::default();
    repeated.visit_roots(&mut repeated_roots).unwrap();
    assert!(repeated_roots.0.is_empty());
    let mut fiber_roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut fiber_roots).unwrap();
    assert!(fiber_roots.0.is_empty());

    let (Some(mut repeated_cause), mut repeated_escrow) = repeated.into_terminal() else {
        panic!("repeated completion retains its rootless diagnostic")
    };
    repeated_cause.release_all(&mut heap).unwrap();
    repeated_escrow.release_all(&mut heap).unwrap();
    assert_eq!(heap_owner_total(&heap), 1);

    let (Some(mut cause), mut escrow) = completion.into_terminal() else {
        panic!("the first completion retains the sole exact exception owner")
    };
    assert_eq!(cause.thrown(), Some(&first_diagnostic));
    cause.release_all(&mut heap).unwrap();
    escrow.release_all(&mut heap).unwrap();
    assert_eq!(first_diagnostic, cloned_diagnostic);
    assert!(heap.entries.is_empty());
}

#[test]
fn naked_thrown_error_is_rootless_diagnostic_and_never_mints_cleanup_authority() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-naked-thrown-diagnostic",
        OWNED_THROW_RESUME_SOURCE,
    );
    let mut heap = IntrinsicDispatchHeap::default();
    let (outcome, _) = origin_owned_throw(&fixture, &mut heap);
    let ResumeOutcome::Throw(exception) = outcome else {
        unreachable!()
    };
    let diagnostic = exception.diagnostic().clone();
    let image = Arc::clone(exception.origin_image());
    let naked = Arc::new(exception.exception().clone());
    let mut diagnostic_cause = VmTerminalCause::from_error(image, VmError::Thrown(naked));

    assert_eq!(diagnostic_cause.root_count(), 0);
    assert_eq!(diagnostic_cause.unresolved_count(), 0);
    assert_eq!(diagnostic_cause.thrown(), Some(&diagnostic));
    let mut roots = IntrinsicRootHandles::default();
    diagnostic_cause.visit_roots(&mut roots).unwrap();
    assert!(roots.0.is_empty());
    diagnostic_cause.release_all(&mut heap).unwrap();
    assert_eq!(heap_owner_total(&heap), 1);

    let mut exact = exception.into_terminal_escrow();
    exact
        .release_all(&mut heap)
        .expect("only the sealed exception carrier releases the payload");
    assert!(heap.entries.is_empty());
}

#[test]
fn phase_3_rethrow_uses_payload_authority_not_the_user_exception_slot() {
    let fixture = ObservationFixture::build_number_parameter(
        "example.com/fiber-phase3-rethrow",
        PHASE_3_RETHROW_SOURCE,
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    fiber.set_error_correlation(ErrorCorrelation {
        trace_id: "phase3-rethrow-trace".to_string(),
        error_id: "phase3-rethrow-error".to_string(),
    });
    let mut heap = IntrinsicDispatchHeap::default();
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::Rethrow, 1);

    let frame = fiber.current_frame().unwrap().clone();
    let decoded = fiber
        .function(frame.function())
        .unwrap()
        .instructions()
        .get(frame.instruction().get() as usize)
        .unwrap();
    let LinkedInstructionTarget::FrameSlot(source) = fiber
        .resolved_target(frame.function(), frame.instruction(), decoded, 0)
        .unwrap()
    else {
        panic!("Rethrow carries its user Exception source slot")
    };
    let slot_count = fiber
        .function(frame.function())
        .unwrap()
        .frame()
        .slot_types()
        .len();
    let source_index = VmFiber::slot_index(&frame, slot_count, source, frame.function()).unwrap();
    let source_value = fiber.values[source_index];
    let payload = heap.record_field(&source_value, "error").unwrap();
    let payload_handle = payload.as_handle().unwrap().get();
    let hidden_index = fiber.caught_by_payload[&payload_handle];
    assert_ne!(source_index, hidden_index);
    let caught_before = fiber.caught_exceptions[&hidden_index].clone();
    let mut roots_before = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots_before).unwrap();
    roots_before.0.sort_unstable();

    heap.fail_release_at = Some(heap.release_attempts + 1);
    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("the user Exception record release is the first rethrow mutation"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(
        fiber.current_frame().unwrap().instruction(),
        frame.instruction()
    );
    assert!(fiber.live_values[source_index]);
    assert!(fiber.values[source_index] == source_value);
    assert_eq!(
        fiber.caught_by_payload.get(&payload_handle),
        Some(&hidden_index)
    );
    let caught_after = &fiber.caught_exceptions[&hidden_index];
    assert!(Arc::ptr_eq(&caught_after.envelope, &caught_before.envelope));
    assert!(fiber.unwind.is_none());
    let mut roots_after = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots_after).unwrap();
    roots_after.0.sort_unstable();
    assert_eq!(roots_after.0, roots_before.0);

    heap.fail_release_at = None;
    assert!(matches!(
        fiber.dispatch_one(&mut heap).unwrap(),
        DispatchOutcome::Continue
    ));
    assert!(!fiber.live_values[source_index]);
    assert!(!fiber.caught_exceptions.contains_key(&hidden_index));
    let outer_hidden_index = fiber.caught_by_payload[&payload_handle];
    assert_ne!(outer_hidden_index, hidden_index);
    assert!(Arc::ptr_eq(
        &fiber.caught_exceptions[&outer_hidden_index].envelope,
        &caught_before.envelope
    ));

    let mut budget = ResumeBudget;
    let completion = loop {
        match fiber.run_segment(&mut heap, &mut budget) {
            VmControl::Continue => {}
            VmControl::Complete(completion) => break completion,
            _ => panic!("outer Phase3 catch completes without a boundary handoff"),
        }
    };
    assert_eq!(
        completion.returned_values().unwrap().values()[0].as_number(),
        Some(2.0)
    );
    let (cause, mut escrow) = completion.into_terminal();
    assert!(cause.is_none());
    escrow.release_all(&mut heap).unwrap();
    assert!(
        heap.entries.is_empty(),
        "remaining entries {:?}, owners {}",
        heap.debug_inventory(),
        heap_owner_total(&heap)
    );
}

#[test]
fn phase_3_rethrow_postcommit_failure_keeps_exact_unwind_without_replaying_source() {
    let fixture = ObservationFixture::build_number_parameter(
        "example.com/fiber-phase3-rethrow-postcommit",
        PHASE_3_RETHROW_SOURCE,
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    fiber.set_error_correlation(ErrorCorrelation {
        trace_id: "phase3-rethrow-postcommit-trace".to_string(),
        error_id: "phase3-rethrow-postcommit-error".to_string(),
    });
    let mut heap = IntrinsicDispatchHeap::default();
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::Rethrow, 1);
    let frame = fiber.current_frame().unwrap().clone();
    let decoded = fiber
        .function(frame.function())
        .unwrap()
        .instructions()
        .get(frame.instruction().get() as usize)
        .unwrap();
    let LinkedInstructionTarget::FrameSlot(source) = fiber
        .resolved_target(frame.function(), frame.instruction(), decoded, 0)
        .unwrap()
    else {
        unreachable!()
    };
    let slot_count = fiber
        .function(frame.function())
        .unwrap()
        .frame()
        .slot_types()
        .len();
    let source_index = VmFiber::slot_index(&frame, slot_count, source, frame.function()).unwrap();
    let source_handle = fiber.values[source_index].as_handle().unwrap().get();
    let source_releases = heap
        .release_history
        .iter()
        .filter(|handle| **handle == source_handle)
        .count();
    let payload = heap
        .record_field(&fiber.values[source_index], "error")
        .unwrap();
    let payload_handle = payload.as_handle().unwrap().get();
    let hidden_index = fiber.caught_by_payload[&payload_handle];
    let envelope = Arc::clone(&fiber.caught_exceptions[&hidden_index].envelope);
    heap.fail_share_at = Some(heap.share_attempts + 1);

    let error = match fiber.dispatch_one(&mut heap) {
        Err(error) => error,
        Ok(_) => panic!("outer catch share fails after the rethrow source commit"),
    };

    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::SnapshotShare,
            ..
        })
    ));
    assert!(!fiber.live_values[source_index]);
    assert_eq!(
        heap.release_history
            .iter()
            .filter(|handle| **handle == source_handle)
            .count(),
        source_releases + 1
    );
    assert!(!fiber.caught_by_payload.contains_key(&payload_handle));
    assert!(!fiber.caught_exceptions.contains_key(&hidden_index));
    let unwind = fiber
        .unwind
        .as_ref()
        .expect("exact envelope moved into unwind");
    assert!(Arc::ptr_eq(&unwind.envelope, &envelope));
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    let live_payload_slots = fiber
        .values
        .iter()
        .zip(&fiber.live_values)
        .filter(|(value, live)| {
            **live
                && value
                    .as_handle()
                    .is_some_and(|handle| handle.get() == payload_handle)
        })
        .count();
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|candidate| **candidate == payload_handle)
            .count(),
        live_payload_slots + 1,
        "the exact unwind is the only non-slot payload root"
    );

    heap.fail_share_at = None;
    let mut budget = ResumeBudget;
    let completion = loop {
        match fiber.run_segment(&mut heap, &mut budget) {
            VmControl::Continue => {}
            VmControl::Complete(completion) => break completion,
            _ => panic!("postcommit unwind resumes into the outer handler"),
        }
    };
    assert_eq!(
        completion.returned_values().unwrap().values()[0].as_number(),
        Some(2.0)
    );
    assert_eq!(
        heap.release_history
            .iter()
            .filter(|handle| **handle == source_handle)
            .count(),
        source_releases + 1,
        "resuming unwind must not replay the committed Rethrow source release"
    );
    let (cause, mut escrow) = completion.into_terminal();
    assert!(cause.is_none());
    escrow.release_all(&mut heap).unwrap();
    assert!(heap.entries.is_empty());
}

#[test]
fn owned_values_bind_exact_resume_plans_and_return_untrusted_roots_on_rejection() {
    let token = host_result_resume_token();
    let resume = token
        .image()
        .resume_sites()
        .get(token.resume_site())
        .expect("host token retains its direct resume row");
    let [result_type] = resume.result_types() else {
        panic!("unary HTTP resume has one exact result type")
    };
    let mut heap = IntrinsicDispatchHeap::default();
    let result = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(result_type.get()),
        ValueFlags::new(0),
    );

    let values = VmOwnedValues::try_from_resume(&token, Box::new([result]))
        .expect("the exact token accepts its matching runtime carrier");

    assert!(Arc::ptr_eq(values.image(), token.image()));
    let mut roots = IntrinsicRootHandles::default();
    values.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0, vec![result.as_request_heap_ref().unwrap().get()]);
    let mut escrow = values.into_terminal_escrow();
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(escrow.unresolved_count(), 0);
    escrow
        .release_all(&mut heap)
        .expect("abandoned values use the captured resume plan exactly");
    assert!(heap.entries.is_empty());

    let image_scoped = ValueSlot::const_ref(
        VmHandle::new(99),
        compact_tag(result_type.get()),
        ValueFlags::new(0),
    );
    let rejected = match VmOwnedValues::try_from_resume(&token, Box::new([image_scoped])) {
        Ok(_) => panic!("a tag-compatible constant without origin image authority must reject"),
        Err(rejected) => rejected,
    };
    assert!(rejected.values() == [image_scoped]);
    let (error, escrow) = rejected.into_terminal_escrow();
    assert_eq!(error, VmError::ResumeTokenMismatch);
    assert!(escrow.is_empty());

    let wrong_type = result_type
        .get()
        .checked_add(1)
        .expect("fixture result type leaves one compact tag");
    let result = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(wrong_type),
        ValueFlags::new(0),
    );
    let handle = result.as_request_heap_ref().unwrap().get();

    let rejected = match VmOwnedValues::try_from_resume(&token, Box::new([result])) {
        Ok(_) => panic!("a colliding but wrong runtime TypeIndex must fail closed"),
        Err(rejected) => rejected,
    };

    assert_eq!(rejected.error(), &VmError::ResumeTokenMismatch);
    assert!(Arc::ptr_eq(rejected.image(), token.image()));
    assert!(rejected.values() == [result]);
    let mut roots = IntrinsicRootHandles::default();
    rejected.visit_roots(&mut roots).unwrap();
    assert_eq!(roots.0, vec![handle]);

    let (error, mut escrow) = rejected.into_terminal_escrow();
    assert_eq!(error, VmError::ResumeTokenMismatch);
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(escrow.unresolved_count(), 1);
    assert!(matches!(
        escrow.release_all(&mut heap),
        Err(VmError::TerminalRootLifecycleUnavailable { index: 0, .. })
    ));
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(heap.owner_count(handle), 1);

    // Damaged retention survives until the request destroys its concrete
    // heap/resource authority; no guessed plan is allowed to pop it earlier.
    drop(escrow);
    heap.entries.clear();
    assert!(heap.entries.is_empty());
}

#[test]
fn same_image_cross_token_values_are_rejected_unchanged_then_resume_the_origin() {
    let mut heap = IntrinsicDispatchHeap::default();
    let (mut origin, origin_token) = host_resume_fiber_and_token(&mut heap);
    let (mut receiver, receiver_token) = host_resume_fiber_and_token(&mut heap);
    assert!(Arc::ptr_eq(origin_token.image(), receiver_token.image()));
    assert_eq!(origin_token.sequence(), receiver_token.sequence());
    assert_eq!(origin_token.function(), receiver_token.function());
    assert_eq!(origin_token.instruction(), receiver_token.instruction());
    assert_eq!(origin_token.resume_site(), receiver_token.resume_site());
    assert!(!Arc::ptr_eq(
        origin_token.binding(),
        receiver_token.binding()
    ));

    let resume = origin_token
        .image()
        .resume_sites()
        .get(origin_token.resume_site())
        .expect("origin token resolves its exact resume row");
    let [result_type] = resume.result_types() else {
        panic!("host result is unary")
    };
    let result = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(result_type.get()),
        ValueFlags::new(0),
    );
    let handle = result.as_request_heap_ref().unwrap().get();
    let values = VmOwnedValues::try_from_resume(&origin_token, Box::new([result]))
        .expect("origin token seals the materialized result");

    let failure = receiver
        .resume(receiver_token, ResumeOutcome::Values(values))
        .expect_err("same-image values cannot cross one mint binding");

    assert_eq!(failure.error(), &VmError::ResumeTokenMismatch);
    let mut roots = IntrinsicRootHandles::default();
    failure.visit_roots(&mut roots).unwrap();
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|candidate| **candidate == handle)
            .count(),
        1
    );
    let (error, Some((returned_receiver_token, ResumeOutcome::Values(returned_values)))) =
        failure.into_parts()
    else {
        panic!("rejection returns the exact token and values carrier")
    };
    assert_eq!(error, VmError::ResumeTokenMismatch);
    assert!(returned_values.is_bound_to(origin_token.binding()));
    assert!(!returned_values.is_bound_to(returned_receiver_token.binding()));

    origin
        .resume(origin_token, ResumeOutcome::Values(returned_values))
        .expect("the unchanged carrier still resumes its origin token");
    assert_eq!(origin.state(), VmFiberState::Runnable);

    receiver.state = VmFiberState::Terminal;
    let mut receiver_escrow = receiver.take_terminal_escrow();
    receiver_escrow
        .release_all(&mut heap)
        .expect("rejected receiver drains its independent frame roots");
    origin.state = VmFiberState::Terminal;
    let mut origin_escrow = origin.take_terminal_escrow();
    origin_escrow
        .release_all(&mut heap)
        .expect("accepted result and origin frame drain exactly once");
    assert!(heap.entries.is_empty());
}

#[test]
fn rejected_resume_returns_the_original_outcome_until_consuming_escrow_handoff() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    let token = loop {
        match fiber
            .dispatch_one(&mut heap)
            .expect("drive to stream handoff")
        {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Handoff(VmControl::EmitStream(item)) => {
                break item
                    .release(&mut heap)
                    .expect("release emitted stream item");
            }
            DispatchOutcome::Complete(_) => panic!("fixture must emit before completion"),
            DispatchOutcome::Handoff(_) => panic!("fixture exposes only an emit handoff"),
            DispatchOutcome::Throw(_) => panic!("fixture must not throw"),
        }
    };
    let source = live_string_owner(&fiber, &heap);
    let handle = source.as_request_heap_ref().unwrap().get();
    let baseline_owners = heap.owner_count(handle);
    let incoming = heap
        .snapshot_share(&source)
        .expect("share rejected outcome");
    let values = VmOwnedValues::new_exact(
        Arc::clone(fiber.entry.image()),
        Box::new([incoming]),
        Box::new([intrinsic_snapshot_plan()]),
    );
    let token_sequence = token.sequence();
    let token_function = token.function();
    let token_instruction = token.instruction();
    // Force a genuine TCB rejection after a port has produced an owned input.
    fiber.pending_resume = None;

    let rejected = fiber
        .resume(token, ResumeOutcome::Values(values))
        .expect_err("missing pending registration rejects the exact outcome");

    assert_eq!(rejected.error(), &VmError::ResumeNotExpected);
    let Some((resume, outcome)) = rejected.rejected_parts() else {
        panic!("owned Values must use the rejection carrier")
    };
    assert_eq!(resume.sequence(), token_sequence);
    assert_eq!(resume.function(), token_function);
    assert_eq!(resume.instruction(), token_instruction);
    let ResumeOutcome::Values(returned) = outcome else {
        panic!("the rejected input stays in its original Values envelope")
    };
    assert!(returned.values() == [incoming]);
    let mut roots = IntrinsicRootHandles::default();
    rejected.visit_roots(&mut roots).unwrap();
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|candidate| **candidate == handle)
            .count(),
        1
    );

    let (primary, mut escrow) = fiber.escrow_rejected_resume(rejected);
    assert_eq!(primary.diagnostic(), Some(&VmError::ResumeNotExpected));
    assert_eq!(escrow.root_count(), 1);
    escrow
        .release_all(&mut heap)
        .expect("rejected Values converts through its pinned exact type plan");
    assert_eq!(heap.owner_count(handle), baseline_owners);
    fiber
        .discard_terminal_roots(&mut heap)
        .expect("the rejected fiber drains its independent retained roots");
    assert!(heap.entries.is_empty());
}

#[test]
fn raw_thrown_failure_is_rejected_unchanged_but_remains_rootless_diagnostic() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-malformed-thrown-resume",
        OWNED_THROW_RESUME_SOURCE,
    );
    let function_index = FunctionIndex::new(fixture.root_function_index());
    let function = &fixture.image.functions()[function_index.get() as usize];
    let region = function.exception_regions()[0].clone();
    let LinkedCatchMatcher::Type(leaf_type) = region.catch_matchers()[0] else {
        panic!("malformed-resume fixture has one exact Leaf catch")
    };
    let mut heap = IntrinsicDispatchHeap::default();
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    while fiber.current_frame().unwrap().instruction() != region.start() {
        assert!(matches!(
            fiber.dispatch_one(&mut heap).unwrap(),
            DispatchOutcome::Continue
        ));
    }
    let payload = heap.allocate(
        IntrinsicDispatchValue::Opaque,
        compact_tag(leaf_type.get()),
        ValueFlags::new(0),
    );
    let handle = payload.as_request_heap_ref().unwrap().get();
    let identity = runtime_leaf_catch_identity(&fixture.image, &payload)
        .expect("manual malformed envelope retains a valid local identity");
    let source = InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    };
    let envelope = Arc::new(
        RequestException::local_vm(
            payload,
            identity,
            source.clone(),
            vec![skiff_runtime_model::service_error::ExceptionStackFrame::Local { site: source }],
            ErrorCorrelation {
                trace_id: "malformed-resume-trace".to_string(),
                error_id: "malformed-resume-error".to_string(),
            },
        )
        .unwrap(),
    );
    let token = fiber
        .mint_resume(
            function_index,
            region.start(),
            VmResumeAuthority::Child(ChildTarget::StreamNext),
            ResumeSiteIndex::new(0),
            region.start(),
            None,
            0,
            0,
        )
        .unwrap();
    fiber.state = VmFiberState::BlockedOnChild;

    let failure = fiber
        .resume(token, ResumeOutcome::Failure(VmError::Thrown(envelope)))
        .expect_err("raw Failure(Thrown) is never an accepted owned exception");

    assert_eq!(failure.error(), &VmError::ResumeTokenMismatch);
    let Some((_, ResumeOutcome::Failure(VmError::Thrown(returned)))) = failure.rejected_parts()
    else {
        panic!("malformed thrown input is returned unchanged")
    };
    assert_eq!(
        returned
            .vm_local_slot()
            .and_then(|slot| slot.as_request_heap_ref())
            .map(VmHandle::get),
        Some(handle)
    );
    let mut roots = IntrinsicRootHandles::default();
    failure.visit_roots(&mut roots).unwrap();
    assert!(roots.0.is_empty());

    let (primary, mut escrow) = fiber.escrow_rejected_resume(failure);
    assert_eq!(primary.diagnostic(), Some(&VmError::ResumeTokenMismatch));
    assert!(escrow.is_empty());
    assert_eq!(escrow.unresolved_count(), 0);
    escrow.release_all(&mut heap).unwrap();
    assert_eq!(heap.owner_count(handle), 1);

    fiber
        .discard_terminal_roots(&mut heap)
        .expect("the rejected receiver has no independent damaged owners");
    drop(escrow);
    heap.entries.clear();
    assert!(heap.entries.is_empty());
}

#[test]
fn store_slot_changed_owner_stays_rooted_when_destination_release_fails() {
    let mut heap = IntrinsicDispatchHeap::default();
    let mut fiber = intrinsic_fiber(&mut heap);
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::StoreSlot, 4);

    let frame = fiber.current_frame().unwrap().clone();
    let decoded = fiber
        .function(frame.function())
        .unwrap()
        .instructions()
        .get(frame.instruction().get() as usize)
        .unwrap();
    let LinkedInstructionTarget::FrameSlot(destination) = fiber
        .resolved_target(frame.function(), frame.instruction(), decoded, 0)
        .unwrap()
    else {
        panic!("StoreSlot carries an exact destination slot")
    };
    let slot_count = fiber
        .function(frame.function())
        .unwrap()
        .frame()
        .slot_types()
        .len();
    let destination_index =
        VmFiber::slot_index(&frame, slot_count, destination, frame.function()).unwrap();
    let previous = fiber.values[destination_index];
    let previous_handle = previous.as_request_heap_ref().unwrap().get();
    let operand_index = frame.operand_base() + frame.operand_height() - 1;
    assert!(matches!(
        fiber.values[operand_index].kind(),
        Some(skiff_runtime_model::vm_value::ValueKind::RequestHeapRef)
    ));
    let incoming = fiber.values[operand_index];

    heap.change_transfer_at = Some(heap.transfer_attempts + 1);
    heap.fail_release_at = Some(heap.release_attempts + 1);
    let mut budget = ResumeBudget;
    let VmControl::Complete(completion) = fiber.run_segment(&mut heap, &mut budget) else {
        panic!("destination cleanup failure must terminalize StoreSlot")
    };
    assert!(matches!(
        completion.failure(),
        Some(VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        }))
    ));
    assert_eq!(fiber.state(), VmFiberState::Terminal);
    assert_eq!(heap.owner_count(previous_handle), 1);
    let incoming_handle = incoming.as_request_heap_ref().unwrap().get();
    assert!(heap.entries.contains_key(&incoming_handle));

    let mut roots = IntrinsicRootHandles::default();
    completion.visit_roots(&mut roots).unwrap();
    assert!(roots.0.contains(&previous_handle));
    assert!(roots.0.contains(&incoming_handle));
    let mut slots = ResumeRootCollector::default();
    completion.visit_roots(&mut slots).unwrap();
    assert!(slots.0.iter().any(|slot| {
        slot.as_request_heap_ref()
            .is_some_and(|handle| handle.get() == incoming_handle)
            && slot.flags() != ValueFlags::new(0)
    }));

    let (Some(mut cause), mut escrow) = completion.into_terminal() else {
        panic!("failed completion retains its rootless primary diagnostic")
    };
    assert_eq!(cause.root_count(), 0);
    heap.fail_release_at = None;
    cause.release_all(&mut heap).unwrap();
    escrow
        .release_all(&mut heap)
        .expect("exact slot and operand plans drain after the release retry");
    assert!(heap.entries.is_empty());
}

#[test]
fn root_return_frame_exit_retries_first_middle_and_last_release_from_terminal_escrow() {
    for release_selector in 0..3 {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = intrinsic_fiber(&mut heap);
        drive_to_root_return(&mut fiber, &mut heap);
        let owner_total = heap_owner_total(&heap);
        assert!(owner_total >= 3, "fixture must retain three frame owners");
        let release_offset = match release_selector {
            0 => 1,
            1 => (owner_total + 1) / 2,
            _ => owner_total,
        };
        heap.fail_release_at = Some(heap.release_attempts + release_offset);
        let mut budget = ResumeBudget;

        let VmControl::Complete(completion) = fiber.run_segment(&mut heap, &mut budget) else {
            panic!("the selected Return frame release must fail")
        };

        assert!(matches!(
            completion.failure(),
            Some(VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            }))
        ));
        assert_eq!(fiber.state(), VmFiberState::Terminal);
        assert!(fiber.frames.is_empty());
        assert_eq!(
            heap_owner_total(&heap),
            owner_total - (release_offset - 1),
            "only the successful frame-release prefix is consumed"
        );
        let mut roots = IntrinsicRootHandles::default();
        completion.visit_roots(&mut roots).unwrap();
        assert_eq!(roots.0.len(), heap_owner_total(&heap));

        let (Some(mut cause), mut escrow) = completion.into_terminal() else {
            panic!("failed Return completion retains its diagnostic")
        };
        heap.fail_release_at = None;
        cause.release_all(&mut heap).unwrap();
        escrow
            .release_all(&mut heap)
            .expect("terminal carrier resumes the remaining frame releases");
        assert!(heap.entries.is_empty());
    }
}

#[test]
fn throw_frame_exit_failure_keeps_the_envelope_and_remaining_frame_owners_rooted() {
    const SOURCE: &str = "type Held { marker: number }\n\
         type Leaf { marker: number }\n\
         function run() -> number {\n\
           final first = Held { marker: 1 }\n\
           final second = Held { marker: 2 }\n\
           final third = Held { marker: 3 }\n\
           throw Leaf { marker: 4 }\n\
         }";
    for release_selector in 0..3 {
        let fixture = ObservationFixture::build(
            &format!("example.com/fiber-throw-transaction-{release_selector}"),
            SOURCE,
        );
        let observer = BytecodeExecutionObserver::new(
            Arc::new(RecordingSink::default()),
            observation_correlation(),
        );
        let mut fiber = fixture.start(vm_limits(), observer, Box::<[ValueSlot]>::default());
        fiber.set_error_correlation(ErrorCorrelation {
            trace_id: "throw-transaction-trace".to_string(),
            error_id: format!("throw-transaction-error-{release_selector}"),
        });
        let mut heap = IntrinsicDispatchHeap::default();
        drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::Throw, 1);
        let frame = fiber.current_frame().unwrap().clone();
        let payload_index = frame.operand_base() + frame.operand_height() - 1;
        let frame_release_count = fiber
            .values
            .iter()
            .enumerate()
            .filter(|(index, value)| {
                *index != payload_index
                    && fiber.live_values[*index]
                    && matches!(
                        value.kind(),
                        Some(
                            skiff_runtime_model::vm_value::ValueKind::RequestHeapRef
                                | skiff_runtime_model::vm_value::ValueKind::ResourceRef
                        )
                    )
            })
            .count();
        assert!(frame_release_count >= 3);
        let release_offset = match release_selector {
            0 => 1,
            1 => (frame_release_count + 1) / 2,
            _ => frame_release_count,
        };
        let owner_total = heap_owner_total(&heap);
        heap.fail_release_at = Some(heap.release_attempts + release_offset);
        let mut budget = ResumeBudget;

        let VmControl::Complete(completion) = fiber.run_segment(&mut heap, &mut budget) else {
            panic!("the selected Throw frame release must fail")
        };

        assert!(matches!(
            completion.failure(),
            Some(VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            }))
        ));
        assert_eq!(fiber.state(), VmFiberState::Terminal);
        assert!(fiber.frames.is_empty());
        assert!(fiber.unwind.is_none());
        assert_eq!(heap_owner_total(&heap), owner_total - (release_offset - 1));
        let mut roots = IntrinsicRootHandles::default();
        completion.visit_roots(&mut roots).unwrap();
        assert_eq!(roots.0.len(), heap_owner_total(&heap));

        let (Some(mut cause), mut escrow) = completion.into_terminal() else {
            panic!("failed Throw completion retains its diagnostic")
        };
        heap.fail_release_at = None;
        cause.release_all(&mut heap).unwrap();
        escrow
            .release_all(&mut heap)
            .expect("terminal cleanup releases the envelope and frame suffix");
        assert!(heap.entries.is_empty());
    }
}

#[test]
fn caught_envelope_release_failure_survives_after_its_catch_slot_is_cleared() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-caught-envelope-transaction",
        "type Leaf { marker: number }\n\
         function run() -> number {\n\
           final attempted = catch<Leaf>(throw Leaf { marker: 1 })\n\
           return 1\n\
         }",
    );
    let observer = BytecodeExecutionObserver::new(
        Arc::new(RecordingSink::default()),
        observation_correlation(),
    );
    let mut fiber = fixture.start(vm_limits(), observer, Box::<[ValueSlot]>::default());
    fiber.set_error_correlation(ErrorCorrelation {
        trace_id: "caught-envelope-trace".to_string(),
        error_id: "caught-envelope-error".to_string(),
    });
    let mut heap = IntrinsicDispatchHeap::default();
    drive_to_opcode_occurrence(&mut fiber, &mut heap, Opcode::Return, 1);
    let caught = fiber
        .caught_exceptions
        .values()
        .next()
        .expect("the catch handler retains its opaque envelope")
        .clone();
    let caught_handle = caught
        .envelope
        .vm_local_slot()
        .unwrap()
        .as_handle()
        .unwrap();
    let live_frame_owners = fiber
        .values
        .iter()
        .zip(fiber.live_values.iter().copied())
        .filter(|(value, live)| {
            *live
                && matches!(
                    value.kind(),
                    Some(
                        skiff_runtime_model::vm_value::ValueKind::RequestHeapRef
                            | skiff_runtime_model::vm_value::ValueKind::ResourceRef
                    )
                )
        })
        .count();
    assert!(
        live_frame_owners >= 1,
        "the catch slot owns one shared record"
    );
    heap.fail_release_at = Some(heap.release_attempts + live_frame_owners + 1);
    let mut budget = ResumeBudget;

    let VmControl::Complete(completion) = fiber.run_segment(&mut heap, &mut budget) else {
        panic!("caught-envelope release must fail after frame slots drain")
    };

    assert!(matches!(
        completion.failure(),
        Some(VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        }))
    ));
    assert_eq!(fiber.state(), VmFiberState::Terminal);
    assert!(fiber.frames.is_empty());
    assert!(fiber.caught_exceptions.is_empty());
    let mut roots = IntrinsicRootHandles::default();
    completion.visit_roots(&mut roots).unwrap();
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|candidate| **candidate == caught_handle.get())
            .count(),
        1,
        "after catch-slot cleanup the retained envelope is the sole logical owner"
    );

    let (Some(mut cause), mut escrow) = completion.into_terminal() else {
        panic!("failed catch cleanup retains its diagnostic")
    };
    heap.fail_release_at = None;
    cause.release_all(&mut heap).unwrap();
    escrow
        .release_all(&mut heap)
        .expect("terminal cleanup retries the retained caught envelope");
    assert!(heap.entries.is_empty());
}
