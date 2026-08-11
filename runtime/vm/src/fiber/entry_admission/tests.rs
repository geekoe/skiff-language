use skiff_artifact_model::{CallableEffectSummary, ParamModeIr};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, LinkedCallableSignature, LinkedFrameLayout, LinkedParameterSlot,
    LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
};

use super::{validate_frame_shape, validate_signature_shape};
use crate::{VmError, VmVerifiedInvariant};

#[test]
fn v4_parameter_admission_compares_the_complete_plan() {
    let frame = frame(
        ParamModeIr::Value,
        snapshot_plan(LinkedValueDropPlan::Trivial),
    );
    let signature = signature(
        ParamModeIr::Value,
        snapshot_plan(LinkedValueDropPlan::SnapshotRelease),
    );

    assert_eq!(
        validate_frame_shape(&frame, &signature),
        Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ParameterTransferPlan,
        })
    );
}

#[test]
fn v4_parameter_admission_accepts_exact_value_facts() {
    let plan = snapshot_plan(LinkedValueDropPlan::SnapshotRelease);
    let frame = frame(ParamModeIr::Value, plan.clone());
    let signature = signature(ParamModeIr::Value, plan);

    assert_eq!(validate_signature_shape(&signature, 1), Ok(()));
    assert_eq!(validate_frame_shape(&frame, &signature), Ok(()));
}

#[test]
fn v4_root_entry_rejects_inout_from_signature_or_frame() {
    let plan = snapshot_plan(LinkedValueDropPlan::Trivial);
    let inout_signature = signature(ParamModeIr::InOut, plan.clone());
    assert_eq!(
        validate_signature_shape(&inout_signature, 1),
        Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ExternalInOutParameter,
        })
    );

    let inout_frame = frame(ParamModeIr::InOut, plan.clone());
    let value_signature = signature(ParamModeIr::Value, plan);
    assert_eq!(
        validate_frame_shape(&inout_frame, &value_signature),
        Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::ExternalInOutParameter,
        })
    );
}

fn snapshot_plan(drop: LinkedValueDropPlan) -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare { drop }
}

fn signature(mode: ParamModeIr, plan: LinkedValueTransferPlan) -> LinkedCallableSignature {
    LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([mode]),
        Box::new([plan]),
        Box::new([]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect("test signature has one mode and plan per parameter")
}

fn frame(mode: ParamModeIr, plan: LinkedValueTransferPlan) -> LinkedFrameLayout {
    LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            mode,
            plan.clone(),
        )]),
        Box::new([]),
        Box::new([]),
        Box::new([plan]),
        Box::new([]),
        None,
    )
    .expect("test frame is locally well-shaped")
}
