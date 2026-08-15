//! X6-owned local interface child leaf.
//!
//! Local interface is registered on the same flat child lifecycle as service.
//! Remote interface, callback and requirement tables remain disabled until
//! their capability lanes land.

use std::sync::Arc;

use skiff_runtime_linked_bytecode::{InterfaceTableIndex, LinkedInterfaceTableKind};
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver, vm_heap::VmHeap,
};
use skiff_runtime_scheduler::{
    BytecodeChildHandoff, BytecodePortFailure, BytecodeSchedulerError, RequestResourceTable,
};
use skiff_runtime_vm::{ChildInvocation, ChildTarget, VmBudget, VmFiber, VmLimits, VmResumeToken};

use super::{BytecodeChildHeapFactory, BytecodeRequestChildComposition};

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_interface_child(
    invocation: ChildInvocation,
    _heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    _composition: &BytecodeRequestChildComposition,
    _child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    _resources: RequestResourceTable,
    _observer: BytecodeExecutionObserver,
    _limits: VmLimits,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let ChildTarget::Interface {
        table,
        method_ordinal,
    } = invocation.target()
    else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    };

    let image = Arc::clone(invocation.resume().image());
    let Some(row) = interface_table_by_index(&image, table) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(interface_required_fact(
                "F6 linked LinkedInterfaceTableKind::Local rows",
            )),
            invocation,
        ));
    };

    match row.kind() {
        LinkedInterfaceTableKind::Local(local) => {
            let method_absent = local
                .methods()
                .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
                .filter(|method| method.method_slot() == method_ordinal)
                .is_none();
            if method_absent {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "local interface method row is absent from the linked table".to_string(),
                    ),
                    invocation,
                ));
            }
            Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(interface_required_fact(
                    "K6 heap-neutral local interface carrier read/materialization API",
                )),
                invocation,
            ))
        }
        LinkedInterfaceTableKind::Requirement(_)
        | LinkedInterfaceTableKind::Remote(_)
        | LinkedInterfaceTableKind::Callback(_) => Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        )),
    }
}

fn interface_table_by_index(
    image: &skiff_runtime_linker::DeploymentExecutionImage,
    table: InterfaceTableIndex,
) -> Option<&skiff_runtime_linked_bytecode::LinkedInterfaceTable> {
    let position = usize::try_from(table.get()).ok()?;
    image
        .interface_tables()
        .get(position)
        .filter(|row| row.index() == table)
}

pub(crate) fn interface_required_fact(seam: &str) -> String {
    format!(
        "local interface child requires {seam}; F6 must link \
         LinkedInterfaceTableKind::Local with concrete_type, method_abi_id, function and \
         receiver_call_abi; K6 must expose a heap-neutral local interface carrier \
         read/materialization API; X6 will then start the local method on the same flat \
         ChildHeapCarrier/ChildFinish lifecycle as service"
    )
}

#[cfg(test)]
mod tests {
    use super::super::BytecodeChildLane;
    use super::*;

    #[test]
    fn local_interface_registration_names_flat_child_lifecycle() {
        let required = interface_required_fact("K6 local interface carrier read");
        assert!(
            required.contains("ChildHeapCarrier/ChildFinish"),
            "local interface must register on the service flat child lifecycle: {required}"
        );
        assert!(
            required.contains("LinkedInterfaceTableKind::Local"),
            "local interface must require the F6 local table: {required}"
        );
    }

    #[test]
    fn disabled_targets_do_not_route_to_interface() {
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::StreamNext),
            BytecodeChildLane::Disabled
        );
    }
}
