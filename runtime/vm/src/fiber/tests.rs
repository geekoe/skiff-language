use skiff_artifact_model::Opcode;
use skiff_runtime_model::vm_value::ValueSlot;

use super::{comparable_equality, opcode_supported, DispatchOutcome, VerifiedVmEntry, Vm, VmFiber};
use crate::{VmError, VmLimits};

type VmStartFn = fn(VerifiedVmEntry, Box<[ValueSlot]>, VmLimits) -> Result<VmFiber, VmError>;

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
fn opcode_dispatch_still_has_no_budget_or_heap_port() {
    let dispatch: fn(&mut VmFiber) -> Result<DispatchOutcome, VmError> = VmFiber::dispatch_one;

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
    for opcode in [
        Opcode::SwitchTag,
        Opcode::CallService,
        Opcode::NewRecord,
        Opcode::InvokeHost,
        Opcode::InvokeIntrinsic,
    ] {
        assert!(
            !opcode_supported(opcode),
            "{opcode:?} should be unsupported"
        );
    }
}

#[test]
fn comparable_equality_matches_only_same_immediate_kind() {
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::integer(3)),
        Some(true)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::number(3.0)),
        None
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
