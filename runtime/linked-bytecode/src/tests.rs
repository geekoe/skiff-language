use std::any::TypeId;
use std::collections::BTreeMap;

use skiff_artifact_model::{
    AbiInterfaceId, ActorAbiIdentity, ActorMethodIdentity, CallableEffectSummary,
    ContractOperationId, GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayHttpProtocolSurface, GatewayProtocolSurface, LiteralIr,
    NativeTarget, Opcode, PackageBuildId, PackageCallableId, ParamModeIr, ServiceRequirementKey,
    TypeRefIr, ValueTransferPlanKind, GATEWAY_ENTRY_IDENTITY_PREFIX,
};

use crate::{
    ActorMethodIndex, ArtifactFunctionKey, ArtifactFunctionKeyParseError, CandidateTable,
    ConstantIndex, FrameSlotIndex, FunctionIndex, HostEffectAdapterIndex, InterfaceTableIndex,
    LinkedActorMethodTarget, LinkedBytecodeCandidate, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts, LinkedCallableEffectDeclaration, LinkedCallableSignature,
    LinkedCallableSignatureError, LinkedCallbackCapture, LinkedConstantEntry, LinkedConstantValue,
    LinkedExactLocalTarget, LinkedFrameLayout, LinkedFrameLayoutError, LinkedFunction,
    LinkedFunctionTables, LinkedGatewayCallable, LinkedGatewayCallableRole, LinkedGatewayEntry,
    LinkedGatewayEntryError, LinkedHostEffectAdapterTarget, LinkedInstruction,
    LinkedInterfaceMethod, LinkedInterfaceTable, LinkedOperationEntry, LinkedResumeSite,
    LinkedServiceOperationTarget, LinkedShapeEntry, LinkedSyntheticCallbackTarget, LinkedTypeEntry,
    ResumeSiteIndex, ServiceOperationIndex, ShapeIndex, SpecializationKey, SyntheticCallbackIndex,
    TypeIndex,
};

fn specialization(name: &str) -> SpecializationKey {
    SpecializationKey::new(
        PackageBuildId::new("package-build:fixture"),
        artifact_function_key(&format!("module::{name}")),
        PackageCallableId::new(name),
        Box::new([]),
        None,
    )
}

fn artifact_function_key(value: &str) -> ArtifactFunctionKey {
    ArtifactFunctionKey::parse(value).expect("fixture artifact function key is canonical")
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
        Box::new([ParamModeIr::Value]),
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

fn gateway_identity() -> GatewayEntryIdentity {
    let digest = "0".repeat(64);
    GatewayEntryIdentity::parse(format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{digest}"))
        .expect("fixture gateway identity is canonical")
}

fn gateway_surface() -> GatewayEntryProtocolSurface {
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

fn gateway_adapter_plan() -> GatewayAdapterPlan {
    GatewayAdapterPlan {
        kind: GatewayAdapterKind::RawHttp,
        args: Vec::new(),
    }
}

fn gateway_callable(
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

fn gateway_entry(key: &str, callables: Box<[LinkedGatewayCallable]>) -> LinkedGatewayEntry {
    LinkedGatewayEntry::try_new(
        gateway_key(key),
        gateway_identity(),
        gateway_surface(),
        callables,
        gateway_adapter_plan(),
        None,
    )
    .expect("fixture gateway roles are unique and canonically ordered")
}

fn function(index: u32, name: &str) -> LinkedFunction {
    function_with_key(index, specialization(name), name)
}

fn function_with_key(index: u32, key: SpecializationKey, name: &str) -> LinkedFunction {
    LinkedFunction::new(
        FunctionIndex::new(index),
        key,
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
        PackageBuildId::new("package-build:a"),
        artifact_function_key("module::a"),
        PackageCallableId::new("callable:a"),
        Box::new([TypeIndex::new(0)]),
        None,
    );
    let second = SpecializationKey::new(
        PackageBuildId::new("package-build:a"),
        artifact_function_key("module::a"),
        PackageCallableId::new("callable:a"),
        Box::new([TypeIndex::new(0)]),
        Some(TypeIndex::new(0)),
    );
    let third = SpecializationKey::new(
        PackageBuildId::new("package-build:a"),
        artifact_function_key("module::b"),
        PackageCallableId::new("callable:a"),
        Box::new([]),
        None,
    );
    let fourth = SpecializationKey::new(
        PackageBuildId::new("package-build:b"),
        artifact_function_key("module::a"),
        PackageCallableId::new("callable:a"),
        Box::new([]),
        None,
    );
    let mut shuffled = vec![fourth.clone(), third.clone(), second.clone(), first.clone()];

    shuffled.sort();

    assert_eq!(shuffled, vec![first.clone(), second, third, fourth]);
    assert_eq!(
        first.package_build_id(),
        &PackageBuildId::new("package-build:a")
    );
    assert_eq!(first.artifact_function_key().as_str(), "module::a");
    assert_eq!(
        first.template_function_key(),
        &PackageCallableId::new("callable:a")
    );
}

#[test]
fn artifact_function_key_rejects_noncanonical_text() {
    assert_eq!(
        ArtifactFunctionKey::parse(""),
        Err(ArtifactFunctionKeyParseError::Empty)
    );
    assert!(matches!(
        ArtifactFunctionKey::parse("module::bad key"),
        Err(ArtifactFunctionKeyParseError::WhitespaceOrControl {
            character_index: 11,
            ..
        })
    ));
    assert!(matches!(
        ArtifactFunctionKey::parse("module::bad\nkey"),
        Err(ArtifactFunctionKeyParseError::WhitespaceOrControl { .. })
    ));
}

#[test]
fn candidate_function_uniqueness_uses_exact_artifact_provenance() {
    let first = SpecializationKey::new(
        PackageBuildId::new("package-build:a"),
        artifact_function_key("module::root"),
        PackageCallableId::new("callable:root"),
        Box::new([]),
        None,
    );
    let second = SpecializationKey::new(
        PackageBuildId::new("package-build:b"),
        artifact_function_key("module::root"),
        PackageCallableId::new("callable:root"),
        Box::new([]),
        None,
    );
    let candidate = LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![
        function_with_key(0, first.clone(), "callable:root"),
        function_with_key(1, second, "callable:root"),
    ]))
    .expect("the same semantic callable in two exact artifacts is not a duplicate key");
    assert_eq!(candidate.functions().len(), 2);

    let error = LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![
        function_with_key(0, first.clone(), "callable:root"),
        function_with_key(1, first.clone(), "callable:root"),
    ]))
    .expect_err("the same exact specialization must be unique");
    assert_eq!(
        error,
        LinkedBytecodeCandidateError::DuplicateFunctionKey { key: first }
    );
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
fn callable_signature_rejects_local_shape_mismatch() {
    let mode_error = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([]),
        Box::new([ValueTransferPlanKind::SnapshotShare]),
        Box::new([]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect_err("one parameter type requires one explicit parameter mode");
    assert_eq!(
        mode_error,
        LinkedCallableSignatureError::ParameterModeCountMismatch {
            parameter_type_count: 1,
            parameter_mode_count: 0,
        }
    );

    let parameter_error = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([ParamModeIr::Value]),
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
fn callable_signature_preserves_in_out_parameter_mode() {
    let signature = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([ParamModeIr::InOut]),
        Box::new([ValueTransferPlanKind::SnapshotShare]),
        Box::new([]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect("the signature has one explicit mode and plan per parameter");

    assert_eq!(signature.parameter_modes(), [ParamModeIr::InOut]);
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
        gateway_entry(
            "gateway:duplicate",
            Box::new([gateway_callable(
                LinkedGatewayCallableRole::Handler,
                "callable:handler",
                0,
            )]),
        ),
        gateway_entry(
            "gateway:duplicate",
            Box::new([gateway_callable(
                LinkedGatewayCallableRole::Handler,
                "callable:handler",
                0,
            )]),
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

    let mut gateway_parts = minimal_parts(vec![function(0, "callable:root")]);
    gateway_parts.gateway_entries = vec![
        gateway_entry("gateway:z", Box::new([])),
        gateway_entry("gateway:a", Box::new([])),
    ];
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(gateway_parts),
        Err(LinkedBytecodeCandidateError::NonCanonicalGatewayEntryOrder { .. })
    ));
}

#[test]
fn gateway_entry_rejects_duplicate_or_noncanonical_roles() {
    let duplicate = LinkedGatewayEntry::try_new(
        gateway_key("gateway:duplicate-role"),
        gateway_identity(),
        gateway_surface(),
        Box::new([
            gateway_callable(
                LinkedGatewayCallableRole::Handler,
                "callable:handler-one",
                0,
            ),
            gateway_callable(
                LinkedGatewayCallableRole::Handler,
                "callable:handler-two",
                0,
            ),
        ]),
        gateway_adapter_plan(),
        None,
    )
    .expect_err("a gateway role may appear at most once");
    assert_eq!(
        duplicate,
        LinkedGatewayEntryError::DuplicateCallableRole {
            role: LinkedGatewayCallableRole::Handler,
        }
    );

    let noncanonical = LinkedGatewayEntry::try_new(
        gateway_key("gateway:role-order"),
        gateway_identity(),
        gateway_surface(),
        Box::new([
            gateway_callable(LinkedGatewayCallableRole::Guard, "callable:guard", 0),
            gateway_callable(LinkedGatewayCallableRole::Pre, "callable:pre", 0),
        ]),
        gateway_adapter_plan(),
        None,
    )
    .expect_err("gateway roles must use their deterministic enum order");
    assert_eq!(
        noncanonical,
        LinkedGatewayEntryError::NonCanonicalCallableRoleOrder {
            previous: LinkedGatewayCallableRole::Guard,
            current: LinkedGatewayCallableRole::Pre,
        }
    );
}

#[test]
fn candidate_rejects_out_of_bounds_gateway_role_function() {
    let mut parts = minimal_parts(vec![function(0, "callable:root")]);
    parts.gateway_entries = vec![gateway_entry(
        "gateway:chat",
        Box::new([gateway_callable(
            LinkedGatewayCallableRole::CloseHandler,
            "callable:close",
            1,
        )]),
    )];

    assert_eq!(
        LinkedBytecodeCandidate::try_from_parts(parts)
            .expect_err("every gateway role must resolve inside the candidate function table"),
        LinkedBytecodeCandidateError::GatewayCallableFunctionOutOfBounds {
            gateway_entry_index: 0,
            gateway_entry_key: gateway_key("gateway:chat"),
            role: LinkedGatewayCallableRole::CloseHandler,
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
    let entry_identity = gateway_identity();
    let protocol_surface = gateway_surface();
    let adapter_plan = gateway_adapter_plan();
    let close_adapter_plan = GatewayAdapterPlan {
        kind: GatewayAdapterKind::WebSocketConnectionClosed,
        args: Vec::new(),
    };
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
    parts.gateway_entries = vec![LinkedGatewayEntry::try_new(
        gateway_key("gateway:chat"),
        entry_identity.clone(),
        protocol_surface.clone(),
        Box::new([
            gateway_callable(
                LinkedGatewayCallableRole::Handler,
                "callable:gateway-handler",
                0,
            ),
            gateway_callable(LinkedGatewayCallableRole::Pre, "callable:gateway-pre", 0),
            gateway_callable(
                LinkedGatewayCallableRole::Guard,
                "callable:gateway-guard",
                0,
            ),
            gateway_callable(
                LinkedGatewayCallableRole::CloseHandler,
                "callable:gateway-close",
                0,
            ),
        ]),
        adapter_plan.clone(),
        Some(close_adapter_plan.clone()),
    )
    .expect("fixture gateway roles are unique and canonically ordered")];
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
        candidate.gateway_entries()[0]
            .handler()
            .expect("fixture has a handler")
            .function(),
        FunctionIndex::new(0)
    );
    assert_eq!(
        candidate.gateway_entries()[0].gateway_entry_identity(),
        &entry_identity
    );
    assert_eq!(
        candidate.gateway_entries()[0].protocol_surface(),
        &protocol_surface
    );
    assert_eq!(candidate.gateway_entries()[0].adapter_plan(), &adapter_plan);
    assert_eq!(
        candidate.gateway_entries()[0].close_adapter_plan(),
        Some(&close_adapter_plan)
    );
    assert_eq!(candidate.gateway_entries()[0].callables().len(), 4);
    assert_eq!(
        candidate.gateway_entries()[0]
            .pre()
            .expect("fixture has a pre callable")
            .package_callable_id(),
        &PackageCallableId::new("callable:gateway-pre")
    );
    assert_eq!(
        candidate.gateway_entries()[0]
            .guard()
            .expect("fixture has a guard callable")
            .signature()
            .parameter_modes(),
        [ParamModeIr::Value]
    );
    assert_eq!(
        candidate.gateway_entries()[0]
            .close_handler()
            .expect("fixture has a close handler")
            .role(),
        LinkedGatewayCallableRole::CloseHandler
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
