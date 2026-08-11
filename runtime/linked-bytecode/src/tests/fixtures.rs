use std::collections::BTreeSet;

use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, host_effect_registry_identity,
    intrinsic_registry_identity, native_value_lifecycle_registry_identity,
    opcode_table_fingerprint, value_lifecycle_policy_identity, BytecodeArtifactRef,
    CallableEffectSummary, CallableMayEffects, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey,
    GatewayEntryProtocolSurface, GatewayExternalErrorProjection, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, Opcode, PackageBuildId, PackageCallableId, ParamModeIr,
    PlatformErrorProjectionRegistryRef, TypeRefIr, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, GATEWAY_ENTRY_IDENTITY_PREFIX,
};

use crate::{
    ArtifactFunctionKey, ArtifactTypeIndex, BytecodePackageIndex, FrameSlotIndex, FunctionIndex,
    LinkedArtifactPoolOrigin, LinkedBytecodeAuthorityPins, LinkedBytecodeCandidateParts,
    LinkedCallableEffectDeclaration, LinkedCallableSignature, LinkedFrameLayout, LinkedFunction,
    LinkedFunctionTables, LinkedGatewayCallable, LinkedGatewayCallableRole, LinkedInstruction,
    LinkedNativeCallableSignature, LinkedPackageBytecodeProvenance, LinkedParameterSlot,
    LinkedProgramPointState, LinkedSlotState, LinkedStackMapCandidate, LinkedStackValue,
    LinkedStatementEntry, LinkedTypeEntry, LinkedValueDropPlan, LinkedValueTransferPlan,
    SpecializationKey, TypeIndex,
};

pub(super) fn build_id() -> PackageBuildId {
    PackageBuildId::new("package-build:fixture")
}

pub(super) fn artifact_function_key(value: &str) -> ArtifactFunctionKey {
    ArtifactFunctionKey::parse(value).expect("fixture artifact function key is canonical")
}

pub(super) fn specialization(name: &str) -> SpecializationKey {
    specialization_for(build_id(), name, Box::new([]), None)
}

pub(super) fn specialization_for(
    package_build_id: PackageBuildId,
    name: &str,
    concrete_type_arguments: Box<[TypeIndex]>,
    concrete_receiver: Option<TypeIndex>,
) -> SpecializationKey {
    SpecializationKey::new(
        package_build_id,
        artifact_function_key(&format!("module::{name}")),
        PackageCallableId::new(name),
        concrete_type_arguments,
        concrete_receiver,
    )
}

pub(super) fn snapshot_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::Trivial,
    }
}

pub(super) fn snapshot_release_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::SnapshotRelease,
    }
}

pub(super) fn analyzed_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

pub(super) fn signature() -> LinkedCallableSignature {
    LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([ParamModeIr::Value]),
        Box::new([snapshot_plan()]),
        Box::new([TypeIndex::new(0)]),
        Box::new([snapshot_plan()]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect("fixture signature has one mode and plan per type")
}

pub(super) fn native_signature() -> LinkedNativeCallableSignature {
    LinkedNativeCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([ParamModeIr::Value]),
        Box::new([snapshot_plan()]),
        Box::new([TypeIndex::new(0)]),
        Box::new([snapshot_plan()]),
        analyzed_effects(),
    )
    .expect("fixture native signature has one mode and plan per type")
}

pub(super) fn frame() -> LinkedFrameLayout {
    let plan = snapshot_plan();
    LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            ParamModeIr::Value,
            plan.clone(),
        )]),
        Box::new([]),
        Box::new([TypeIndex::new(0)]),
        Box::new([plan]),
        Box::new([snapshot_plan()]),
        None,
    )
    .expect("fixture frame is locally well-shaped")
}

pub(super) fn function(index: u32, name: &str) -> LinkedFunction {
    function_with_key(index, specialization(name), name)
}

pub(super) fn function_with_key(index: u32, key: SpecializationKey, name: &str) -> LinkedFunction {
    let instruction =
        LinkedInstruction::new(Opcode::BudgetCheckpoint, Box::new([]), Box::new([]), 0)
            .expect("fixture instruction has no operands");
    let stack_map = LinkedStackMapCandidate::try_new(
        Box::new([LinkedProgramPointState::new(
            crate::InstructionIndex::new(0),
            Box::new([]),
            Box::new([LinkedSlotState::Live(LinkedStackValue::new(
                TypeIndex::new(0),
                snapshot_plan(),
            ))]),
            Box::new([]),
            Box::new([]),
        )]),
        1,
        1,
        1,
    )
    .expect("fixture stack-map claim has one state and one slot");
    LinkedFunction::new(
        FunctionIndex::new(index),
        key,
        Box::new([instruction]),
        frame(),
        1,
        LinkedCallableEffectDeclaration::new(
            PackageCallableId::new(name),
            CallableEffectSummary::analysis_pending(),
        ),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        ),
        stack_map,
    )
}

pub(super) fn function_with_statement_entries(
    instruction_count: usize,
    statement_entries: Vec<LinkedStatementEntry>,
) -> LinkedFunction {
    let base = function(0, "statements");
    let instructions = (0..instruction_count)
        .map(|_| {
            LinkedInstruction::new(Opcode::BudgetCheckpoint, Box::new([]), Box::new([]), 0)
                .expect("fixture instruction has no operands")
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let stack_map = LinkedStackMapCandidate::try_new(
        (0..instruction_count)
            .map(|position| {
                LinkedProgramPointState::new(
                    crate::InstructionIndex::new(
                        u32::try_from(position).expect("fixture instruction position fits u32"),
                    ),
                    Box::new([]),
                    Box::new([LinkedSlotState::Live(LinkedStackValue::new(
                        TypeIndex::new(0),
                        snapshot_plan(),
                    ))]),
                    Box::new([]),
                    Box::new([]),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        instruction_count,
        1,
        1,
    )
    .expect("fixture stack-map has one state per instruction");
    LinkedFunction::new(
        base.index(),
        base.key().clone(),
        instructions,
        base.frame().clone(),
        base.max_operand_depth(),
        base.effect().clone(),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            statement_entries.into_boxed_slice(),
            Box::new([]),
        ),
        stack_map,
    )
}

pub(super) fn authority_pins() -> LinkedBytecodeAuthorityPins {
    authority_pins_with_platform_error_registry(
        current_platform_error_projection_registry_ref().clone(),
    )
}

pub(super) fn authority_pins_with_platform_error_registry(
    platform_error_projection_registry: PlatformErrorProjectionRegistryRef,
) -> LinkedBytecodeAuthorityPins {
    LinkedBytecodeAuthorityPins::new(
        native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy_identity().clone(),
        host_effect_registry_identity().clone(),
        intrinsic_registry_identity().clone(),
        platform_error_projection_registry,
    )
    .expect("compile-time authority identities are canonical")
}

pub(super) fn historical_platform_error_projection_registry_ref(
) -> PlatformErrorProjectionRegistryRef {
    let current = current_platform_error_projection_registry_ref();
    let zero_fingerprint = format!("sha256:{}", "0".repeat(64));
    let fingerprint = if zero_fingerprint == current.fingerprint() {
        format!("sha256:{}", "1".repeat(64))
    } else {
        zero_fingerprint
    };
    serde_json::from_value(serde_json::json!({
        "registryId": current.registry_id(),
        "registryVersion": current.registry_version(),
        "fingerprint": fingerprint,
    }))
    .expect("historical registry descriptor satisfies the strict general shape")
}

pub(super) fn package(
    index: u32,
    package_build_id: PackageBuildId,
) -> LinkedPackageBytecodeProvenance {
    package_with_authority_pins(index, package_build_id, authority_pins())
}

pub(super) fn package_with_authority_pins(
    index: u32,
    package_build_id: PackageBuildId,
    authorities: LinkedBytecodeAuthorityPins,
) -> LinkedPackageBytecodeProvenance {
    let bytecode_identity = format!("bytecode:{}", package_build_id.as_str());
    LinkedPackageBytecodeProvenance::new(
        BytecodePackageIndex::new(index),
        package_build_id,
        BytecodeArtifactRef::new(bytecode_identity.clone()),
        bytecode_identity,
        BYTECODE_MAGIC,
        BYTECODE_SCHEMA_VERSION,
        BYTECODE_ISA_VERSION,
        opcode_table_fingerprint(),
        authorities,
    )
    .expect("fixture package provenance has a coherent exact header")
}

pub(super) fn type_origin(
    artifact_index: u32,
    specialization: Option<SpecializationKey>,
) -> LinkedArtifactPoolOrigin<ArtifactTypeIndex> {
    LinkedArtifactPoolOrigin::new(
        build_id(),
        ArtifactTypeIndex::new(artifact_index),
        specialization,
    )
    .expect("fixture type origin and specialization have the same owner")
}

pub(super) fn minimal_parts(functions: Vec<LinkedFunction>) -> LinkedBytecodeCandidateParts {
    let package_build_ids = functions
        .iter()
        .map(|function| function.key().package_build_id().clone())
        .chain(std::iter::once(build_id()))
        .collect::<BTreeSet<_>>();
    let packages = package_build_ids
        .into_iter()
        .enumerate()
        .map(|(index, package_build_id)| {
            package(
                u32::try_from(index).expect("fixture package table fits u32"),
                package_build_id,
            )
        })
        .collect();

    LinkedBytecodeCandidateParts {
        packages,
        functions,
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: Vec::new(),
        service_operations: Vec::new(),
        actor_creates: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        callback_capture_layouts: Vec::new(),
        host_effect_adapters: Vec::new(),
        intrinsics: Vec::new(),
        types: vec![LinkedTypeEntry::new(
            TypeIndex::new(0),
            type_origin(0, None),
            TypeRefIr::builtin("string"),
            None,
        )],
        shapes: Vec::new(),
        constants: Vec::new(),
        constant_roots: Vec::new(),
        frozen_constant_nodes: Vec::new(),
        resume_sites: Vec::new(),
        writable_paths: Vec::new(),
    }
}

pub(super) fn gateway_key(value: &str) -> GatewayEntryKey {
    GatewayEntryKey::parse(value).expect("fixture gateway key is canonical")
}

pub(super) fn gateway_identity() -> GatewayEntryIdentity {
    GatewayEntryIdentity::parse(format!(
        "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
        "0".repeat(64)
    ))
    .expect("fixture gateway identity is canonical")
}

pub(super) fn gateway_surface() -> GatewayEntryProtocolSurface {
    GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::RawHttp,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpRequest],
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    }
}

pub(super) fn gateway_adapter_plan() -> GatewayAdapterPlan {
    GatewayAdapterPlan {
        kind: GatewayAdapterKind::RawHttp,
        args: Vec::new(),
    }
}

pub(super) fn gateway_callable(
    role: LinkedGatewayCallableRole,
    callable_id: &str,
    function: u32,
) -> LinkedGatewayCallable {
    LinkedGatewayCallable::new(
        role,
        PackageCallableId::new(callable_id),
        FunctionIndex::new(function),
        signature(),
    )
}
