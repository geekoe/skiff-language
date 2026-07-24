use skiff_artifact_model::{
    PackageUnit, RecoverableAdapterSchemaCompatibility, RecoverableBoundaryContext,
    RecoverableBoundaryKind, RecoverableBoundaryPlan, RecoverableCustomRestorePlan,
    RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot, RecoverableFieldIdentityRef,
    RecoverableInterfaceMethodIdentityFact, RecoverableInterfaceMethodIdentityRef,
    RecoverableInterfaceProjectionIdentityRef, RecoverableNativeAdapterOwner,
    RecoverableNativeAdapterPlan, RecoverableRestoreCapability, RecoverableStorageLane,
    RecoverableStorageLanePlan, RecoverableTrustBoundary, RecoverableTypeIdentityRef,
    RecoverableUnionBranchIdentityRef, TypeRefIr,
};

pub(super) fn recoverable_plan(reverse: bool) -> RecoverableExpectedTypePlan {
    let mut plan = RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeRef {
            ty: TypeRefIr::builtin("string"),
        },
        root_type_identity_ref: Some(RecoverableTypeIdentityRef("type:string".to_string())),
        runtime_carrier_check_required: true,
        interface_projection_refs: vec![
            RecoverableInterfaceProjectionIdentityRef("projection:a".to_string()),
            RecoverableInterfaceProjectionIdentityRef("projection:b".to_string()),
        ],
        interface_method_refs: vec![
            RecoverableInterfaceMethodIdentityRef("method:a".to_string()),
            RecoverableInterfaceMethodIdentityRef("method:b".to_string()),
        ],
        field_refs: vec![
            RecoverableFieldIdentityRef("field:a".to_string()),
            RecoverableFieldIdentityRef("field:b".to_string()),
        ],
        union_branch_refs: vec![
            RecoverableUnionBranchIdentityRef("union:a".to_string()),
            RecoverableUnionBranchIdentityRef("union:b".to_string()),
        ],
    };
    if reverse {
        plan.interface_projection_refs.reverse();
        plan.interface_method_refs.reverse();
        plan.field_refs.reverse();
        plan.union_branch_refs.reverse();
    }
    plan
}

pub(super) fn install_recoverable_plans(unit: &mut PackageUnit, plan: RecoverableExpectedTypePlan) {
    unit.recoverable_metadata
        .identity_tables
        .interface_methods
        .insert(
            "method".to_string(),
            RecoverableInterfaceMethodIdentityFact {
                interface_projection_ref: RecoverableInterfaceProjectionIdentityRef(
                    "projection:owner".to_string(),
                ),
                method_name: "call".to_string(),
                method_abi_id: Some("method:call".to_string()),
                signature: Some(plan.clone()),
            },
        );
    unit.recoverable_metadata.boundary_plans.insert(
        "boundary".to_string(),
        RecoverableBoundaryPlan {
            context: RecoverableBoundaryContext {
                boundary_kind: RecoverableBoundaryKind::DbPayload,
                trust_boundary: RecoverableTrustBoundary::OwnerInternal,
                origin_service: None,
                target_service: None,
                explicit_recoverable_slot: true,
            },
            expected_type: plan.clone(),
            runtime_carrier_check_required: true,
            storage_lane_ref: None,
            custom_restore_plan_ref: None,
            native_adapter_plan_ref: None,
        },
    );
    unit.recoverable_metadata.storage_lanes.insert(
        "lane".to_string(),
        RecoverableStorageLanePlan {
            lane: RecoverableStorageLane::RecoverableEnvelope,
            expected_type: Some(plan.clone()),
            schema_projection_ref: None,
            envelope_slot_ref: Some("slot".to_string()),
        },
    );
    unit.recoverable_metadata.custom_restore_plans.insert(
        "custom".to_string(),
        RecoverableCustomRestorePlan {
            concrete_type_identity: "type:custom".to_string(),
            durable_state_type_plan: plan.clone(),
            encode_hook_id: "encode".to_string(),
            decode_hook_id: "decode".to_string(),
            restore_capability: RecoverableRestoreCapability::Exact,
        },
    );
    unit.recoverable_metadata.native_adapter_plans.insert(
        "native".to_string(),
        RecoverableNativeAdapterPlan {
            adapter_identity: "adapter:native".to_string(),
            adapter_schema_version: "v1".to_string(),
            native_type_identity: "native:type".to_string(),
            durable_state_type_plan: plan,
            encode_hook_id: "native-encode".to_string(),
            decode_hook_id: "native-decode".to_string(),
            owner: RecoverableNativeAdapterOwner {
                service_identity: "service:owner".to_string(),
            },
            schema_compatibility: RecoverableAdapterSchemaCompatibility::Exact,
        },
    );
}
