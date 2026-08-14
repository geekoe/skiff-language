use skiff_artifact_model::{NativeValueAdapterRole, NativeValueLifecycleAdapter};

use crate::{
    CandidateLocation, CandidateReferenceKind, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts, LinkedCallableSignature, LinkedNativeCallableSignature,
    LinkedResourceDropPlan, LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
};

use super::check_index;

pub(super) fn validate_callable_signature(
    signature: &LinkedCallableSignature,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_signature_parts(
        signature.parameter_types(),
        signature.parameter_plans(),
        signature.result_types(),
        signature.result_plans(),
        location,
        parts,
    )
}

pub(super) fn validate_native_signature(
    signature: &LinkedNativeCallableSignature,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    validate_signature_parts(
        signature.parameter_types(),
        signature.parameter_plans(),
        signature.result_types(),
        signature.result_plans(),
        location,
        parts,
    )
}

fn validate_signature_parts(
    parameter_types: &[crate::TypeIndex],
    parameter_plans: &[LinkedValueTransferPlan],
    result_types: &[crate::TypeIndex],
    result_plans: &[LinkedValueTransferPlan],
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    for (ty, plan) in parameter_types
        .iter()
        .zip(parameter_plans)
        .chain(result_types.iter().zip(result_plans))
    {
        validate_type_plan(*ty, plan, location, parts)?;
    }
    Ok(())
}

pub(super) fn validate_type_plan(
    ty: TypeIndex,
    plan: &LinkedValueTransferPlan,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    check_index(
        location,
        CandidateReferenceKind::Type,
        ty.get(),
        parts.types.len(),
    )?;
    validate_plan(plan, location, parts)?;
    if parts.types[ty.get() as usize].plan() != plan {
        return Err(LinkedBytecodeCandidateError::TypePlanMismatch {
            location,
            type_index: ty,
        });
    }
    Ok(())
}

pub(super) fn validate_plan(
    plan: &LinkedValueTransferPlan,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    match plan {
        LinkedValueTransferPlan::SnapshotShare { drop }
        | LinkedValueTransferPlan::MoveOnly { drop } => validate_value_drop(drop, location, parts),
        LinkedValueTransferPlan::AffineResource { drop } => {
            validate_resource_drop(drop, location, parts)
        }
        LinkedValueTransferPlan::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => {
            validate_adapter(clone_adapter, NativeValueAdapterRole::CloneLease, location)?;
            validate_resource_drop(drop, location, parts)
        }
    }
}

fn validate_value_drop(
    drop: &LinkedValueDropPlan,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    match drop {
        LinkedValueDropPlan::Trivial | LinkedValueDropPlan::SnapshotRelease => Ok(()),
        LinkedValueDropPlan::RecursiveShape { shape } => check_index(
            location,
            CandidateReferenceKind::Shape,
            shape.get(),
            parts.shapes.len(),
        ),
        LinkedValueDropPlan::NativeAdapter { adapter } => {
            validate_adapter(adapter, NativeValueAdapterRole::ValueDrop, location)
        }
    }
}

fn validate_resource_drop(
    drop: &LinkedResourceDropPlan,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    match drop {
        LinkedResourceDropPlan::ResourceTableRelease => Ok(()),
        LinkedResourceDropPlan::RecursiveShape { shape } => check_index(
            location,
            CandidateReferenceKind::Shape,
            shape.get(),
            parts.shapes.len(),
        ),
        LinkedResourceDropPlan::NativeAdapter { adapter } => {
            validate_adapter(adapter, NativeValueAdapterRole::ResourceDrop, location)
        }
    }
}

fn validate_adapter(
    adapter: &NativeValueLifecycleAdapter,
    expected: NativeValueAdapterRole,
    location: CandidateLocation,
) -> Result<(), LinkedBytecodeCandidateError> {
    if adapter.binding_key.is_empty() {
        return Err(LinkedBytecodeCandidateError::EmptyLifecycleAdapterBindingKey { location });
    }
    if adapter.abi_version == 0 {
        return Err(
            LinkedBytecodeCandidateError::ZeroLifecycleAdapterAbiVersion {
                location,
                binding_key: adapter.binding_key.clone(),
            },
        );
    }
    if adapter.role != expected {
        return Err(LinkedBytecodeCandidateError::LifecycleAdapterRoleMismatch {
            location,
            expected,
            actual: adapter.role,
        });
    }
    Ok(())
}
