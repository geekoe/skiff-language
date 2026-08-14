use skiff_artifact_model::{
    BytecodePoolEntry, BytecodePools, CallLoanBinding, CallLoanLayout, FrameLayout,
    PackageCallableParameter, PackageTypeRef, ParamModeIr, ParameterSlotDecl, ResourceDropPlan,
    ValueDropPlan, ValueTransferPlan, WritablePathDeclaration,
};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, ArtifactWritablePathIndex, CallLoanLayoutIndex, FrameSlotIndex,
    FunctionIndex, LinkedArtifactPoolOrigin, LinkedBytecodeCandidateParts, LinkedCallLoanBinding,
    LinkedCallLoanLayout, LinkedFrameLayout, LinkedInstruction, LinkedInstructionTarget,
    LinkedParameterSlot, LinkedResolvedOperand, LinkedResourceDropPlan, LinkedSlotState,
    LinkedStackValue, LinkedTypeEntry, LinkedValueDropPlan, LinkedValueTransferPlan,
    LinkedWritablePathEntry, TypeIndex, WritablePathIndex,
};

use super::{EffectGraphCallKind, EffectGraphFunction};

pub(super) fn artifact_pools(
    mut pools: BytecodePools,
    functions: &[EffectGraphFunction],
) -> BytecodePools {
    if functions.iter().any(|function| {
        matches!(function.call_kind, EffectGraphCallKind::InOut)
            || function.call_kind.is_stream_read()
            || matches!(
                function.call_kind,
                EffectGraphCallKind::Resume | EffectGraphCallKind::StreamProducer
            )
    }) {
        pools.types.push(BytecodePoolEntry::TypeRef {
            ty: skiff_artifact_model::TypeRefIr::builtin("bytes"),
        });
        if functions
            .iter()
            .any(|function| matches!(function.call_kind, EffectGraphCallKind::InOut))
        {
            pools
                .writable_paths
                .push(BytecodePoolEntry::WritablePath(WritablePathDeclaration {
                    root_type_ref: 0,
                    leaf_type_ref: 0,
                    segments: Vec::new(),
                }));
        }
        if functions.iter().any(|function| {
            function.call_kind.is_stream_read()
                || matches!(
                    function.call_kind,
                    EffectGraphCallKind::Resume | EffectGraphCallKind::StreamProducer
                )
        }) {
            pools
                .types
                .push(BytecodePoolEntry::TypeRef { ty: stream_type() });
        }
    }
    pools
}

pub(super) fn artifact_frame(kind: EffectGraphCallKind) -> FrameLayout {
    if kind.is_stream_read() {
        return FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![1],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: ParamModeIr::Value,
                plan: artifact_stream_plan(),
            }],
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: vec![artifact_stream_plan()],
        };
    }
    if matches!(kind, EffectGraphCallKind::Resume) {
        return FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: Some(1),
            slot_plans: Vec::new(),
        };
    }
    if matches!(kind, EffectGraphCallKind::StreamProducer) {
        return FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![0],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: ParamModeIr::Value,
                plan: artifact_plan(),
            }],
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: Some(1),
            slot_plans: vec![artifact_plan()],
        };
    }
    if matches!(kind, EffectGraphCallKind::InOut) {
        return FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![0],
            parameter_slots: Vec::new(),
            writable_local_slots: vec![0],
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
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
        stream_result_type_ref: None,
        slot_plans: Vec::new(),
    }
}

pub(super) fn package_parameters(kind: EffectGraphCallKind) -> Vec<PackageCallableParameter> {
    if matches!(kind, EffectGraphCallKind::StreamProducer) {
        return vec![PackageCallableParameter {
            name: "item".to_string(),
            ty: PackageTypeRef::Local {
                local_type: skiff_artifact_model::TypeRefIr::builtin("bytes"),
            },
            mode: ParamModeIr::Value,
        }];
    }
    if !kind.is_stream_read() {
        return Vec::new();
    }
    vec![PackageCallableParameter {
        name: "stream".to_string(),
        ty: PackageTypeRef::Local {
            local_type: stream_type(),
        },
        mode: ParamModeIr::Value,
    }]
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
    if kind.is_stream_read() {
        let plan = linked_stream_plan();
        return LinkedFrameLayout::new(
            Box::new([TypeIndex::new(1)]),
            Box::new([LinkedParameterSlot::new(
                FrameSlotIndex::new(0),
                ParamModeIr::Value,
                plan.clone(),
            )]),
            Box::new([]),
            Box::new([]),
            Box::new([plan]),
            Box::new([]),
            None,
        )
        .unwrap();
    }
    if matches!(kind, EffectGraphCallKind::Resume) {
        return LinkedFrameLayout::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Some(TypeIndex::new(1)),
        )
        .unwrap();
    }
    if matches!(kind, EffectGraphCallKind::StreamProducer) {
        let plan = linked_plan();
        return LinkedFrameLayout::new(
            Box::new([TypeIndex::new(0)]),
            Box::new([LinkedParameterSlot::new(
                FrameSlotIndex::new(0),
                ParamModeIr::Value,
                plan.clone(),
            )]),
            Box::new([]),
            Box::new([]),
            Box::new([plan]),
            Box::new([]),
            Some(TypeIndex::new(1)),
        )
        .unwrap();
    }
    if matches!(kind, EffectGraphCallKind::InOut) {
        let plan = linked_plan();
        return LinkedFrameLayout::new(
            Box::new([TypeIndex::new(0)]),
            Box::new([]),
            Box::new([FrameSlotIndex::new(0)]),
            Box::new([]),
            Box::new([plan]),
            Box::new([]),
            None,
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
        None,
    )
    .unwrap()
}

pub(super) fn linked_slot_states(kind: EffectGraphCallKind) -> Box<[LinkedSlotState]> {
    match kind {
        EffectGraphCallKind::InOut => Box::new([LinkedSlotState::Uninitialized]),
        EffectGraphCallKind::StreamProducer => Box::new([LinkedSlotState::Live(
            LinkedStackValue::new(TypeIndex::new(0), linked_plan()),
        )]),
        EffectGraphCallKind::StreamRead | EffectGraphCallKind::StreamReadTwice => {
            Box::new([LinkedSlotState::Live(LinkedStackValue::new(
                TypeIndex::new(1),
                linked_stream_plan(),
            ))])
        }
        EffectGraphCallKind::Ordinary | EffectGraphCallKind::Tail | EffectGraphCallKind::Resume => {
            Box::new([])
        }
    }
}

pub(super) const fn slot_count(kind: EffectGraphCallKind) -> usize {
    match kind {
        EffectGraphCallKind::InOut
        | EffectGraphCallKind::StreamProducer
        | EffectGraphCallKind::StreamRead
        | EffectGraphCallKind::StreamReadTwice => 1,
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
    let needs_types = functions.iter().any(|function| {
        matches!(function.call_kind, EffectGraphCallKind::InOut)
            || function.call_kind.is_stream_read()
            || matches!(
                function.call_kind,
                EffectGraphCallKind::Resume | EffectGraphCallKind::StreamProducer
            )
    });
    if !needs_types {
        return;
    }
    parts.types.push(LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(0), None).unwrap(),
        skiff_artifact_model::TypeRefIr::builtin("bytes"),
        None,
    ));
    if functions.iter().any(|function| {
        function.call_kind.is_stream_read()
            || matches!(
                function.call_kind,
                EffectGraphCallKind::Resume | EffectGraphCallKind::StreamProducer
            )
    }) {
        parts.types.push(LinkedTypeEntry::new(
            TypeIndex::new(1),
            LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(1), None).unwrap(),
            stream_type(),
            None,
        ));
    }
    if let Some((ordinal, _)) = functions
        .iter()
        .enumerate()
        .find(|(_, function)| matches!(function.call_kind, EffectGraphCallKind::InOut))
    {
        let specialization = parts.functions[ordinal].key().clone();
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

pub(super) fn linked_item_plan() -> LinkedValueTransferPlan {
    linked_plan()
}

pub(super) fn linked_stream_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::AffineResource {
        drop: LinkedResourceDropPlan::ResourceTableRelease,
    }
}

pub(super) fn artifact_item_plan() -> ValueTransferPlan {
    artifact_plan()
}

fn artifact_stream_plan() -> ValueTransferPlan {
    ValueTransferPlan::AffineResource {
        drop: ResourceDropPlan::ResourceTableRelease,
    }
}

fn stream_type() -> skiff_artifact_model::TypeRefIr {
    skiff_artifact_model::TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![skiff_artifact_model::TypeRefIr::builtin("bytes")],
    }
}
