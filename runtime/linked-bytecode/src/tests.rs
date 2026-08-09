use std::any::TypeId;
use std::collections::BTreeMap;

use skiff_artifact_model::{
    AbiInterfaceId, ActorAbiIdentity, ActorMethodIdentity, CallableEffectSummary,
    ContractOperationId, GatewayEntryKey, LiteralIr, NativeTarget, Opcode, PackageBuildId,
    PackageCallableId, ServiceRequirementKey, TypeRefIr, ValueTransferPlanKind,
};

use crate::{
    ActorMethodIndex, CandidateTable, ConstantIndex, FrameSlotIndex, FunctionIndex,
    HostEffectAdapterIndex, InterfaceTableIndex, LinkedActorMethodTarget, LinkedBytecodeCandidate,
    LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts, LinkedCallableEffectDeclaration,
    LinkedCallableSignature, LinkedCallableSignatureError, LinkedCallbackCapture,
    LinkedConstantEntry, LinkedConstantValue, LinkedExactLocalTarget, LinkedFrameLayout,
    LinkedFrameLayoutError, LinkedFunction, LinkedFunctionTables, LinkedGatewayEntry,
    LinkedHostEffectAdapterTarget, LinkedInstruction, LinkedInterfaceMethod, LinkedInterfaceTable,
    LinkedOperationEntry, LinkedResumeSite, LinkedServiceOperationTarget, LinkedShapeEntry,
    LinkedSyntheticCallbackTarget, LinkedTypeEntry, ResumeSiteIndex, ServiceOperationIndex,
    ShapeIndex, SpecializationKey, SyntheticCallbackIndex, TypeIndex,
};

fn specialization(name: &str) -> SpecializationKey {
    SpecializationKey::new(PackageCallableId::new(name), Box::new([]), None)
}

fn frame() -> LinkedFrameLayout {
    LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([FrameSlotIndex::new(0)]),
        Box::new([TypeIndex::new(0)]),
        Box::new([ValueTransferPlanKind::SnapshotShare]),
        Box::new([ValueTransferPlanKind::SnapshotShare]),
    )
    .expect("fixture frame is locally well-shaped")
}

fn signature() -> LinkedCallableSignature {
    LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([ValueTransferPlanKind::SnapshotShare]),
        Box::new([TypeIndex::new(0)]),
        Box::new([ValueTransferPlanKind::SnapshotShare]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect("fixture signature has one plan for each concrete type")
}

fn gateway_key(value: &str) -> GatewayEntryKey {
    GatewayEntryKey::parse(value).expect("fixture gateway key is lexically valid")
}

fn function(index: u32, name: &str) -> LinkedFunction {
    LinkedFunction::new(
        FunctionIndex::new(index),
        specialization(name),
        Box::new([LinkedInstruction::new(
            Opcode::BudgetCheckpoint,
            Box::new([]),
            0,
        )]),
        frame(),
        1,
        LinkedCallableEffectDeclaration::new(
            PackageCallableId::new(name),
            CallableEffectSummary::analysis_pending(),
        ),
        LinkedFunctionTables::new(Box::new([]), Box::new([]), Box::new([]), Box::new([])),
    )
}

fn minimal_parts(functions: Vec<LinkedFunction>) -> LinkedBytecodeCandidateParts {
    LinkedBytecodeCandidateParts {
        functions,
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: Vec::new(),
        service_operations: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        host_effect_adapters: Vec::new(),
        types: vec![LinkedTypeEntry::new(
            TypeIndex::new(0),
            TypeRefIr::builtin("string"),
        )],
        shapes: Vec::new(),
        constants: Vec::new(),
        resume_sites: Vec::new(),
    }
}

#[test]
fn image_indices_are_distinct_rust_types() {
    assert_ne!(TypeId::of::<FunctionIndex>(), TypeId::of::<TypeIndex>());
    assert_eq!(FunctionIndex::new(7).get(), TypeIndex::new(7).get());
}

#[test]
fn specialization_key_order_is_canonical() {
    let first = SpecializationKey::new(
        PackageCallableId::new("callable:a"),
        Box::new([TypeIndex::new(0)]),
        None,
    );
    let second = SpecializationKey::new(
        PackageCallableId::new("callable:a"),
        Box::new([TypeIndex::new(0)]),
        Some(TypeIndex::new(0)),
    );
    let third = SpecializationKey::new(PackageCallableId::new("callable:b"), Box::new([]), None);
    let mut shuffled = vec![third.clone(), second.clone(), first.clone()];

    shuffled.sort();

    assert_eq!(shuffled, vec![first, second, third]);
}

#[test]
fn frame_rejects_plan_shape_mismatch() {
    let error = LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
    )
    .expect_err("one slot requires one slot plan");

    assert_eq!(
        error,
        LinkedFrameLayoutError::SlotPlanCountMismatch {
            slot_type_count: 1,
            slot_plan_count: 0,
        }
    );
}

#[test]
fn callable_signature_rejects_plan_shape_mismatch() {
    let parameter_error = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect_err("one parameter type requires one transfer plan");
    assert_eq!(
        parameter_error,
        LinkedCallableSignatureError::ParameterPlanCountMismatch {
            parameter_type_count: 1,
            parameter_plan_count: 0,
        }
    );

    let result_error = LinkedCallableSignature::new(
        Box::new([]),
        Box::new([]),
        Box::new([TypeIndex::new(0)]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect_err("one result type requires one transfer plan");
    assert_eq!(
        result_error,
        LinkedCallableSignatureError::ResultPlanCountMismatch {
            result_type_count: 1,
            result_plan_count: 0,
        }
    );
}

#[test]
fn candidate_rejects_non_dense_function_indices() {
    let error =
        LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![function(1, "callable:one")]))
            .expect_err("function table must start at zero");

    assert_eq!(
        error,
        LinkedBytecodeCandidateError::NonDenseIndex {
            table: CandidateTable::Functions,
            position: 0,
            expected: 0,
            actual: 1,
        }
    );
}

#[test]
fn candidate_rejects_duplicate_deployment_entry_identities() {
    let mut operation_parts = minimal_parts(vec![function(0, "callable:root")]);
    operation_parts.operation_entries = vec![
        LinkedOperationEntry::new(
            ContractOperationId::new("operation:duplicate"),
            FunctionIndex::new(0),
            signature(),
        ),
        LinkedOperationEntry::new(
            ContractOperationId::new("operation:duplicate"),
            FunctionIndex::new(0),
            signature(),
        ),
    ];
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(operation_parts),
        Err(LinkedBytecodeCandidateError::DuplicateOperationEntry { .. })
    ));

    let mut gateway_parts = minimal_parts(vec![function(0, "callable:root")]);
    gateway_parts.gateway_entries = vec![
        LinkedGatewayEntry::new(
            gateway_key("gateway:duplicate"),
            FunctionIndex::new(0),
            signature(),
        ),
        LinkedGatewayEntry::new(
            gateway_key("gateway:duplicate"),
            FunctionIndex::new(0),
            signature(),
        ),
    ];
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(gateway_parts),
        Err(LinkedBytecodeCandidateError::DuplicateGatewayEntry { .. })
    ));
}

#[test]
fn candidate_rejects_noncanonical_deployment_entry_order() {
    let mut parts = minimal_parts(vec![function(0, "callable:root")]);
    parts.operation_entries = vec![
        LinkedOperationEntry::new(
            ContractOperationId::new("operation:z"),
            FunctionIndex::new(0),
            signature(),
        ),
        LinkedOperationEntry::new(
            ContractOperationId::new("operation:a"),
            FunctionIndex::new(0),
            signature(),
        ),
    ];

    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(parts),
        Err(LinkedBytecodeCandidateError::NonCanonicalOperationEntryOrder { .. })
    ));
}

#[test]
fn candidate_rejects_out_of_bounds_deployment_entry_function() {
    let mut parts = minimal_parts(vec![function(0, "callable:root")]);
    parts.gateway_entries = vec![LinkedGatewayEntry::new(
        gateway_key("gateway:chat"),
        FunctionIndex::new(1),
        signature(),
    )];

    assert_eq!(
        LinkedBytecodeCandidate::try_from_parts(parts)
            .expect_err("gateway entry function must be in the candidate function table"),
        LinkedBytecodeCandidateError::RootFunctionOutOfBounds {
            source_table: CandidateTable::GatewayEntries,
            source_index: 0,
            function_index: 1,
            function_len: 1,
        }
    );
}

#[test]
fn service_target_remains_symbolic_and_provider_free() {
    let requirement = ServiceRequirementKey {
        caller_package_build_id: PackageBuildId::new("package-build:caller"),
        service_requirement_slot: 3,
    };
    let operation = ContractOperationId::new("contract-operation:send");
    let target = LinkedServiceOperationTarget::new(
        ServiceOperationIndex::new(0),
        requirement.clone(),
        operation.clone(),
        signature(),
    );

    assert_eq!(target.service_requirement_key(), &requirement);
    assert_eq!(target.contract_operation_id(), &operation);
    assert_eq!(target.signature().parameter_types(), [TypeIndex::new(0)]);
    assert_eq!(
        target.signature().effect_summary(),
        &CallableEffectSummary::analysis_pending()
    );
}

#[test]
fn type_entry_preserves_type_param_for_the_verifier() {
    let entry = LinkedTypeEntry::new(
        TypeIndex::new(0),
        TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
    );

    assert!(matches!(entry.type_ref(), TypeRefIr::TypeParam { name } if name == "T"));
}

#[test]
fn candidate_exposes_read_only_component_views() {
    let key = specialization("callable:root");
    let requirement = ServiceRequirementKey {
        caller_package_build_id: PackageBuildId::new("package-build:caller"),
        service_requirement_slot: 0,
    };
    let mut parts = minimal_parts(vec![function(0, "callable:root")]);
    parts.operation_entries = vec![LinkedOperationEntry::new(
        ContractOperationId::new("operation:chat"),
        FunctionIndex::new(0),
        signature(),
    )];
    parts.gateway_entries = vec![LinkedGatewayEntry::new(
        gateway_key("gateway:chat"),
        FunctionIndex::new(0),
        signature(),
    )];
    parts.exact_local_targets = vec![LinkedExactLocalTarget::new(
        key.clone(),
        FunctionIndex::new(0),
    )];
    parts.service_operations = vec![LinkedServiceOperationTarget::new(
        ServiceOperationIndex::new(0),
        requirement,
        ContractOperationId::new("contract-operation:send"),
        signature(),
    )];
    parts.actor_methods = vec![LinkedActorMethodTarget::new(
        ActorMethodIndex::new(0),
        ActorAbiIdentity::new("actor:worker"),
        ActorMethodIdentity::new("actor-method:run"),
        FunctionIndex::new(0),
        signature(),
    )];
    parts.interface_tables = vec![LinkedInterfaceTable::new(
        InterfaceTableIndex::new(0),
        AbiInterfaceId::from_key_bytes(vec![1, 2, 3]),
        Box::new([LinkedInterfaceMethod::new(0, signature())]),
    )];
    parts.synthetic_callbacks = vec![LinkedSyntheticCallbackTarget::new(
        SyntheticCallbackIndex::new(0),
        FunctionIndex::new(0),
        Box::new([LinkedCallbackCapture::new(
            FrameSlotIndex::new(0),
            TypeIndex::new(0),
            ValueTransferPlanKind::SnapshotShare,
        )]),
        signature(),
    )];
    parts.host_effect_adapters = vec![LinkedHostEffectAdapterTarget::new(
        HostEffectAdapterIndex::new(0),
        NativeTarget {
            namespace: "host".to_string(),
            symbol: "clock".to_string(),
            binding_key: None,
            metadata: BTreeMap::new(),
        },
        signature(),
    )];
    parts.shapes = vec![LinkedShapeEntry::new(
        ShapeIndex::new(0),
        Box::new([TypeIndex::new(0)]),
    )];
    parts.constants = vec![LinkedConstantEntry::new(
        ConstantIndex::new(0),
        LinkedConstantValue::Literal(LiteralIr::Null),
    )];
    parts.resume_sites = vec![LinkedResumeSite::new(
        ResumeSiteIndex::new(0),
        TypeIndex::new(0),
        1,
        ValueTransferPlanKind::SnapshotShare,
    )];

    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("all fixture table indices and local references are in bounds");

    assert_eq!(candidate.functions().len(), 1);
    assert_eq!(candidate.functions()[0].key(), &key);
    assert_eq!(candidate.operation_entries().len(), 1);
    assert_eq!(
        candidate.operation_entries()[0].contract_operation_id(),
        &ContractOperationId::new("operation:chat")
    );
    assert_eq!(
        candidate.operation_entries()[0]
            .signature()
            .parameter_types(),
        [TypeIndex::new(0)]
    );
    assert_eq!(candidate.gateway_entries().len(), 1);
    assert_eq!(
        candidate.gateway_entries()[0].gateway_entry_key(),
        &gateway_key("gateway:chat")
    );
    assert_eq!(
        candidate.gateway_entries()[0].function(),
        FunctionIndex::new(0)
    );
    assert_eq!(candidate.exact_local_targets().len(), 1);
    assert_eq!(candidate.service_operations().len(), 1);
    assert_eq!(candidate.actor_methods().len(), 1);
    assert_eq!(candidate.interface_tables().len(), 1);
    assert_eq!(candidate.synthetic_callbacks().len(), 1);
    assert_eq!(candidate.host_effect_adapters().len(), 1);
    assert_eq!(
        candidate.actor_methods()[0].signature().result_plans(),
        [ValueTransferPlanKind::SnapshotShare]
    );
    assert_eq!(
        candidate.interface_tables()[0].methods()[0]
            .signature()
            .parameter_types(),
        [TypeIndex::new(0)]
    );
    assert_eq!(
        candidate.synthetic_callbacks()[0]
            .signature()
            .result_types(),
        [TypeIndex::new(0)]
    );
    assert_eq!(
        candidate.host_effect_adapters()[0]
            .signature()
            .parameter_plans(),
        [ValueTransferPlanKind::SnapshotShare]
    );
    assert_eq!(candidate.types().len(), 1);
    assert_eq!(candidate.shapes().len(), 1);
    assert_eq!(candidate.constants().len(), 1);
    assert_eq!(candidate.resume_sites().len(), 1);
}
