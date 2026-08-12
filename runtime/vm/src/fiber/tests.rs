use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, InstructionBoundaryIndex, InstructionIndex, LinkedCatchMatcher,
    LinkedExceptionRegion, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionEntry;
use skiff_runtime_model::bytecode_execution_observation::BytecodeExecutionObserver;
use skiff_runtime_model::vm_heap::VmHeap;
use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

use super::{
    catch_matches, comparable_equality, comparable_equality_with_string_resolver,
    find_exception_region, nominal_tag_index, opcode_supported, DispatchOutcome, Vm, VmFiber,
};
use crate::{VmError, VmLimits};

type VmStartFn =
    fn(
        DeploymentExecutionEntry,
        Box<[ValueSlot]>,
        VmLimits,
        BytecodeExecutionObserver,
    ) -> Result<VmFiber, VmError>;

#[test]
fn production_start_signature_requires_the_concrete_pinned_entry() {
    let entry: VmStartFn = Vm::start;

    let _ = entry;
}

#[test]
fn fiber_keeps_frame_and_values_out_of_the_managed_heap() {
    fn assert_root_source<T: skiff_runtime_model::vm_root::VmRootSource>() {}
    assert_root_source::<VmFiber>();
}

#[test]
fn opcode_dispatch_has_a_heap_port_but_no_budget_port() {
    let dispatch: fn(&mut VmFiber, &mut dyn VmHeap) -> Result<DispatchOutcome, VmError> =
        VmFiber::dispatch_one;

    let _ = dispatch;
}

#[test]
fn value_control_and_scalar_opcodes_are_supported() {
    for opcode in [
        Opcode::Const,
        Opcode::CopySlot,
        Opcode::StoreSlot,
        Opcode::Drop,
        Opcode::Dup,
        Opcode::LoadSlot,
        Opcode::TakeSlot,
        Opcode::Pop,
        Opcode::MoveSlot,
        Opcode::Jump,
        Opcode::JumpIfTrue,
        Opcode::JumpIfFalse,
        Opcode::BudgetCheckpoint,
        Opcode::CallLocal,
        Opcode::TailCallLocal,
        Opcode::Return,
        Opcode::Not,
        Opcode::Negate,
        Opcode::Add,
        Opcode::Subtract,
        Opcode::Multiply,
        Opcode::Divide,
        Opcode::Equal,
        Opcode::NotEqual,
        Opcode::LessThan,
        Opcode::LessOrEqual,
        Opcode::GreaterThan,
        Opcode::GreaterOrEqual,
    ] {
        assert!(opcode_supported(opcode), "{opcode:?} should be supported");
    }
}

#[test]
fn unsupported_opcodes_remain_fail_closed() {
    let opcode = Opcode::CallLocalInOut;
    assert!(
        !opcode_supported(opcode),
        "{opcode:?} should be unsupported"
    );
}

#[test]
fn production_opcode_families_are_dispatched() {
    for opcode in [
        Opcode::SwitchTag,
        Opcode::Trap,
        Opcode::CallService,
        Opcode::CallActor,
        Opcode::CallInterface,
        Opcode::InvokeHost,
        Opcode::InvokeIntrinsic,
        Opcode::MakeCallback,
        Opcode::InvokeCallback,
        Opcode::NewRecord,
        Opcode::GetDenseField,
        Opcode::SetWritablePath,
        Opcode::RepresentationWrap,
        Opcode::NewArrayBuilder,
        Opcode::ArrayBuilderPush,
        Opcode::FreezeArray,
        Opcode::ArrayGet,
        Opcode::ArrayPushOwned,
        Opcode::ArrayLen,
        Opcode::NewMapBuilder,
        Opcode::MapBuilderPut,
        Opcode::FreezeMap,
        Opcode::MapGet,
        Opcode::MapPutOwned,
        Opcode::MapLen,
        Opcode::MapEntryAt,
        Opcode::StreamNext,
        Opcode::EmitStream,
        Opcode::Throw,
        Opcode::Rethrow,
        Opcode::EnterRegion,
        Opcode::LeaveRegion,
        Opcode::InterfaceBoxLocal,
        Opcode::InterfaceBoxRemote,
    ] {
        assert!(opcode_supported(opcode), "{opcode:?} should be supported");
    }
}

#[test]
fn exception_region_selection_uses_the_innermost_matching_handler() {
    let outer = exception_region(0, 8, 20, TypeIndex::new(1), FrameSlotIndex::new(0));
    let inner = exception_region(2, 5, 30, TypeIndex::new(2), FrameSlotIndex::new(1));
    let regions = [outer.clone(), inner];

    assert_eq!(
        find_exception_region(&regions, InstructionIndex::new(3), Some(TypeIndex::new(2)))
            .map(|region| region.handler()),
        Some(InstructionIndex::new(30))
    );
    assert_eq!(
        find_exception_region(&regions, InstructionIndex::new(6), Some(TypeIndex::new(1)))
            .map(|region| region.handler()),
        Some(InstructionIndex::new(20))
    );
    assert_eq!(
        find_exception_region(&regions, InstructionIndex::new(3), Some(TypeIndex::new(9))),
        None
    );
}

#[test]
fn catch_all_and_exact_type_matchers_are_closed() {
    assert!(catch_matches(&LinkedCatchMatcher::CatchAll, None));
    assert!(!catch_matches(
        &LinkedCatchMatcher::Type(TypeIndex::new(3)),
        Some(TypeIndex::new(4))
    ));
    assert!(catch_matches(
        &LinkedCatchMatcher::Type(TypeIndex::new(3)),
        Some(TypeIndex::new(3))
    ));
}

#[test]
fn nominal_switch_tag_comes_from_reference_metadata_only() {
    assert_eq!(nominal_tag_index(&ValueSlot::number(1.0)), 0);
    assert_eq!(
        nominal_tag_index(&ValueSlot::request_heap_ref(
            skiff_runtime_model::vm_value::VmHandle::new(1),
            skiff_runtime_model::vm_value::CompactTypeTag::new(42),
            skiff_runtime_model::vm_value::ValueFlags::new(0),
        )),
        42
    );
}

fn exception_region(
    start: u32,
    end: u32,
    handler: u32,
    catch_type: TypeIndex,
    catch_slot: FrameSlotIndex,
) -> LinkedExceptionRegion {
    LinkedExceptionRegion::new(
        InstructionIndex::new(start),
        InstructionBoundaryIndex::new(end),
        InstructionIndex::new(handler),
        0,
        Box::new([LinkedCatchMatcher::Type(catch_type)]),
        catch_slot,
        catch_type,
        0,
    )
}

#[test]
fn comparable_equality_matches_same_kind_and_numeric_equality() {
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::integer(3)),
        Some(true)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::number(3.0)),
        Some(true)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::number(4.0)),
        Some(false)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::bool(false), &ValueSlot::bool(true)),
        Some(false)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::null(), &ValueSlot::null()),
        Some(true)
    );
}

#[test]
fn comparable_equality_resolves_const_and_request_heap_strings() {
    let const_left =
        ValueSlot::const_ref(VmHandle::new(1), CompactTypeTag::new(0), ValueFlags::new(0));
    let const_right =
        ValueSlot::const_ref(VmHandle::new(2), CompactTypeTag::new(0), ValueFlags::new(0));
    let heap_same =
        ValueSlot::request_heap_ref(VmHandle::new(3), CompactTypeTag::new(0), ValueFlags::new(0));
    let heap_different =
        ValueSlot::request_heap_ref(VmHandle::new(4), CompactTypeTag::new(0), ValueFlags::new(0));
    let resolve_string = |value: &ValueSlot| match value.as_handle()?.get() {
        1 | 2 | 3 => Some("same".to_string()),
        4 => Some("different".to_string()),
        _ => None,
    };

    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &const_right, resolve_string),
        Some(true)
    );
    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &heap_same, resolve_string),
        Some(true)
    );
    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &heap_different, resolve_string),
        Some(false)
    );
    assert_eq!(
        comparable_equality_with_string_resolver(&heap_same, &heap_same, resolve_string),
        Some(true)
    );

    let unresolved =
        ValueSlot::const_ref(VmHandle::new(9), CompactTypeTag::new(0), ValueFlags::new(0));
    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &unresolved, resolve_string),
        None
    );
}
