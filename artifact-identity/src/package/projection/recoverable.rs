use skiff_artifact_model::{RecoverableArtifactMetadata, RecoverableExpectedTypePlan};

/// Normalizes only collections that the recoverable contract itself treats as
/// sets. Type arguments, function params, union descriptors and every other
/// semantically ordered vector remain untouched.
pub(super) fn canonical_recoverable_metadata(
    metadata: &RecoverableArtifactMetadata,
) -> RecoverableArtifactMetadata {
    let mut canonical = metadata.clone();

    for fact in canonical.identity_tables.interface_methods.values_mut() {
        if let Some(signature) = &mut fact.signature {
            canonical_expected_type_plan(signature);
        }
    }
    for plan in canonical.boundary_plans.values_mut() {
        canonical_expected_type_plan(&mut plan.expected_type);
    }
    for plan in canonical.storage_lanes.values_mut() {
        if let Some(expected_type) = &mut plan.expected_type {
            canonical_expected_type_plan(expected_type);
        }
    }
    for plan in canonical.custom_restore_plans.values_mut() {
        canonical_expected_type_plan(&mut plan.durable_state_type_plan);
    }
    for plan in canonical.native_adapter_plans.values_mut() {
        canonical_expected_type_plan(&mut plan.durable_state_type_plan);
    }

    canonical
}

fn canonical_expected_type_plan(plan: &mut RecoverableExpectedTypePlan) {
    plan.interface_projection_refs
        .sort_by(|left, right| left.0.cmp(&right.0));
    plan.interface_projection_refs.dedup();
    plan.interface_method_refs
        .sort_by(|left, right| left.0.cmp(&right.0));
    plan.interface_method_refs.dedup();
    plan.field_refs.sort_by(|left, right| left.0.cmp(&right.0));
    plan.field_refs.dedup();
    plan.union_branch_refs
        .sort_by(|left, right| left.0.cmp(&right.0));
    plan.union_branch_refs.dedup();
}
