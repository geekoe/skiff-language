use skiff_artifact_model::{
    BytecodePoolEntry, BytecodePools, CallLoanBinding, CallLoanLayout, FrameLayout, ValueDropPlan,
    ValueTransferPlan, WritablePathDeclaration,
};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, ArtifactWritablePathIndex, CallLoanLayoutIndex, FrameSlotIndex,
    FunctionIndex, LinkedArtifactPoolOrigin, LinkedBytecodeCandidateParts, LinkedCallLoanBinding,
    LinkedCallLoanLayout, LinkedFrameLayout, LinkedInstruction, LinkedInstructionTarget,
    LinkedResolvedOperand, LinkedSlotState, LinkedTypeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan, LinkedWritablePathEntry, TypeIndex, WritablePathIndex,
};

use super::{EffectGraphCallKind, EffectGraphFunction};

pub(super) fn artifact_pools(
    mut pools: BytecodePools,
    functions: &[EffectGraphFunction],
) -> BytecodePools {
    if functions
        .iter()
        .any(|function| matches!(function.call_kind, EffectGraphCallKind::InOut))
    {
        pools.types.push(BytecodePoolEntry::TypeRef {
            ty: skiff_artifact_model::TypeRefIr::builtin("string"),
        });
        pools
            .writable_paths
            .push(BytecodePoolEntry::WritablePath(WritablePathDeclaration {
                root_type_ref: 0,
                leaf_type_ref: 0,
                segments: Vec::new(),
            }));
    }
    pools
}

pub(super) fn artifact_frame(kind: EffectGraphCallKind) -> FrameLayout {
    if matches!(kind, EffectGraphCallKind::InOut) {
        return FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![0],
            parameter_slots: Vec::new(),
            writable_local_slots: vec![0],
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: vec![artifact_plan()],
        };
    }
    FrameLayout {
        slot_count: 0,
        slot_type_refs: Vec::new(),
        parameter_slots: Vec::new(),
        writable_local_slots: Vec::new(),
        result_count: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        slot_plans: Vec::new(),
    }
}

pub(super) fn artifact_loan_layouts(kind: EffectGraphCallKind) -> Vec<CallLoanLayout> {
    if matches!(kind, EffectGraphCallKind::InOut) {
        return vec![CallLoanLayout {
            loans: vec![CallLoanBinding {
                parameter_ordinal: 0,
                root_slot: 0,
                writable_path_ref: 0,
            }],
        }];
    }
    Vec::new()
}

pub(super) fn linked_frame(kind: EffectGraphCallKind) -> LinkedFrameLayout {
    if matches!(kind, EffectGraphCallKind::InOut) {
        let plan = linked_plan();
        return LinkedFrameLayout::new(
            Box::new([TypeIndex::new(0)]),
            Box::new([]),
            Box::new([FrameSlotIndex::new(0)]),
            Box::new([]),
            Box::new([plan]),
            Box::new([]),
        )
        .unwrap();
    }
    LinkedFrameLayout::new(
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
    )
    .unwrap()
}

pub(super) fn linked_slot_states(kind: EffectGraphCallKind) -> Box<[LinkedSlotState]> {
    if matches!(kind, EffectGraphCallKind::InOut) {
        Box::new([LinkedSlotState::Uninitialized])
    } else {
        Box::new([])
    }
}

pub(super) const fn slot_count(kind: EffectGraphCallKind) -> usize {
    match kind {
        EffectGraphCallKind::InOut => 1,
        EffectGraphCallKind::Ordinary | EffectGraphCallKind::Tail | EffectGraphCallKind::Resume => {
            0
        }
    }
}

pub(super) fn linked_loan_layouts(kind: EffectGraphCallKind) -> Box<[LinkedCallLoanLayout]> {
    if matches!(kind, EffectGraphCallKind::InOut) {
        return Box::new([LinkedCallLoanLayout::try_new(
            CallLoanLayoutIndex::new(0),
            Box::new([LinkedCallLoanBinding::new(
                0,
                FrameSlotIndex::new(0),
                WritablePathIndex::new(0),
            )]),
        )
        .unwrap()]);
    }
    Box::new([])
}

pub(super) fn linked_call(target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::CallLocalInOut,
        Box::new([0, 0, 0, 0]),
        Box::new([
            LinkedResolvedOperand::new(
                0,
                LinkedInstructionTarget::Function(FunctionIndex::new(target)),
            ),
            LinkedResolvedOperand::new(
                3,
                LinkedInstructionTarget::CallLoanLayout(CallLoanLayoutIndex::new(0)),
            ),
        ]),
        0,
    )
    .unwrap()
}

pub(super) fn extend_linked_parts(
    parts: &mut LinkedBytecodeCandidateParts,
    build: &skiff_artifact_model::PackageBuildId,
    functions: &[EffectGraphFunction],
) {
    let Some((ordinal, _)) = functions
        .iter()
        .enumerate()
        .find(|(_, function)| matches!(function.call_kind, EffectGraphCallKind::InOut))
    else {
        return;
    };
    let specialization = parts.functions[ordinal].key().clone();
    parts.types.push(LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(0), None).unwrap(),
        skiff_artifact_model::TypeRefIr::builtin("string"),
        None,
    ));
    parts.writable_paths.push(
        LinkedWritablePathEntry::new(
            WritablePathIndex::new(0),
            LinkedArtifactPoolOrigin::new(
                build.clone(),
                ArtifactWritablePathIndex::new(0),
                Some(specialization),
            )
            .unwrap(),
            TypeIndex::new(0),
            TypeIndex::new(0),
            Box::new([]),
        )
        .unwrap(),
    );
}

fn artifact_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    }
}

fn linked_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::SnapshotRelease,
    }
}
