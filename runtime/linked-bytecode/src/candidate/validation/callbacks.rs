use std::collections::BTreeSet;

use skiff_artifact_model::PackageBuildId;

use crate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts,
};

use super::{
    check_index, plans::validate_callable_signature, plans::validate_type_plan, validate_origin,
};

pub(super) fn validate_synthetic_callback(
    target: &crate::LinkedSyntheticCallbackTarget,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::SyntheticCallbacks,
        row: target.index().get(),
    };
    check_index(
        location,
        CandidateReferenceKind::Function,
        target.function().get(),
        parts.functions.len(),
    )?;
    if let Some(function) = parts.functions.get(target.function().get() as usize) {
        if function.key().artifact_function_key() != target.artifact_function_key() {
            return Err(
                LinkedBytecodeCandidateError::SyntheticCallbackFunctionMismatch {
                    callback: target.index(),
                    function: target.function(),
                },
            );
        }
    }
    if let Some(binding) = target.interface_method() {
        check_index(
            location,
            CandidateReferenceKind::InterfaceTable,
            binding.interface_table().get(),
            parts.interface_tables.len(),
        )?;
    }
    validate_callable_signature(target.signature(), location, parts)
}

pub(super) fn validate_capture_layout(
    layout: &crate::LinkedCallbackCaptureLayout,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::CallbackCaptureLayouts,
        row: layout.index().get(),
    };
    validate_origin(layout.origin(), location, parts, package_ids)?;
    check_index(
        location,
        CandidateReferenceKind::Function,
        layout.function().get(),
        parts.functions.len(),
    )?;
    let Some(function) = parts.functions.get(layout.function().get() as usize) else {
        return Ok(());
    };
    if function.key().artifact_function_key() != layout.artifact_function_key() {
        return Err(
            LinkedBytecodeCandidateError::CallbackCaptureFunctionMismatch {
                layout: layout.index(),
                function: layout.function(),
            },
        );
    }
    if layout.origin().specialization() != Some(function.key()) {
        return Err(
            LinkedBytecodeCandidateError::CallbackCaptureOriginMismatch {
                layout: layout.index(),
                function: layout.function(),
            },
        );
    }
    for capture in layout.captures() {
        check_index(
            location,
            CandidateReferenceKind::FrameSlot,
            capture.slot().get(),
            function.frame().slot_types().len(),
        )?;
        validate_type_plan(capture.ty(), capture.plan(), location, parts)?;
        let slot = capture.slot().get() as usize;
        if function.frame().slot_types().get(slot) != Some(&capture.ty()) {
            return Err(LinkedBytecodeCandidateError::CallbackCaptureTypeMismatch {
                layout: layout.index(),
                slot: capture.slot(),
            });
        }
        if function.frame().slot_plans().get(slot) != Some(capture.plan()) {
            return Err(LinkedBytecodeCandidateError::CallbackCapturePlanMismatch {
                layout: layout.index(),
                slot: capture.slot(),
            });
        }
    }
    Ok(())
}
