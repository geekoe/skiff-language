use skiff_artifact_model::Opcode;
use skiff_runtime_model::vm_value::ValueSlot;

use super::{opcode_execution_class, VerifiedVmEntry, Vm, VmFiber, VmOpcodeExecutionClass};
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
fn opcode_dispatch_has_no_budget_or_heap_port() {
    let dispatch: fn(&mut VmFiber) -> Result<(), VmError> = VmFiber::dispatch_one;

    let _ = dispatch;
}

#[test]
fn value_lifecycle_execution_paths_remain_fail_closed() {
    for opcode in [
        Opcode::Const,
        Opcode::CopySlot,
        Opcode::MoveSlot,
        Opcode::StoreSlot,
        Opcode::Drop,
        Opcode::Dup,
        Opcode::LoadSlot,
        Opcode::TakeSlot,
        Opcode::Pop,
        Opcode::Return,
    ] {
        assert_eq!(
            opcode_execution_class(opcode),
            VmOpcodeExecutionClass::RequiresFullValueLifecyclePlan
        );
    }
}

#[test]
fn local_and_tail_calls_remain_fail_closed_in_this_slice() {
    for opcode in [Opcode::CallLocal, Opcode::TailCallLocal, Opcode::Jump] {
        assert_eq!(
            opcode_execution_class(opcode),
            VmOpcodeExecutionClass::Unsupported
        );
    }
}
