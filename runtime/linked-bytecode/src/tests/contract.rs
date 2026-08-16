use std::any::TypeId;
use std::collections::BTreeMap;

use skiff_artifact_model::{
    derive_package_schema_type_id, ActorAbiIdentity, ActorImplementationIdentity,
    ActorMethodIdentity, BoundaryDropPlan, BoundaryErrorAdmission, BoundaryErrorFallbackIdentity,
    BoundaryErrorPlan, BoundaryErrorPolicy, BoundaryTransfer, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractOperationId, ContractTypeDescriptor, ContractTypeRef, GatewayAdapterKind,
    GatewayAdapterPlan, HostEffectExecutorIdentity, InterfaceInstantiationRef,
    NativeValueAdapterRole, NativeValueLifecycleAdapter, PackageBuildId, PackageCallableId,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRef, ReceiverCallAbi,
    ServiceProtocolIdentity, ServiceRequirementKey, ServiceSymbolRef, TypeRefIr, ValueProvenance,
};

use crate::{
    ActorCreateIndex, ActorMethodIndex, ArtifactCallbackCaptureIndex, ArtifactFunctionKey,
    ArtifactFunctionKeyParseError, CallbackCaptureLayoutIndex, ConstantIndex, FrameSlotIndex,
    FunctionIndex, HostEffectAdapterIndex, InterfaceTableIndex, IntrinsicIndex,
    LinkedActorCreateTarget, LinkedActorImplementationRef, LinkedActorMethodTarget,
    LinkedActorStateField, LinkedArtifactPoolOrigin, LinkedCallbackCapture,
    LinkedCallbackCaptureLayout, LinkedCallbackInterfaceMethod, LinkedGatewayCallableRole,
    LinkedGatewayEntry, LinkedHostBindingKey, LinkedHostEffectAdapterTarget,
    LinkedInterfaceInstantiation, LinkedInterfaceMethodAbiId, LinkedInterfaceRequirementMethod,
    LinkedInterfaceRequirementTable, LinkedInterfaceTable, LinkedInterfaceTableKind,
    LinkedIntrinsicCanonicalKey, LinkedIntrinsicKind, LinkedIntrinsicTarget, LinkedOperationEntry,
    LinkedOperationReceiver, LinkedPublicInstanceKey, LinkedRemoteInterfaceMethod,
    LinkedRemoteInterfaceTable, LinkedResourceDropPlan, LinkedServiceBoundaryErrorPlan,
    LinkedServiceBoundaryPlan, LinkedServiceBoundaryValue, LinkedServiceCallbackPlan,
    LinkedServiceOperationTarget, LinkedStaticIntrinsicTarget, LinkedSyntheticCallbackTarget,
    LinkedTaskPayloadParameter, LinkedTaskPayloadPlan, LinkedTaskTarget, LinkedTaskTiming,
    LinkedValueTransferPlan, ServiceOperationIndex, SpecializationKey, SyntheticCallbackIndex,
    TaskTargetIndex, TypeIndex,
};

use super::fixtures::{
    artifact_function_key, build_id, gateway_adapter_plan, gateway_callable, gateway_identity,
    gateway_key, gateway_surface, native_signature, signature, snapshot_plan, specialization,
};

#[test]
fn image_indices_are_distinct_rust_types() {
    assert_ne!(TypeId::of::<FunctionIndex>(), TypeId::of::<TypeIndex>());
    assert_eq!(FunctionIndex::new(7).get(), TypeIndex::new(7).get());
}

#[test]
fn operation_entry_receiver_retains_exact_const_and_call_abi() {
    let operation = ContractOperationId::new("operation:provider.label");
    let entry = LinkedOperationEntry::new_with_receiver(
        operation.clone(),
        FunctionIndex::new(3),
        signature(),
        LinkedOperationReceiver::new(ConstantIndex::new(7), ReceiverCallAbi::ExplicitSelfFirst),
    );

    assert_eq!(entry.contract_operation_id(), &operation);
    assert_eq!(entry.function(), FunctionIndex::new(3));
    let receiver = entry
        .receiver()
        .expect("provider entry retains its receiver");
    assert_eq!(receiver.constant(), ConstantIndex::new(7));
    assert_eq!(
        receiver.receiver_call_abi(),
        ReceiverCallAbi::ExplicitSelfFirst
    );
}

#[test]
fn specialization_key_order_is_canonical_and_artifact_bound() {
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
        Err(ArtifactFunctionKeyParseError::WhitespaceOrControl { .. })
    ));
}

#[test]
fn full_lifecycle_plan_preserves_adapter_role_and_abi() {
    let clone_adapter = NativeValueLifecycleAdapter {
        binding_key: "lease.clone".to_string(),
        role: NativeValueAdapterRole::CloneLease,
        abi_version: 7,
    };
    let drop_adapter = NativeValueLifecycleAdapter {
        binding_key: "lease.drop".to_string(),
        role: NativeValueAdapterRole::ResourceDrop,
        abi_version: 9,
    };
    let plan = LinkedValueTransferPlan::ExplicitCloneLease {
        clone_adapter,
        drop: LinkedResourceDropPlan::NativeAdapter {
            adapter: drop_adapter,
        },
    };

    let LinkedValueTransferPlan::ExplicitCloneLease {
        clone_adapter,
        drop: LinkedResourceDropPlan::NativeAdapter {
            adapter: drop_adapter,
        },
    } = &plan
    else {
        panic!("fixture must remain an explicit clone lease");
    };
    assert_eq!(clone_adapter.role, NativeValueAdapterRole::CloneLease);
    assert_eq!(clone_adapter.abi_version, 7);
    assert_eq!(drop_adapter.role, NativeValueAdapterRole::ResourceDrop);
    assert_eq!(drop_adapter.abi_version, 9);
}

#[test]
fn symbolic_service_target_retains_protocol_without_provider() {
    let requirement = ServiceRequirementKey {
        caller_package_build_id: build_id(),
        service_requirement_slot: 3,
    };
    let operation = ContractOperationId::new("contract-operation:send");
    let protocol = ServiceProtocolIdentity::new("protocol:chat-v1");
    let target = LinkedServiceOperationTarget::new(
        ServiceOperationIndex::new(0),
        requirement.clone(),
        operation.clone(),
        protocol.clone(),
        signature(),
        service_boundary_plan(),
    );

    assert_eq!(target.service_requirement_key(), &requirement);
    assert_eq!(target.contract_operation_id(), &operation);
    assert_eq!(target.expected_protocol_identity(), &protocol);
    assert_eq!(target.signature().parameter_types(), [TypeIndex::new(0)]);
    assert_eq!(
        target.boundary_plan().callbacks(),
        &LinkedServiceCallbackPlan::None
    );
    assert!(target.boundary_plan().arguments().is_empty());
}

fn service_boundary_plan() -> LinkedServiceBoundaryPlan {
    let value_plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Caller,
        lifetime: BoundaryValueLifetime::Call,
    };
    let fallback = LinkedServiceBoundaryValue::new(
        std_service_internal_error(),
        value_plan.clone(),
        BoundaryTransfer::Move,
        BoundaryDropPlan::SnapshotRelease,
        ValueProvenance::Fresh,
        TypeIndex::new(0),
        TypeRefIr::builtin("string"),
    );
    LinkedServiceBoundaryPlan::new(
        Vec::new(),
        Vec::new(),
        LinkedServiceBoundaryErrorPlan::new(
            BoundaryErrorPlan {
                fallback_contract_type: std_service_internal_error(),
                fallback: value_plan,
                policy: BoundaryErrorPolicy::DynamicPublicSchema {
                    admission: BoundaryErrorAdmission::PublicNameableSchemaClosed,
                    fallback_identity: BoundaryErrorFallbackIdentity::StdServiceInternalError,
                },
                transfer: BoundaryTransfer::Move,
                drop: BoundaryDropPlan::SnapshotRelease,
                source: ValueProvenance::Fresh,
            },
            fallback,
        ),
        None,
        LinkedServiceCallbackPlan::None,
    )
}

#[test]
fn linked_service_boundary_value_retains_exact_linked_type_ref() {
    let linked_type = TypeRefIr::AnyInterface {
        interface: InterfaceInstantiationRef {
            interface_abi_id: "interface-abi:handler".to_string(),
            canonical_type_args: Vec::new(),
        },
    };
    let value = LinkedServiceBoundaryValue::new(
        ContractTypeRef::AnyInterface {
            interface: Box::new(ContractTypeRef::package_schema(
                "example.com/callback".to_string(),
                "Handler".to_string(),
                derive_package_schema_type_id(
                    "example.com/callback",
                    "Handler",
                    &PackageSchemaCanonicalDescriptor {
                        type_params: Vec::new(),
                        descriptor: ContractTypeDescriptor::Record {
                            fields: BTreeMap::new(),
                        },
                    },
                )
                .unwrap(),
            )),
            arguments: Vec::new(),
        },
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            encoding: BoundaryValueEncoding::OpaqueCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
        },
        BoundaryTransfer::Copy,
        BoundaryDropPlan::SnapshotRelease,
        ValueProvenance::CallerParameter { index: 0 },
        TypeIndex::new(0),
        linked_type.clone(),
    );

    assert_eq!(value.linked_type_ref(), &linked_type);
}

fn std_service_internal_error() -> ContractTypeRef {
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("message".to_string(), ContractTypeRef::builtin("string")),
                ("traceId".to_string(), ContractTypeRef::builtin("string")),
                ("errorId".to_string(), ContractTypeRef::builtin("string")),
            ]),
        },
    };
    let type_id =
        derive_package_schema_type_id("skiff.run/std", "std.service.InternalError", &descriptor)
            .expect("canonical std.service.InternalError schema derives");
    ContractTypeRef::package_schema("skiff.run/std", "std.service.InternalError", type_id)
}

#[test]
fn actor_target_retains_exact_owner_and_implementation_identity() {
    let actor = ServiceSymbolRef {
        module_path: "actors".to_string(),
        symbol: "Worker".to_string(),
    };
    let actor_implementation = LinkedActorImplementationRef::new(
        build_id(),
        actor.clone(),
        ActorAbiIdentity::new("actor-abi:worker"),
        ActorImplementationIdentity::new("actor-implementation:worker"),
        "Worker",
        "string",
        "id",
        vec![LinkedActorStateField::new(
            "id",
            actor_boundary_value(BoundaryValueOwner::Caller),
        )],
    );
    let target = LinkedActorMethodTarget::new(
        ActorMethodIndex::new(0),
        actor_implementation.clone(),
        ActorMethodIdentity::new("actor-method:run"),
        FunctionIndex::new(0),
        signature(),
        vec![
            actor_boundary_value(BoundaryValueOwner::Caller),
            actor_boundary_value(BoundaryValueOwner::Caller),
        ],
        vec![actor_boundary_value(BoundaryValueOwner::Provider)],
    );

    assert_eq!(target.owner_package_build_id(), &build_id());
    assert_eq!(target.actor(), &actor);
    assert_eq!(
        target.actor_implementation_identity().as_str(),
        "actor-implementation:worker"
    );
    assert_eq!(target.actor_implementation().key_field(), "id");
    assert_eq!(target.actor_implementation().state_fields()[0].name(), "id");
    assert!(
        target
            .parameter_boundaries()
            .iter()
            .chain(target.result_boundaries())
            .all(|boundary| {
                !matches!(
                    boundary.contract_type(),
                    ContractTypeRef::Builtin { name, .. } if name == "unknown"
                ) && !matches!(boundary.drop(), BoundaryDropPlan::Trivial)
            }),
        "actor boundary facts must be exact linked facts, never unknown/Trivial"
    );

    let create = LinkedActorCreateTarget::new(
        ActorCreateIndex::new(0),
        actor_implementation,
        ActorMethodIdentity::new("actor-create:worker"),
        FunctionIndex::new(1),
        signature(),
        vec![actor_boundary_value(BoundaryValueOwner::Caller)],
        vec![actor_boundary_value(BoundaryValueOwner::Provider)],
    );
    assert_eq!(create.create_identity().as_str(), "actor-create:worker");
    assert_eq!(create.function(), FunctionIndex::new(1));
}

fn actor_boundary_value(owner: BoundaryValueOwner) -> LinkedServiceBoundaryValue {
    LinkedServiceBoundaryValue::new(
        ContractTypeRef::builtin("string"),
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime: BoundaryValueLifetime::Call,
        },
        BoundaryTransfer::Copy,
        BoundaryDropPlan::SnapshotRelease,
        ValueProvenance::Fresh,
        TypeIndex::new(0),
        TypeRefIr::builtin("string"),
    )
}

#[test]
fn interface_table_keeps_requirement_local_remote_and_callback_carriers_distinct() {
    let interface = || {
        LinkedInterfaceInstantiation::new(
            InterfaceInstantiationRef {
                interface_abi_id: "interface-abi:chat".to_string(),
                canonical_type_args: vec![TypeRefIr::builtin("string")],
            },
            Box::new([TypeIndex::new(0)]),
        )
        .expect("fixture linked and artifact type argument counts agree")
    };
    let requirement_method = || {
        LinkedInterfaceRequirementMethod::new(
            0,
            LinkedInterfaceMethodAbiId::parse("method-abi:send")
                .expect("fixture method ABI is canonical"),
            signature(),
        )
    };
    let requirement = LinkedInterfaceTable::new(
        InterfaceTableIndex::new(0),
        interface(),
        LinkedInterfaceTableKind::Requirement(
            LinkedInterfaceRequirementTable::new(Box::new([requirement_method()]))
                .expect("fixture method slots are canonical"),
        ),
    );
    let callback = LinkedInterfaceTable::new(
        InterfaceTableIndex::new(1),
        interface(),
        LinkedInterfaceTableKind::Callback(
            LinkedInterfaceRequirementTable::new(Box::new([requirement_method()]))
                .expect("fixture callback method slots are canonical"),
        ),
    );

    assert!(matches!(
        requirement.kind(),
        LinkedInterfaceTableKind::Requirement(_)
    ));
    assert!(matches!(
        callback.kind(),
        LinkedInterfaceTableKind::Callback(_)
    ));
    assert_eq!(
        callback.interface().artifact().interface_abi_id,
        "interface-abi:chat"
    );
}

#[test]
fn remote_method_retains_exact_linked_service_operation() {
    let method = LinkedRemoteInterfaceMethod::new(
        0,
        LinkedInterfaceMethodAbiId::parse("method-abi:read")
            .expect("fixture method ABI is canonical"),
        signature(),
        ContractOperationId::new("operation:read"),
    )
    .with_service_operation(ServiceOperationIndex::new(4));
    assert_eq!(
        method.service_operation(),
        Some(ServiceOperationIndex::new(4))
    );
    assert_eq!(method.method_slot(), 0);
    assert_eq!(method.method_abi_id().as_str(), "method-abi:read");
}

#[test]
fn task_target_retains_exact_function_and_timing_facts() {
    let target = LinkedTaskTarget::new(
        TaskTargetIndex::new(3),
        "function:work",
        FunctionIndex::new(1),
        signature(),
        LinkedTaskTiming::After { expression: 7 },
    )
    .expect("task target identity is canonical")
    .with_payload_plan(record_plan())
    .expect("exact task payload record plan must be retained");
    assert_eq!(target.index(), TaskTargetIndex::new(3));
    assert_eq!(target.target_identity(), "function:work");
    assert_eq!(target.function(), FunctionIndex::new(1));
    assert_eq!(target.timing(), LinkedTaskTiming::After { expression: 7 });
    assert_eq!(target.signature(), &signature());
    assert_eq!(
        target
            .parameter_names()
            .expect("payload plan is present")
            .as_ref(),
        ["value"]
    );
}

#[test]
fn task_target_retains_exact_parameter_tuple_plan() {
    let target = LinkedTaskTarget::new(
        TaskTargetIndex::new(4),
        "function:tuple",
        FunctionIndex::new(1),
        signature(),
        LinkedTaskTiming::Immediate,
    )
    .expect("task target identity is canonical")
    .with_payload_plan(
        LinkedTaskPayloadPlan::try_tuple(vec![payload_parameter("first")])
            .expect("task payload tuple names are unique"),
    )
    .expect("exact task payload tuple plan must be retained");
    let LinkedTaskPayloadPlan::Tuple { parameters } =
        target.payload_plan().expect("payload plan is present")
    else {
        panic!("expected linked tuple payload plan");
    };
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0].name(), "first");
    assert_eq!(parameters[0].ty(), TypeIndex::new(0));
    assert_eq!(parameters[0].transfer(), &snapshot_plan());
}

#[test]
fn task_target_without_payload_plan_fails_closed() {
    let target = LinkedTaskTarget::new(
        TaskTargetIndex::new(5),
        "function:missing",
        FunctionIndex::new(1),
        signature(),
        LinkedTaskTiming::Immediate,
    )
    .expect("task target identity is canonical");
    assert!(matches!(
        target.payload_plan(),
        Err(crate::LinkedTaskTargetError::MissingPayloadPlan)
    ));
}

fn payload_parameter(name: &str) -> LinkedTaskPayloadParameter {
    LinkedTaskPayloadParameter::new(name, TypeIndex::new(0), snapshot_plan())
        .expect("task payload parameter name is canonical")
}

fn record_plan() -> LinkedTaskPayloadPlan {
    LinkedTaskPayloadPlan::try_record(vec![payload_parameter("value")])
        .expect("task payload record names are unique")
}

#[test]
fn callback_capture_and_interface_correlation_keep_full_plan() {
    let specialization = specialization("module::callback");
    let capture_layout = LinkedCallbackCaptureLayout::try_new(
        CallbackCaptureLayoutIndex::new(0),
        LinkedArtifactPoolOrigin::new(
            build_id(),
            ArtifactCallbackCaptureIndex::new(0),
            Some(specialization.clone()),
        )
        .expect("fixture capture origin matches its specialization owner"),
        artifact_function_key("module::callback"),
        FunctionIndex::new(0),
        Box::new([LinkedCallbackCapture::new(
            FrameSlotIndex::new(0),
            TypeIndex::new(0),
            snapshot_plan(),
        )]),
    )
    .expect("fixture callback capture slots are unique");
    let method = LinkedCallbackInterfaceMethod::new(
        InterfaceTableIndex::new(0),
        2,
        LinkedInterfaceMethodAbiId::parse("method-abi:visit")
            .expect("fixture method ABI is canonical"),
        PackageSchemaTypeRef {
            package_id: "example.observer".to_string(),
            stable_schema_key: "Observer".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("contract:observer"),
        },
    );
    let callback = LinkedSyntheticCallbackTarget::new(
        SyntheticCallbackIndex::new(0),
        artifact_function_key("module::callback"),
        FunctionIndex::new(0),
        method,
        signature(),
    );

    assert_eq!(capture_layout.captures()[0].plan(), &snapshot_plan());
    assert_eq!(callback.interface_method().method_slot(), 2);
    assert_eq!(
        callback.interface_method().contract().stable_schema_key,
        "Observer"
    );
}

#[test]
fn remote_interface_table_remains_symbolic_and_protocol_pinned() {
    let requirement = ServiceRequirementKey {
        caller_package_build_id: build_id(),
        service_requirement_slot: 5,
    };
    let protocol = ServiceProtocolIdentity::new("protocol:callback-v1");
    let method = LinkedRemoteInterfaceMethod::new(
        0,
        LinkedInterfaceMethodAbiId::parse("method-abi:notify")
            .expect("fixture method ABI is canonical"),
        signature(),
        ContractOperationId::new("operation:notify"),
    );
    let table = LinkedRemoteInterfaceTable::new(
        requirement.clone(),
        LinkedPublicInstanceKey::parse("instance:notifications")
            .expect("fixture public-instance key is canonical"),
        Box::new([method]),
        protocol.clone(),
    )
    .expect("fixture remote method slots are canonical");

    assert_eq!(table.service_requirement_key(), &requirement);
    assert_eq!(table.callee_protocol_identity(), &protocol);
    assert_eq!(
        table.methods()[0].contract_operation_id(),
        &ContractOperationId::new("operation:notify")
    );
}

#[test]
fn host_and_intrinsic_targets_retain_exact_native_facts() {
    let host = LinkedHostEffectAdapterTarget::new(
        HostEffectAdapterIndex::new(0),
        HostEffectExecutorIdentity::Sleep,
        "host",
        "clock",
        LinkedHostBindingKey::parse("host.clock").expect("fixture host binding key is canonical"),
        BTreeMap::new(),
        native_signature(),
    )
    .expect("fixture host target has non-empty identity leaves");
    let intrinsic = LinkedIntrinsicTarget::new(
        IntrinsicIndex::new(0),
        LinkedIntrinsicKind::Static(
            LinkedStaticIntrinsicTarget::new(
                LinkedIntrinsicCanonicalKey::parse("std.string.length")
                    .expect("fixture intrinsic key is canonical"),
                3,
            )
            .expect("fixture intrinsic signature version is non-zero"),
        ),
        native_signature(),
    );

    assert_eq!(host.binding_key().as_str(), "host.clock");
    assert!(!host.signature().effects().may_pending);
    let LinkedIntrinsicKind::Static(target) = intrinsic.kind() else {
        panic!("fixture must be a static intrinsic");
    };
    assert_eq!(target.canonical_key().as_str(), "std.string.length");
    assert_eq!(target.signature_version(), 3);
}

#[test]
fn gateway_entry_preserves_all_optional_callable_roles() {
    let close_adapter_plan = GatewayAdapterPlan {
        kind: GatewayAdapterKind::WebSocketConnectionClosed,
        args: Vec::new(),
    };
    let entry = LinkedGatewayEntry::try_new(
        gateway_key("gateway:chat"),
        gateway_identity(),
        gateway_surface(),
        Box::new([
            gateway_callable(LinkedGatewayCallableRole::Handler, "handler", 0),
            gateway_callable(LinkedGatewayCallableRole::Pre, "pre", 0),
            gateway_callable(LinkedGatewayCallableRole::Guard, "guard", 0),
            gateway_callable(LinkedGatewayCallableRole::CloseHandler, "close", 0),
        ]),
        gateway_adapter_plan(),
        Some(close_adapter_plan.clone()),
    )
    .expect("fixture gateway roles are canonical");

    assert!(entry.handler().is_some());
    assert!(entry.pre().is_some());
    assert!(entry.guard().is_some());
    assert!(entry.close_handler().is_some());
    assert_eq!(entry.callables().len(), 4);
    assert_eq!(entry.close_adapter_plan(), Some(&close_adapter_plan));
}

#[test]
fn local_interface_method_keeps_name_function_and_receiver_abi() {
    let method = crate::LinkedLocalInterfaceMethod::new(
        0,
        "send",
        LinkedInterfaceMethodAbiId::parse("method-abi:send")
            .expect("fixture method ABI is canonical"),
        signature(),
        FunctionIndex::new(0),
        ReceiverCallAbi::ExplicitSelfFirst,
    )
    .expect("fixture method name is canonical");

    assert_eq!(method.method_name(), "send");
    assert_eq!(method.function(), FunctionIndex::new(0));
    assert_eq!(
        method.receiver_call_abi(),
        ReceiverCallAbi::ExplicitSelfFirst
    );
}
