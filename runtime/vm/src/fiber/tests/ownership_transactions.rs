use super::intrinsic_dispatch::*;
use super::*;
use crate::control::VmLifecycleSite;
use crate::{VmOwnedValues, VmTerminalEscrow};

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
    assert!(matches!(
        fiber.run_segment(&mut heap, &mut budget),
        VmControl::Complete(Err(VmError::FiberNotRunnable {
            state: VmFiberState::Terminal
        }))
    ));
    assert_eq!(heap.typed_bytes_allocations, allocations_after_failure);
    heap.fail_typed_bytes_at = None;
    fiber
        .discard_terminal_roots(&mut heap)
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
        assert!(matches!(
            fiber.run_segment(&mut heap, &mut budget),
            VmControl::Complete(Err(VmError::FiberNotRunnable {
                state: VmFiberState::Terminal
            }))
        ));
        assert_eq!(heap.release_attempts, releases_after_failure);

        heap.fail_release_at = None;
        fiber
            .discard_terminal_roots(&mut heap)
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

    let (primary, mut escrow) = fiber.escrow_abandoned_completion(Ok(values));

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
    let (mut fiber, completed, exception_pointer) = origin_throw_completion(&fixture, &mut heap);

    let (Some(mut cause), escrow) = fiber.escrow_abandoned_completion(completed) else {
        panic!("uncaught Throw yields one owner-aware terminal cause")
    };

    assert!(escrow.is_empty());
    assert_eq!(cause.root_count(), 1);
    assert_eq!(cause.unresolved_count(), 0);
    assert_eq!(
        cause.thrown().map(|exception| exception as *const _),
        Some(exception_pointer)
    );
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
    assert_eq!(
        cause.thrown().map(|exception| exception as *const _),
        Some(exception_pointer),
        "cleanup failure preserves the same primary diagnostic carrier"
    );
    assert_eq!(heap_owner_total(&heap), 1);

    heap.fail_release_at = None;
    cause
        .release_all(&mut heap)
        .expect("retry consumes the exact same owner once");
    assert_eq!(cause.root_count(), 0);
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
fn raw_thrown_failure_is_rejected_unchanged_and_retained_as_damaged() {
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
    assert_eq!(roots.0, vec![handle]);

    let (primary, mut escrow) = fiber.escrow_rejected_resume(failure);
    assert_eq!(primary.diagnostic(), Some(&VmError::ResumeTokenMismatch));
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(escrow.unresolved_count(), 1);
    assert!(matches!(
        escrow.release_all(&mut heap),
        Err(VmError::TerminalRootLifecycleUnavailable { index: 0, .. })
    ));
    assert_eq!(escrow.root_count(), 1);
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

    heap.change_transfer_at = Some(heap.transfer_attempts + 1);
    heap.fail_release_at = Some(heap.release_attempts + 1);
    let mut budget = ResumeBudget;
    let VmControl::Complete(Err(error)) = fiber.run_segment(&mut heap, &mut budget) else {
        panic!("destination cleanup failure must terminalize StoreSlot")
    };
    assert!(matches!(
        error,
        VmError::Heap(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(fiber.state(), VmFiberState::Terminal);
    assert!(fiber.live_values[destination_index]);
    assert!(fiber.values[destination_index] == previous);
    assert_eq!(heap.owner_count(previous_handle), 1);
    assert!(fiber.live_values[operand_index]);
    let incoming = fiber.values[operand_index];
    assert_ne!(incoming.flags(), ValueFlags::new(0));
    assert!(heap
        .entries
        .contains_key(&incoming.as_request_heap_ref().unwrap().get()));

    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert!(roots.0.contains(&previous_handle));
    assert!(roots
        .0
        .contains(&incoming.as_request_heap_ref().unwrap().get()));

    heap.fail_release_at = None;
    fiber
        .discard_terminal_roots(&mut heap)
        .expect("exact slot and operand plans drain after the release retry");
    assert!(heap.entries.is_empty());
}

#[test]
fn root_return_frame_exit_retries_first_middle_and_last_release_from_terminal_escrow() {
    for release_selector in 0..3 {
        let mut heap = IntrinsicDispatchHeap::default();
        let mut fiber = intrinsic_fiber(&mut heap);
        drive_to_root_return(&mut fiber, &mut heap);
        let frame = fiber.current_frame().unwrap().clone();
        let instruction = frame.instruction();
        let owner_total = heap_owner_total(&heap);
        assert!(owner_total >= 3, "fixture must retain three frame owners");
        let release_offset = match release_selector {
            0 => 1,
            1 => (owner_total + 1) / 2,
            _ => owner_total,
        };
        heap.fail_release_at = Some(heap.release_attempts + release_offset);
        let mut budget = ResumeBudget;

        let VmControl::Complete(Err(error)) = fiber.run_segment(&mut heap, &mut budget) else {
            panic!("the selected Return frame release must fail")
        };

        assert!(matches!(
            error,
            VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            })
        ));
        assert_eq!(fiber.state(), VmFiberState::Terminal);
        assert_eq!(fiber.current_frame().unwrap().instruction(), instruction);
        assert_eq!(
            heap_owner_total(&heap),
            owner_total - (release_offset - 1),
            "only the successful frame-release prefix is consumed"
        );
        let mut roots = IntrinsicRootHandles::default();
        fiber.visit_roots(&mut roots).unwrap();
        assert_eq!(roots.0.len(), heap_owner_total(&heap));

        heap.fail_release_at = None;
        fiber
            .discard_terminal_roots(&mut heap)
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
        let instruction = frame.instruction();
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

        let VmControl::Complete(Err(error)) = fiber.run_segment(&mut heap, &mut budget) else {
            panic!("the selected Throw frame release must fail")
        };

        assert!(matches!(
            error,
            VmError::Heap(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            })
        ));
        assert_eq!(fiber.state(), VmFiberState::Terminal);
        assert_eq!(fiber.current_frame().unwrap().instruction(), instruction);
        assert!(
            fiber.unwind.is_some(),
            "the opaque envelope remains installed"
        );
        assert_eq!(heap_owner_total(&heap), owner_total - (release_offset - 1));
        let mut roots = IntrinsicRootHandles::default();
        fiber.visit_roots(&mut roots).unwrap();
        assert_eq!(roots.0.len(), heap_owner_total(&heap));

        heap.fail_release_at = None;
        fiber
            .discard_terminal_roots(&mut heap)
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
    let frame = fiber.current_frame().unwrap().clone();
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

    let VmControl::Complete(Err(error)) = fiber.run_segment(&mut heap, &mut budget) else {
        panic!("caught-envelope release must fail after frame slots drain")
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
        fiber.current_frame().unwrap().instruction(),
        frame.instruction()
    );
    assert_eq!(fiber.caught_exceptions.len(), 1);
    let mut roots = IntrinsicRootHandles::default();
    fiber.visit_roots(&mut roots).unwrap();
    assert_eq!(
        roots
            .0
            .iter()
            .filter(|candidate| **candidate == caught_handle.get())
            .count(),
        1,
        "after catch-slot cleanup the retained envelope is the sole logical owner"
    );

    heap.fail_release_at = None;
    fiber
        .discard_terminal_roots(&mut heap)
        .expect("terminal cleanup retries the retained caught envelope");
    assert!(heap.entries.is_empty());
}
