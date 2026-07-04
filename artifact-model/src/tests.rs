use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::json;

use crate::service_unit::{PublicInstanceExport, PublicInstanceOperation};
use crate::{builtin_receiver_op_by_name, BuiltinReceiverOp};
use crate::{
    ActorMetadataIr, ActorMethodMetadataIr, DbIndexDirectionIr, DbIndexFieldIr, DbMetadataIndexIr,
    DbMetadataIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, FieldPathIr, SpawnTargetIr,
    SpawnTargetKindIr,
};
use crate::{
    BlockIr, BoxSourceIr, CallTargetIr, ConstIr, ExecutableBody, ExecutableIr, ExecutableKind,
    ExecutableLinkTargetIr, ExprIr, ExprRefIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
    GatewayConfig, InterfaceInstantiationRef, InterfaceMethodSlotPlanIr,
    InterfaceMethodSlotSignatureIr, InterfaceMethodSlotTargetIr, InterfaceMethodTablePlanIr,
    LiteralIr, LocalReceiverExecutableRef, OperationAbiRef, OperationCallableKind,
    OperationConstReceiverRef, OperationTargetRef, PackageDependencyConstraint,
    PackageOperationTarget, PackageRefIr, PackageSymbolRef, PackageTestAssembly,
    PackageTestAssemblyKind, PackageTestEntrypointKind, PackageUnit, PublicationAbiUnit,
    PublicationOperationKind, ReceiverCallAbi, RecoverableAdapterSchemaCompatibility,
    RecoverableArtifactMetadata, RecoverableBoundaryContext, RecoverableBoundaryKind,
    RecoverableBoundaryPlan, RecoverableCapabilityFlag, RecoverableCustomRestorePlan,
    RecoverableCustomRestorePlanRef, RecoverableExpectedTypePlan, RecoverableExpectedTypeRoot,
    RecoverableFieldIdentityFact, RecoverableFieldIdentityRef,
    RecoverableInterfaceMethodIdentityFact, RecoverableInterfaceMethodIdentityRef,
    RecoverableInterfaceProjectionIdentityFact, RecoverableInterfaceProjectionIdentityRef,
    RecoverableNativeAdapterOwner, RecoverableNativeAdapterPlan, RecoverableNativeAdapterPlanRef,
    RecoverableRestoreCapability, RecoverableStorageLane, RecoverableStorageLanePlan,
    RecoverableStorageLaneRef, RecoverableTrustBoundary, RecoverableTypeIdentityFact,
    RecoverableTypeIdentityRef, RecoverableUnionBranchIdentityFact,
    RecoverableUnionBranchIdentityRef, RemoteOperationSlotPlanIr, RemoteOperationTablePlanIr,
    ServiceConfigMetadata, ServiceDependencyConstraint, ServiceDependencySymbolRef, ServiceMeta,
    ServiceOperation, ServiceOperationTarget, ServiceSymbolRef, ServiceUnit, SlotLayout,
    SourceMapSource, SourceMapSpan, SourcePosition, SourceSpanRef, StmtIr, StmtRefIr, TypeDeclIr,
    TypeDescriptorIr, TypeLinkTargetIr, TypeRefIr,
};

fn string_type() -> TypeRefIr {
    TypeRefIr::native("string")
}

fn number_type() -> TypeRefIr {
    TypeRefIr::native("number")
}

#[test]
fn generic_interface_instantiation_uses_declaration_identity_and_canonical_args() {
    let interface_string = TypeRefIr::Native {
        name: "pkg.Boxed".to_owned(),
        args: vec![string_type()],
    };
    let interface_number = TypeRefIr::Native {
        name: "pkg.Boxed".to_owned(),
        args: vec![number_type()],
    };
    let declaration_identity = crate::type_ref_abi_key(&TypeRefIr::Native {
        name: "pkg.Boxed".to_owned(),
        args: Vec::new(),
    });

    let string_ref = crate::interface_instantiation_ref_for_type_ref(&interface_string);
    let number_ref = crate::interface_instantiation_ref_for_type_ref(&interface_number);

    assert_eq!(string_ref.interface_abi_id, declaration_identity);
    assert_eq!(number_ref.interface_abi_id, string_ref.interface_abi_id);
    assert_eq!(string_ref.canonical_type_args, vec![string_type()]);
    assert_eq!(number_ref.canonical_type_args, vec![number_type()]);
    assert_ne!(string_ref, number_ref);
    assert_ne!(
        crate::canonical_interface_method_abi_id(&string_ref, "get"),
        crate::canonical_interface_method_abi_id(&number_ref, "get")
    );
    let value = serde_json::to_value(&string_ref).expect("interface ref serializes");
    assert_eq!(value["interfaceAbiId"], declaration_identity);
    assert_eq!(value["canonicalTypeArgs"], json!([string_type()]));
}

fn operation_ref(
    operation_abi_id: &str,
    kind: PublicationOperationKind,
    public_path: &str,
) -> OperationAbiRef {
    OperationAbiRef {
        operation_abi_id: operation_abi_id.to_owned(),
        kind,
        public_path: public_path.to_owned(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: public_path.to_owned(),
    }
}

fn instance_method_operation_ref() -> OperationAbiRef {
    OperationAbiRef {
        operation_abi_id: "operation:remoteLlm:0.1.0:managedLlmService.sendChat".to_owned(),
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: "managedLlmService.sendChat".to_owned(),
        public_instance_key: Some("managedLlmService".to_owned()),
        interface: Some(InterfaceInstantiationRef {
            interface_abi_id: "iface:managed-llm".to_owned(),
            canonical_type_args: Vec::new(),
        }),
        method_abi_id: Some("method:sendChat".to_owned()),
        display_name: "managedLlmService.sendChat".to_owned(),
    }
}

fn operation_target_ref(
    callable_abi_id: &str,
    callable_kind: OperationCallableKind,
) -> OperationTargetRef {
    OperationTargetRef {
        file_ref: FileIrRef::new("file:users", "svc.users"),
        executable_index: 0,
        callable_abi_id: callable_abi_id.to_owned(),
        callable_kind,
    }
}

fn const_receiver_ref() -> OperationConstReceiverRef {
    OperationConstReceiverRef {
        file_ref: FileIrRef::new("file:users", "svc.users"),
        const_index: 0,
        const_abi_id: "const:managed-llm".to_owned(),
        const_type_abi_id: "type:managed-llm".to_owned(),
    }
}

fn local_receiver_executable_ref() -> LocalReceiverExecutableRef {
    LocalReceiverExecutableRef {
        receiver: const_receiver_ref(),
        executable_target: operation_target_ref(
            "callable:send-chat",
            OperationCallableKind::ImplMethod,
        ),
        method_abi_id: "method:sendChat".to_owned(),
        receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
    }
}

fn publication_abi_json(
    publication_id: &str,
    version: &str,
    abi_identity: &str,
) -> serde_json::Value {
    json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": publication_id,
        "version": version,
        "abiIdentity": abi_identity
    })
}

fn recoverable_type_ref_plan(ty: TypeRefIr) -> RecoverableExpectedTypePlan {
    RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeRef { ty },
        root_type_identity_ref: None,
        runtime_carrier_check_required: false,
        interface_projection_refs: Vec::new(),
        interface_method_refs: Vec::new(),
        field_refs: Vec::new(),
        union_branch_refs: Vec::new(),
    }
}

fn recoverable_identity_plan(ty: TypeRefIr, identity: &str) -> RecoverableExpectedTypePlan {
    RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeRef { ty },
        root_type_identity_ref: Some(RecoverableTypeIdentityRef(identity.to_string())),
        runtime_carrier_check_required: false,
        interface_projection_refs: Vec::new(),
        interface_method_refs: Vec::new(),
        field_refs: Vec::new(),
        union_branch_refs: Vec::new(),
    }
}

fn sample_recoverable_metadata() -> RecoverableArtifactMetadata {
    let type_ref = RecoverableTypeIdentityRef("type:user".to_owned());
    let interface_projection_ref =
        RecoverableInterfaceProjectionIdentityRef("ifaceProjection:user:managed".to_owned());
    let interface_method_ref =
        RecoverableInterfaceMethodIdentityRef("ifaceMethod:user:managed.send".to_owned());
    let field_ref = RecoverableFieldIdentityRef("field:user.name".to_owned());
    let union_branch_ref = RecoverableUnionBranchIdentityRef("union:userResult.ok".to_owned());
    let storage_lane_ref = RecoverableStorageLaneRef("lane:user.db".to_owned());
    let restore_plan_ref = RecoverableCustomRestorePlanRef("restore:user".to_owned());
    let native_adapter_plan_ref =
        RecoverableNativeAdapterPlanRef("nativeAdapter:std.date".to_owned());

    let expected = RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeRef {
            ty: TypeRefIr::native("User"),
        },
        root_type_identity_ref: Some(type_ref.clone()),
        runtime_carrier_check_required: true,
        interface_projection_refs: vec![interface_projection_ref.clone()],
        interface_method_refs: vec![interface_method_ref.clone()],
        field_refs: vec![field_ref.clone()],
        union_branch_refs: vec![union_branch_ref.clone()],
    };
    let identity_expected = RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeIdentityRef {
            type_identity_ref: type_ref.clone(),
        },
        root_type_identity_ref: Some(type_ref.clone()),
        runtime_carrier_check_required: false,
        interface_projection_refs: Vec::new(),
        interface_method_refs: Vec::new(),
        field_refs: Vec::new(),
        union_branch_refs: Vec::new(),
    };

    let mut metadata = RecoverableArtifactMetadata::default();
    metadata.identity_tables.types.insert(
        type_ref.0.clone(),
        RecoverableTypeIdentityFact {
            ty: TypeRefIr::native("User"),
            abi_type_id: Some("abiType:user".to_owned()),
            contract_revision: Some("contract:user:v1".to_owned()),
        },
    );
    metadata.identity_tables.interface_projections.insert(
        interface_projection_ref.0.clone(),
        RecoverableInterfaceProjectionIdentityFact {
            interface_type: TypeRefIr::native("ManagedUser"),
            implemented_by: Some(TypeRefIr::native("User")),
            interface_abi_id: Some("abiInterface:managedUser".to_owned()),
        },
    );
    metadata.identity_tables.interface_methods.insert(
        interface_method_ref.0.clone(),
        RecoverableInterfaceMethodIdentityFact {
            interface_projection_ref,
            method_name: "send".to_owned(),
            method_abi_id: Some("abiMethod:send".to_owned()),
            signature: Some(recoverable_type_ref_plan(TypeRefIr::native("SendResult"))),
        },
    );
    metadata.identity_tables.union_branches.insert(
        union_branch_ref.0.clone(),
        RecoverableUnionBranchIdentityFact {
            union_type_ref: type_ref.clone(),
            branch_index: 0,
            branch_type: TypeRefIr::native("User"),
            branch_abi_id: Some("abiBranch:userResult.ok".to_owned()),
        },
    );
    metadata.identity_tables.fields.insert(
        field_ref.0.clone(),
        RecoverableFieldIdentityFact {
            owner_type_ref: type_ref,
            field_name: "name".to_owned(),
            field_type: Some(string_type()),
            field_abi_id: Some("abiField:user.name".to_owned()),
        },
    );
    metadata.storage_lanes.insert(
        storage_lane_ref.0.clone(),
        RecoverableStorageLanePlan {
            lane: RecoverableStorageLane::SchemaProjectable,
            expected_type: Some(expected.clone()),
            schema_projection_ref: Some("db:users.User".to_owned()),
            envelope_slot_ref: None,
        },
    );
    metadata.custom_restore_plans.insert(
        restore_plan_ref.0.clone(),
        RecoverableCustomRestorePlan {
            concrete_type_identity: "type:user".to_owned(),
            durable_state_type_plan: identity_expected,
            encode_hook_id: "restore:user.encode".to_owned(),
            decode_hook_id: "restore:user.decode".to_owned(),
            restore_capability: RecoverableRestoreCapability::Exact,
        },
    );
    metadata.native_adapter_plans.insert(
        native_adapter_plan_ref.0.clone(),
        RecoverableNativeAdapterPlan {
            adapter_identity: "adapter:std.date".to_owned(),
            adapter_schema_version: "1".to_owned(),
            native_type_identity: "native:std.Date".to_owned(),
            durable_state_type_plan: recoverable_type_ref_plan(TypeRefIr::native("Json")),
            encode_hook_id: "adapter:std.date.encode".to_owned(),
            decode_hook_id: "adapter:std.date.decode".to_owned(),
            owner: RecoverableNativeAdapterOwner {
                service_identity: "std".to_owned(),
            },
            schema_compatibility: RecoverableAdapterSchemaCompatibility::Exact,
        },
    );
    metadata.boundary_plans.insert(
        "boundary:db:user".to_owned(),
        RecoverableBoundaryPlan {
            context: RecoverableBoundaryContext {
                boundary_kind: RecoverableBoundaryKind::DbPayload,
                trust_boundary: RecoverableTrustBoundary::OwnerInternal,
                origin_service: Some("users".to_owned()),
                target_service: None,
                explicit_recoverable_slot: false,
            },
            expected_type: expected,
            runtime_carrier_check_required: true,
            storage_lane_ref: Some(storage_lane_ref),
            custom_restore_plan_ref: Some(restore_plan_ref),
            native_adapter_plan_ref: Some(native_adapter_plan_ref),
        },
    );
    metadata.capabilities.flags.insert(
        "recoverableArtifactMetadataV1".to_owned(),
        RecoverableCapabilityFlag {
            enabled: true,
            revision: Some(1),
        },
    );

    metadata
}

#[test]
fn recoverable_expected_type_compatibility_matrix_fails_closed() {
    let base = recoverable_identity_plan(TypeRefIr::native("User"), "type:user");
    assert!(crate::recoverable_expected_type_plans_compatible(
        &base, &base
    ));

    let local_before = recoverable_identity_plan(
        TypeRefIr::LocalType { type_index: 0 },
        "type:source:module:app:User",
    );
    let local_after = recoverable_identity_plan(
        TypeRefIr::LocalType { type_index: 1 },
        "type:source:module:app:User",
    );
    assert!(crate::recoverable_expected_type_plans_compatible(
        &local_before,
        &local_after
    ));

    let package_by_id = recoverable_identity_plan(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "pkg.example".to_string(),
                },
                symbol_path: "User".to_string(),
                abi_expectation: None,
            },
        },
        "type:package:pkg.example:User",
    );
    let package_by_dependency_alias = recoverable_identity_plan(
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "user_pkg".to_string(),
                },
                symbol_path: "User".to_string(),
                abi_expectation: None,
            },
        },
        "type:package:pkg.example:User",
    );
    assert!(crate::recoverable_expected_type_plans_compatible(
        &package_by_id,
        &package_by_dependency_alias
    ));

    let identity_root_before = RecoverableExpectedTypePlan {
        root: RecoverableExpectedTypeRoot::TypeIdentityRef {
            type_identity_ref: RecoverableTypeIdentityRef("type:source:module:app:User".into()),
        },
        root_type_identity_ref: None,
        runtime_carrier_check_required: false,
        interface_projection_refs: Vec::new(),
        interface_method_refs: Vec::new(),
        field_refs: Vec::new(),
        union_branch_refs: Vec::new(),
    };
    let identity_root_after = identity_root_before.clone();
    assert!(crate::recoverable_expected_type_plans_compatible(
        &identity_root_before,
        &identity_root_after
    ));

    let renamed_field = RecoverableExpectedTypePlan {
        field_refs: vec![RecoverableFieldIdentityRef(
            "field:user.displayName".to_string(),
        )],
        ..base.clone()
    };
    let original_field = RecoverableExpectedTypePlan {
        field_refs: vec![RecoverableFieldIdentityRef("field:user.name".to_string())],
        ..base.clone()
    };
    assert!(!crate::recoverable_expected_type_plans_compatible(
        &original_field,
        &renamed_field
    ));

    let branch_a = RecoverableExpectedTypePlan {
        union_branch_refs: vec![RecoverableUnionBranchIdentityRef(
            "union:result:ok".to_string(),
        )],
        ..base.clone()
    };
    let branch_b = RecoverableExpectedTypePlan {
        union_branch_refs: vec![RecoverableUnionBranchIdentityRef(
            "union:result:success".to_string(),
        )],
        ..base.clone()
    };
    assert!(!crate::recoverable_expected_type_plans_compatible(
        &branch_a, &branch_b
    ));

    let interface_a = RecoverableExpectedTypePlan {
        interface_projection_refs: vec![RecoverableInterfaceProjectionIdentityRef(
            "interface:tool:v1".to_string(),
        )],
        ..base.clone()
    };
    let interface_b = RecoverableExpectedTypePlan {
        interface_projection_refs: vec![RecoverableInterfaceProjectionIdentityRef(
            "interface:tool:v2".to_string(),
        )],
        ..base.clone()
    };
    assert!(!crate::recoverable_expected_type_plans_compatible(
        &interface_a,
        &interface_b
    ));

    let method_a = RecoverableExpectedTypePlan {
        interface_method_refs: vec![RecoverableInterfaceMethodIdentityRef(
            "method:tool.call:v1".to_string(),
        )],
        ..base.clone()
    };
    let method_b = RecoverableExpectedTypePlan {
        interface_method_refs: vec![RecoverableInterfaceMethodIdentityRef(
            "method:tool.call:v2".to_string(),
        )],
        ..base.clone()
    };
    assert!(!crate::recoverable_expected_type_plans_compatible(
        &method_a, &method_b
    ));

    let other_nominal = recoverable_identity_plan(TypeRefIr::native("User"), "type:account");
    assert!(!crate::recoverable_expected_type_plans_compatible(
        &base,
        &other_nominal
    ));
    let different_local_identity = recoverable_identity_plan(
        TypeRefIr::LocalType { type_index: 0 },
        "type:source:module:other:User",
    );
    assert!(!crate::recoverable_expected_type_plans_compatible(
        &local_before,
        &different_local_identity
    ));

    assert!(!crate::recoverable_expected_type_plans_compatible(
        &recoverable_type_ref_plan(TypeRefIr::native("number")),
        &recoverable_type_ref_plan(TypeRefIr::native("string"))
    ));
}

#[test]
fn recoverable_custom_and_native_plans_validate_required_fields() {
    let mut metadata = RecoverableArtifactMetadata::default();
    metadata.custom_restore_plans.insert(
        "restore:user".to_string(),
        RecoverableCustomRestorePlan {
            concrete_type_identity: String::new(),
            durable_state_type_plan: recoverable_type_ref_plan(TypeRefIr::native("Json")),
            encode_hook_id: String::new(),
            decode_hook_id: "restore:user.decode".to_string(),
            restore_capability: RecoverableRestoreCapability::Exact,
        },
    );
    metadata.native_adapter_plans.insert(
        "native:date".to_string(),
        RecoverableNativeAdapterPlan {
            adapter_identity: "adapter:date".to_string(),
            adapter_schema_version: String::new(),
            native_type_identity: "native:Date".to_string(),
            durable_state_type_plan: recoverable_type_ref_plan(TypeRefIr::native("Json")),
            encode_hook_id: "adapter:date.encode".to_string(),
            decode_hook_id: String::new(),
            owner: RecoverableNativeAdapterOwner {
                service_identity: String::new(),
            },
            schema_compatibility: RecoverableAdapterSchemaCompatibility::Exact,
        },
    );

    let error = crate::validate_recoverable_artifact_metadata(&metadata)
        .expect_err("empty required custom/native recoverable fields must fail");
    let message = error.to_string();

    assert!(message.contains("restore:user concrete_type_identity is required"));
    assert!(message.contains("restore:user encode_hook_id is required"));
    assert!(message.contains("native:date adapter_schema_version is required"));
    assert!(message.contains("native:date decode_hook_id is required"));
    assert!(message.contains("native:date owner.service_identity is required"));
}

#[test]
fn recoverable_custom_plan_rejects_missing_required_schema_fields() {
    let value = json!({
        "concreteTypeIdentity": "type:user",
        "durableStateTypePlan": recoverable_type_ref_plan(TypeRefIr::native("Json")),
        "encodeHookId": "restore:user.encode",
        "restoreCapability": "exact"
    });

    assert!(serde_json::from_value::<RecoverableCustomRestorePlan>(value).is_err());

    let missing_durable_state = json!({
        "concreteTypeIdentity": "type:user",
        "encodeHookId": "restore:user.encode",
        "decodeHookId": "restore:user.decode",
        "restoreCapability": "exact"
    });
    assert!(serde_json::from_value::<RecoverableCustomRestorePlan>(missing_durable_state).is_err());
}

#[test]
fn recoverable_native_plan_rejects_missing_required_schema_fields() {
    let value = json!({
        "adapterIdentity": "adapter:date",
        "adapterSchemaVersion": "1",
        "nativeTypeIdentity": "native:Date",
        "durableStateTypePlan": recoverable_type_ref_plan(TypeRefIr::native("Json")),
        "encodeHookId": "adapter:date.encode",
        "decodeHookId": "adapter:date.decode",
        "schemaCompatibility": "exact"
    });

    assert!(serde_json::from_value::<RecoverableNativeAdapterPlan>(value).is_err());
}

fn sample_file_ir_unit() -> FileIrUnit {
    let mut unit = FileIrUnit::empty("svc.users", "source:users");
    unit.source_map.sources.push(SourceMapSource {
        id: 0,
        path: "src/users.skiff".to_owned(),
        module_path: "svc.users".to_owned(),
        source_ast_hash: Some("source:users".to_owned()),
    });
    unit.source_map.spans.push(SourceMapSpan {
        id: 0,
        source: 0,
        kind: "function".to_owned(),
        name: Some("getUser".to_owned()),
        span: SourceSpanRef {
            source_id: 0,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(3, 1),
        },
    });
    unit.type_table.push(TypeDeclIr {
        name: "User".to_owned(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([("name".to_owned(), string_type())]),
        },
        type_params: Vec::new(),
        discriminator: None,
        implements: Vec::new(),
        source_span: None,
    });
    unit.constants.push(ConstIr {
        name: "DEFAULT_NAME".to_owned(),
        ty: string_type(),
        body: ExecutableBody {
            expressions: vec![ExprIr::Literal {
                value: LiteralIr::String {
                    value: "Ada".to_owned(),
                },
            }],
            ..ExecutableBody::default()
        },
        source_span: None,
    });
    unit.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "getUser".to_owned(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::LocalType { type_index: 0 },
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_owned(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return { value: None }],
            expressions: Vec::new(),
        },
        source_span: None,
    });
    unit.link_targets
        .types
        .insert("User".to_owned(), TypeLinkTargetIr { type_index: 0 });
    unit.link_targets.executables.insert(
        "getUser".to_owned(),
        ExecutableLinkTargetIr {
            executable_index: 0,
        },
    );
    unit.external_refs.service_symbols.push(ServiceSymbolRef {
        module_path: "svc.accounts".to_owned(),
        symbol: "currentAccount".to_owned(),
    });
    unit
}

fn assert_unknown_field_rejected<T>(value: serde_json::Value)
where
    T: DeserializeOwned,
{
    let err = match serde_json::from_value::<T>(value) {
        Ok(_) => panic!("unknown field should be rejected"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("unknown field"),
        "unexpected error: {err}"
    );
}

#[test]
fn file_ir_ref_requires_module_path() {
    let error = serde_json::from_value::<FileIrRef>(json!({
        "fileIrIdentity": "file:main",
        "artifactPath": "units/files/main.json"
    }))
    .expect_err("modulePath is part of the canonical lightweight file ref");

    assert!(
        error.to_string().contains("modulePath"),
        "unexpected error: {error}"
    );
}

#[test]
fn file_ir_ref_rejects_unknown_fields() {
    assert_unknown_field_rejected::<FileIrRef>(json!({
        "fileIrIdentity": "file:main",
        "modulePath": "svc.main",
        "artifactPath": "units/files/main.json",
        "unexpected": true
    }));
}

#[test]
fn file_ir_ref_round_trips_camel_case_shape() {
    let value = json!({
        "fileIrIdentity": "file:main",
        "modulePath": "svc.main",
        "artifactPath": "units/files/main.json",
        "sourceAstHash": "source:file:main"
    });

    let decoded: FileIrRef = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(decoded.file_ir_identity, "file:main");
    assert_eq!(decoded.module_path, "svc.main");
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        value,
        "FileIrRef should serialize using canonical camelCase fields"
    );
}

#[test]
fn file_ir_unit_rejects_unknown_fields() {
    let mut value = serde_json::to_value(sample_file_ir_unit()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("runtimeAddressTable".to_owned(), json!([]));

    assert_unknown_field_rejected::<FileIrUnit>(value);
}

#[test]
fn file_ir_unit_round_trips_canonical_artifact_shape() {
    let unit = sample_file_ir_unit();
    let value = serde_json::to_value(&unit).unwrap();

    assert!(value.get("typeTable").is_some());
    assert!(value.get("constants").is_some());
    assert!(value.get("executables").is_some());
    assert!(value.get("externalRefs").is_some());
    assert!(value.get("linkTargets").is_some());
    assert!(value.get("exports").is_none());
    assert!(value.get("types").is_none());
    assert_eq!(value["declarations"]["interfaces"], json!({}));
    assert_eq!(value["sourceMap"]["format"], "skiff-file-ir-source-map-v1");
    assert_eq!(value["executables"][0]["kind"], "function");

    let decoded: FileIrUnit = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded, unit);
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}

#[test]
fn for_in_value_slot_round_trips_and_defaults_to_single_binding() {
    let entry_value = json!({
        "kind": "forIn",
        "itemSlot": 0,
        "itemType": { "kind": "builtin", "name": "string" },
        "valueSlot": 1,
        "iterable": { "expression": 0 },
        "body": "for_body"
    });

    let decoded: StmtIr = serde_json::from_value(entry_value.clone()).unwrap();
    match &decoded {
        StmtIr::ForIn {
            item_slot,
            item_type,
            value_slot,
            iterable,
            body,
        } => {
            assert_eq!(*item_slot, 0);
            assert_eq!(
                *item_type,
                Some(TypeRefIr::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                })
            );
            assert_eq!(*value_slot, Some(1));
            assert_eq!(iterable.expression, 0);
            assert_eq!(body, "for_body");
        }
        other => panic!("expected forIn statement, got {other:?}"),
    }
    assert_eq!(serde_json::to_value(decoded).unwrap(), entry_value);

    let single_value = json!({
        "kind": "forIn",
        "itemSlot": 0,
        "iterable": { "expression": 0 },
        "body": "for_body"
    });
    let single_decoded: StmtIr = serde_json::from_value(single_value.clone()).unwrap();
    match &single_decoded {
        StmtIr::ForIn {
            item_type,
            value_slot,
            ..
        } => {
            assert_eq!(*item_type, None);
            assert_eq!(*value_slot, None);
        }
        other => panic!("expected forIn statement, got {other:?}"),
    }
    assert_eq!(serde_json::to_value(single_decoded).unwrap(), single_value);
}

#[test]
fn type_decl_ir_round_trips_discriminator_metadata() {
    let mut unit = sample_file_ir_unit();
    unit.type_table[0].discriminator = Some("kind".to_string());

    let value = serde_json::to_value(&unit).unwrap();
    assert_eq!(value["typeTable"][0]["discriminator"], "kind");

    let decoded: FileIrUnit = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.type_table[0].discriminator.as_deref(), Some("kind"));
}

#[test]
fn file_ir_unit_requires_stable_interfaces_declaration_field() {
    let mut value = serde_json::to_value(sample_file_ir_unit()).unwrap();
    value["declarations"]
        .as_object_mut()
        .unwrap()
        .remove("interfaces");

    let error = serde_json::from_value::<FileIrUnit>(value)
        .expect_err("interfaces is a required FileDeclarations field")
        .to_string();

    assert!(
        error.contains("missing field `interfaces`"),
        "unexpected interfaces field error: {error}"
    );
}

#[test]
fn file_ir_rejects_runtime_only_type_address() {
    let mut value = serde_json::to_value(sample_file_ir_unit()).unwrap();
    value["executables"][0]["returnType"] = json!({
        "kind": "address",
        "addr": {
            "file": 0,
            "typeIndex": 0
        }
    });

    let err = serde_json::from_value::<FileIrUnit>(value)
        .expect_err("artifact TypeRefIr must not accept runtime addresses")
        .to_string();
    assert!(
        err.contains("unknown variant `address`"),
        "unexpected address error: {err}"
    );
}

#[test]
fn type_ref_ir_rejects_legacy_read_record_kind() {
    let err = serde_json::from_value::<TypeRefIr>(json!({
        "kind": "readRecord",
        "object": {
            "kind": "dbObjectSymbol",
            "symbol": {
                "modulePath": "svc.users",
                "symbol": "User"
            }
        },
        "projection": {
            "kind": "full"
        }
    }))
    .expect_err("artifact TypeRefIr must not accept legacy readRecord")
    .to_string();

    assert!(
        err.contains("unknown variant `readRecord`"),
        "unexpected readRecord error: {err}"
    );
}

#[test]
fn call_target_rejects_runtime_only_resolved_executable() {
    let err = serde_json::from_value::<CallTargetIr>(json!({
        "kind": "resolvedExecutable",
        "addr": {
            "file": 0,
            "executableIndex": 0
        }
    }))
    .expect_err("artifact CallTargetIr must not accept runtime linked executable addresses")
    .to_string();

    assert!(
        err.contains("unknown variant `resolvedExecutable`"),
        "unexpected resolvedExecutable error: {err}"
    );
}

#[test]
fn package_call_target_uses_operation_abi_ref_shape() {
    let operation = operation_ref(
        "operation:tools:ping",
        PublicationOperationKind::PublicFunction,
        "tools.ping",
    );
    let target = CallTargetIr::PackageSymbol {
        package_ref: PackageRefIr::Dependency {
            dependency_ref: "tools".to_owned(),
        },
        operation: operation.clone(),
    };
    let value = serde_json::to_value(&target).unwrap();

    assert_eq!(
        value,
        json!({
            "kind": "packageSymbol",
            "packageRef": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "operation": operation
        })
    );
    assert_eq!(
        serde_json::from_value::<CallTargetIr>(value).unwrap(),
        target
    );
}

#[test]
fn package_call_target_rejects_legacy_symbol_path_shape() {
    let error = serde_json::from_value::<CallTargetIr>(json!({
        "kind": "packageSymbol",
        "symbol": {
            "package": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "symbolPath": "tools.ping",
            "abiExpectation": "abi:v1"
        }
    }))
    .expect_err("package public call target must not accept legacy symbolPath shape")
    .to_string();

    assert!(
        error.contains("packageRef")
            || error.contains("operation")
            || error.contains("unknown field"),
        "unexpected legacy package call target error: {error}"
    );
}

#[test]
fn symbol_refs_reject_unknown_fields() {
    assert_unknown_field_rejected::<ServiceSymbolRef>(json!({
        "modulePath": "svc.main",
        "symbol": "handler",
        "display": "svc.main.handler"
    }));

    assert_unknown_field_rejected::<PackageSymbolRef>(json!({
        "package": {
            "kind": "dependency",
            "dependencyRef": "mailer"
        },
        "symbolPath": "email.send",
        "legacyKey": "mailer::email.send"
    }));
}

#[test]
fn symbol_refs_round_trip_canonical_fields() {
    let service_symbol = ServiceSymbolRef {
        module_path: "svc.main".to_owned(),
        symbol: "handler".to_owned(),
    };
    let service_value = serde_json::to_value(&service_symbol).unwrap();
    assert_eq!(
        service_value,
        json!({
            "modulePath": "svc.main",
            "symbol": "handler"
        })
    );
    let decoded_service: ServiceSymbolRef = serde_json::from_value(service_value).unwrap();
    assert_eq!(decoded_service, service_symbol);

    let package_symbol = PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: "mailer".to_owned(),
        },
        symbol_path: "email.send".to_owned(),
        abi_expectation: Some("abi:v1".to_owned()),
    };
    let package_value = serde_json::to_value(&package_symbol).unwrap();
    assert_eq!(
        package_value,
        json!({
            "package": {
                "kind": "dependency",
                "dependencyRef": "mailer"
            },
            "symbolPath": "email.send",
            "abiExpectation": "abi:v1"
        })
    );
    let decoded_package: PackageSymbolRef = serde_json::from_value(package_value).unwrap();
    assert_eq!(decoded_package, package_symbol);
}

#[test]
fn package_unit_rejects_unknown_fields_and_keeps_dependency_config_open() {
    let value = json!({
        "schemaVersion": "skiff-package-unit-v1",
        "packageId": "example.com/mongo",
        "version": "1.0.0",
        "buildIdentity": "build:1",
        "abiIdentity": "abi:1",
        "publicationAbi": publication_abi_json("example.com/mongo", "1.0.0", "abi:1"),
        "files": [],
        "dependencies": [
            {
                "id": "example.com/core",
                "version": "1.0.0",
                "alias": "core",
                "config": {
                    "uri": { "env": "MONGO_URL" },
                    "pool": { "max": 5 }
                }
            }
        ],
        "configAndEffectMetadata": {},
        "runtimeOnly": true
    });

    assert_unknown_field_rejected::<PackageUnit>(value.clone());

    let mut canonical = value;
    canonical.as_object_mut().unwrap().remove("runtimeOnly");
    let decoded: PackageUnit = serde_json::from_value(canonical).unwrap();
    assert_eq!(
        decoded.dependencies[0].config["uri"],
        json!({ "env": "MONGO_URL" })
    );
}

#[test]
fn package_unit_empty_uses_canonical_defaults() {
    let unit = PackageUnit::empty("example.com/mongo", "1.0.0", "build:1", "abi:1");

    assert_eq!(unit.schema_version, "skiff-package-unit-v1");
    assert_eq!(unit.package_id, "example.com/mongo");
    assert_eq!(
        serde_json::to_value(unit).unwrap(),
        json!({
            "schemaVersion": "skiff-package-unit-v1",
            "packageId": "example.com/mongo",
            "version": "1.0.0",
            "buildIdentity": "build:1",
            "abiIdentity": "abi:1",
            "publicationAbi": {
                "schemaVersion": "skiff-publication-abi-unit-v1",
                "publicationId": "example.com/mongo",
                "version": "1.0.0",
                "abiIdentity": "abi:1"
            },
            "files": [],
            "configAndEffectMetadata": {}
        })
    );
}

#[test]
fn empty_service_and_package_units_skip_recoverable_metadata() {
    let service = ServiceUnit::empty("remoteLlm", "0.1.0", "protocol:1");
    let service_value = serde_json::to_value(&service).unwrap();
    assert!(service.recoverable_metadata.is_empty());
    assert!(service_value.get("recoverableMetadata").is_none());

    let package = PackageUnit::empty("example.com/mongo", "1.0.0", "build:1", "abi:1");
    let package_value = serde_json::to_value(&package).unwrap();
    assert!(package.recoverable_metadata.is_empty());
    assert!(package_value.get("recoverableMetadata").is_none());
}

#[test]
fn old_service_and_package_units_default_recoverable_metadata_to_empty() {
    let old_service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "remoteLlm" },
        "version": "0.1.0",
        "protocolIdentity": "protocol:1",
        "publicationAbi": publication_abi_json("remoteLlm", "0.1.0", ""),
        "files": [],
        "gateway": {},
        "config": {}
    });
    let service: ServiceUnit = serde_json::from_value(old_service).unwrap();
    assert!(service.recoverable_metadata.is_empty());
    assert!(serde_json::to_value(&service)
        .unwrap()
        .get("recoverableMetadata")
        .is_none());

    let old_package = json!({
        "schemaVersion": "skiff-package-unit-v1",
        "packageId": "example.com/mongo",
        "version": "1.0.0",
        "buildIdentity": "build:1",
        "abiIdentity": "abi:1",
        "publicationAbi": publication_abi_json("example.com/mongo", "1.0.0", "abi:1"),
        "files": [],
        "configAndEffectMetadata": {}
    });
    let package: PackageUnit = serde_json::from_value(old_package).unwrap();
    assert!(package.recoverable_metadata.is_empty());
    assert!(serde_json::to_value(&package)
        .unwrap()
        .get("recoverableMetadata")
        .is_none());
}

#[test]
fn non_empty_recoverable_metadata_round_trips_on_service_and_package_units() {
    let metadata = sample_recoverable_metadata();
    crate::validate_recoverable_artifact_metadata(&metadata).unwrap();

    let mut service = ServiceUnit::empty("remoteLlm", "0.1.0", "protocol:1");
    service.recoverable_metadata = metadata.clone();
    let service_value = serde_json::to_value(&service).unwrap();
    assert_eq!(
        service_value["recoverableMetadata"]["boundaryPlans"]["boundary:db:user"]["context"]
            ["boundaryKind"],
        "dbPayload"
    );
    assert_eq!(
        service_value["recoverableMetadata"]["identityTables"]["fields"]["field:user.name"]
            ["fieldName"],
        "name"
    );
    assert_eq!(
        serde_json::from_value::<ServiceUnit>(service_value).unwrap(),
        service
    );

    let mut package = PackageUnit::empty("example.com/mongo", "1.0.0", "build:1", "abi:1");
    package.recoverable_metadata = metadata;
    let package_value = serde_json::to_value(&package).unwrap();
    assert_eq!(
        package_value["recoverableMetadata"]["storageLanes"]["lane:user.db"]["lane"],
        "schemaProjectable"
    );
    assert_eq!(
        package_value["recoverableMetadata"]["nativeAdapterPlans"]["nativeAdapter:std.date"]
            ["adapterIdentity"],
        "adapter:std.date"
    );
    assert_eq!(
        serde_json::from_value::<PackageUnit>(package_value).unwrap(),
        package
    );
}

#[test]
fn package_unit_rejects_legacy_top_level_exports() {
    assert_unknown_field_rejected::<PackageUnit>(json!({
        "schemaVersion": "skiff-package-unit-v1",
        "packageId": "example.com/mongo",
        "version": "1.0.0",
        "buildIdentity": "build:1",
        "abiIdentity": "abi:1",
        "files": [],
        "exports": {},
        "configAndEffectMetadata": {}
    }));
}

#[test]
fn package_unit_requires_publication_abi() {
    let error = serde_json::from_value::<PackageUnit>(json!({
        "schemaVersion": "skiff-package-unit-v1",
        "packageId": "example.com/mongo",
        "version": "1.0.0",
        "buildIdentity": "build:1",
        "abiIdentity": "abi:1",
        "files": [],
        "configAndEffectMetadata": {}
    }))
    .expect_err("PackageUnit without publicationAbi must fail closed")
    .to_string();
    assert!(
        error.contains("publicationAbi"),
        "unexpected missing publicationAbi error: {error}"
    );
}

#[test]
fn package_test_assembly_round_trips_canonical_shape() {
    let value = package_test_assembly_json();
    let decoded: PackageTestAssembly = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(decoded.kind, PackageTestAssemblyKind::PackageTest);
    assert_eq!(
        decoded.test_entrypoints[0].kind,
        PackageTestEntrypointKind::TestOnly
    );
    assert_eq!(decoded.test_files[0].file_ir_path, "units/files/test.json");
    assert_eq!(
        decoded.production_package_unit.unit_path,
        "units/packages/example.com/pkg/prod.json"
    );
    assert!(decoded.link_policy.current_package_production.allow_private);
    assert!(!decoded.link_policy.dependency_public_scopes[0].allow_private);
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        value,
        "PackageTestAssembly should serialize using canonical camelCase fields"
    );
}

#[test]
fn package_test_assembly_rejects_unknown_top_level_fields() {
    let mut value = package_test_assembly_json();
    value
        .as_object_mut()
        .unwrap()
        .insert("serviceId".to_owned(), json!("legacy-service-field"));

    assert_unknown_field_rejected::<PackageTestAssembly>(value);
}

#[test]
fn package_test_assembly_rejects_unknown_entrypoint_fields() {
    let mut value = package_test_assembly_json();
    value["testEntrypoints"][0]
        .as_object_mut()
        .unwrap()
        .insert("operationAbiId".to_owned(), json!("operation:legacy"));

    assert_unknown_field_rejected::<PackageTestAssembly>(value);
}

fn package_test_assembly_json() -> serde_json::Value {
    let owner_file = json!({
        "fileIrIdentity": "skiff-file-ir-v3:sha256:testfile",
        "fileIrPath": "units/files/test.json",
        "sourcePath": "tests/pkg.test.skiff",
        "modulePath": "pkg.test"
    });

    json!({
        "schemaVersion": "skiff-package-test-assembly-v1",
        "kind": "packageTest",
        "packageId": "example.com/pkg",
        "packageVersion": "1.0.0",
        "testBuildIdentity": "skiff-package-test-build-v1:sha256:testbuild",
        "productionPackageUnit": {
            "packageId": "example.com/pkg",
            "version": "1.0.0",
            "buildIdentity": "skiff-package-build-v1:sha256:prod",
            "unitPath": "units/packages/example.com/pkg/prod.json",
            "publicAbiIdentity": "skiff-package-abi-v1:sha256:prodabi",
            "implementationLinksIdentity": "sha256:prodlinks"
        },
        "testFiles": [owner_file.clone()],
        "dependencyPackageUnits": [
            {
                "packageId": "example.com/dep",
                "version": "1.0.0",
                "buildIdentity": "skiff-package-build-v1:sha256:dep",
                "unitPath": "units/packages/example.com/dep/dep.json",
                "publicAbiIdentity": "skiff-package-abi-v1:sha256:depabi",
                "implementationLinksIdentity": "sha256:deplinks"
            }
        ],
        "testEntrypoints": [
            {
                "kind": "testOnly",
                "entrypointLocalId": "skiff-package-test-entrypoint-local-v1:sha256:local",
                "entrypointId": "skiff-package-test-entrypoint-v1:sha256:entry",
                "displayName": "runs internal helper",
                "sourcePath": "tests/pkg.test.skiff",
                "modulePath": "pkg.test",
                "ownerTestFile": owner_file,
                "executableRef": {
                    "fileIrIdentity": "skiff-file-ir-v3:sha256:testfile",
                    "executableIndex": 0,
                    "executableLocalId": "test-entrypoint-0",
                    "symbol": "__skiff_package_test_0"
                },
                "defaultRun": true,
                "configAndEffectMetadata": {},
                "runtimeExpectedError": {
                    "code": "ProviderUnavailableError",
                    "messageContains": "offline"
                }
            }
        ],
        "linkPolicy": {
            "currentPackageProduction": {
                "packageId": "example.com/pkg",
                "version": "1.0.0",
                "buildIdentity": "skiff-package-build-v1:sha256:prod",
                "filesDigest": "sha256:prodfiles",
                "implementationLinksDigest": "sha256:prodlinks",
                "allowPrivate": true
            },
            "testFileScopes": [
                {
                    "ownerTestFileIdentity": "skiff-file-ir-v3:sha256:testfile",
                    "sourcePath": "tests/pkg.test.skiff",
                    "modulePath": "pkg.test",
                    "allowedLocalLinkDigest": "sha256:testlinks",
                    "entrypointLocalIds": [
                        "skiff-package-test-entrypoint-local-v1:sha256:local"
                    ]
                }
            ],
            "dependencyPublicScopes": [
                {
                    "packageId": "example.com/dep",
                    "version": "1.0.0",
                    "buildIdentity": "skiff-package-build-v1:sha256:dep",
                    "publicAbiIdentity": "skiff-package-abi-v1:sha256:depabi",
                    "publicExportDigest": "sha256:depexports",
                    "implementationLinksDigest": "sha256:deplinks",
                    "allowPrivate": false
                }
            ]
        },
        "configAndEffectMetadata": {},
        "sourceMap": {
            "sources": []
        }
    })
}

#[test]
fn publication_abi_unit_round_trips_operation_ref() {
    let unit = PublicationAbiUnit {
        operation_exports: vec![OperationAbiRef {
            operation_abi_id: "call:send".to_string(),
            kind: PublicationOperationKind::PublicInstanceMethod,
            public_path: "managedLlm.sendChat".to_string(),
            public_instance_key: Some("managedLlm".to_string()),
            interface: Some(InterfaceInstantiationRef {
                interface_abi_id: "iface:managed-llm".to_string(),
                canonical_type_args: Vec::new(),
            }),
            method_abi_id: Some("method:sendChat".to_string()),
            display_name: "managedLlm.sendChat".to_string(),
        }],
        ..PublicationAbiUnit::empty("example.com/llm", "1.0.0", "abi:llm")
    };

    let value = serde_json::to_value(&unit).unwrap();
    assert_eq!(value["schemaVersion"], "skiff-publication-abi-unit-v1");
    assert_eq!(value["publicationId"], "example.com/llm");
    assert_eq!(value["operationExports"][0]["operationAbiId"], "call:send");
    assert_eq!(
        value["operationExports"][0]["interface"]["interfaceAbiId"],
        "iface:managed-llm"
    );

    assert_eq!(
        serde_json::from_value::<PublicationAbiUnit>(value).unwrap(),
        unit
    );
}

#[test]
fn package_unit_rejects_legacy_binding_requirements_field() {
    let without_binding_requirements = json!({
        "schemaVersion": "skiff-package-unit-v1",
        "packageId": "example.com/agent",
        "version": "1.0.0",
        "buildIdentity": "build:1",
        "abiIdentity": "abi:1",
        "publicationAbi": publication_abi_json("example.com/agent", "1.0.0", "abi:1"),
        "files": [],
        "configAndEffectMetadata": {}
    });
    let decoded_without_binding_requirements: PackageUnit =
        serde_json::from_value(without_binding_requirements).unwrap();
    let value = serde_json::to_value(&decoded_without_binding_requirements).unwrap();
    assert!(value.get("bindingRequirements").is_none());

    let mut legacy = value;
    legacy.as_object_mut().unwrap().insert(
        "bindingRequirements".to_string(),
        json!([{ "alias": "managedLlm" }]),
    );
    assert_unknown_field_rejected::<PackageUnit>(legacy);
}

#[test]
fn service_unit_round_trips_canonical_operation_shape() {
    let operation = ServiceOperation::LocalExecutable(ServiceOperationTarget {
        operation: operation_ref(
            "operation:users:dev:createUser",
            PublicationOperationKind::PublicFunction,
            "createUser",
        ),
        executable: operation_target_ref(
            "callable:create-user",
            OperationCallableKind::PublicFunction,
        ),
    });
    let unit = ServiceUnit {
        schema_version: "skiff-service-unit-v1".to_owned(),
        service: ServiceMeta {
            id: "users".to_owned(),
            display_name: Some("Users".to_owned()),
            metadata: BTreeMap::new(),
        },
        version: "dev".to_owned(),
        protocol_identity: "protocol:1".to_owned(),
        abi_identity_projection: Default::default(),
        publication_abi: PublicationAbiUnit::empty("users", "dev", ""),
        files: vec![FileIrRef::new("file:users", "svc.users")],
        package_dependencies: vec![PackageDependencyConstraint {
            id: "example.com/mongo".to_owned(),
            version: "1.0.0".to_owned(),
            alias: "mongo".to_owned(),
            config: json!({ "uri": { "env": "MONGO_URL" } }),
        }],
        service_dependencies: Vec::new(),
        package_abi_expectations: Vec::new(),
        operations: vec![operation],
        operation_route_bindings: Vec::new(),
        public_instances: Vec::new(),
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        db: vec![DbMetadataIr {
            module_path: "svc.users".to_owned(),
            source_role: "contract".to_owned(),
            package_id: None,
            package_version: None,
            file_ir_identity: None,
            kind: DbObjectKindIr::Object,
            ty: TypeRefIr::native("User"),
            type_name: "User".to_owned(),
            collection_name: "user".to_owned(),
            key: Some(DbObjectKeyIr {
                name: "id".to_owned(),
                ty: string_type(),
            }),
            fields: vec![DbObjectFieldIr {
                name: "name".to_owned(),
                ty: string_type(),
            }],
            retention: None,
            leases: Vec::new(),
            indexes: vec![DbMetadataIndexIr {
                name: "byName".to_owned(),
                unique: true,
                fields: vec![DbIndexFieldIr {
                    field: FieldPathIr {
                        text: "name".to_owned(),
                        segments: vec!["name".to_owned()],
                    },
                    direction: DbIndexDirectionIr::Asc,
                }],
                where_expr: None,
            }],
        }],
        spawn_targets: vec![SpawnTargetIr {
            target_identity: "function:Session.activate".to_owned(),
            kind: SpawnTargetKindIr::Function,
            executable_target: operation_target_ref(
                "callable:activate-session",
                OperationCallableKind::InternalFunction,
            ),
            param_types: vec![string_type(), number_type()],
            return_type: Some(number_type()),
            service_protocol_identity: "protocol:1".to_owned(),
        }],
        actors: vec![ActorMetadataIr {
            actor_type_identity: TypeRefIr::ServiceSymbol {
                symbol: crate::ServiceSymbolRef {
                    module_path: "svc.users".to_owned(),
                    symbol: "SessionActor".to_owned(),
                },
            },
            actor_id_type_identity: string_type(),
            methods: vec![ActorMethodMetadataIr {
                method_identity: "svc.users.SessionActor.activate".to_owned(),
                executable_target: operation_target_ref(
                    "callable:session-actor-activate",
                    OperationCallableKind::ImplMethod,
                ),
                param_types: vec![string_type()],
                return_type: Some(number_type()),
            }],
        }],
        gateway: GatewayConfig::default(),
        timeout: Default::default(),
        config: ServiceConfigMetadata::default(),
    };

    let value = serde_json::to_value(&unit).unwrap();
    assert_eq!(value["operations"][0]["kind"], "localExecutable");
    assert_eq!(
        value["operations"][0]["executable"]["fileRef"]["modulePath"],
        "svc.users"
    );
    assert_eq!(
        value["operations"][0]["executable"]["callableAbiId"],
        "callable:create-user"
    );
    assert_eq!(value["config"]["packageConfigs"], serde_json::Value::Null);
    // Byte-shape parity with runtime artifact shapes:
    // nested `type` key, always-present nullable `key`/`retention`, and `where` emitted as null.
    assert_eq!(value["db"][0]["type"]["name"], "User");
    assert_eq!(value["db"][0]["retention"], serde_json::Value::Null);
    assert_eq!(
        value["db"][0]["indexes"][0]["where"],
        serde_json::Value::Null
    );
    assert_eq!(value["spawnTargets"][0]["kind"], "function");
    assert_eq!(
        value["spawnTargets"][0]["executableTarget"]["callableKind"],
        "internalFunction"
    );
    assert_eq!(
        value["actors"][0]["methods"][0]["methodIdentity"],
        "svc.users.SessionActor.activate"
    );

    let decoded: ServiceUnit = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, unit);
}

#[test]
fn service_unit_requires_publication_abi() {
    let error = serde_json::from_value::<ServiceUnit>(json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "remoteLlm" },
        "version": "0.1.0",
        "protocolIdentity": "protocol:1",
        "files": [],
        "gateway": {},
        "config": {}
    }))
    .expect_err("ServiceUnit without publicationAbi must fail closed")
    .to_string();
    assert!(
        error.contains("publicationAbi"),
        "unexpected missing publicationAbi error: {error}"
    );
}

#[test]
fn service_dependency_requires_publication_abi() {
    let error = serde_json::from_value::<ServiceDependencyConstraint>(json!({
        "id": "example.com/upstream",
        "version": "1.0.0",
        "alias": "upstream",
        "buildId": "build:upstream",
        "serviceProtocolIdentity": "protocol:upstream"
    }))
    .expect_err("ServiceDependencyConstraint without publicationAbi must fail closed")
    .to_string();
    assert!(
        error.contains("publicationAbi"),
        "unexpected missing publicationAbi error: {error}"
    );
}

#[test]
fn service_unit_public_instances_round_trip_and_default_to_empty() {
    let instance = PublicInstanceExport {
        name: "managedLlmService".to_owned(),
        module_path: "api.llm".to_owned(),
        declared_receiver_type: TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "api.llm".to_owned(),
                symbol: "ManagedLlm".to_owned(),
            },
        },
        implemented_interfaces: vec![TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: String::new(),
                symbol: "llm.ManagedLlmService".to_owned(),
            },
        }],
        operations: vec![PublicInstanceOperation {
            operation: instance_method_operation_ref(),
            receiver_executable: local_receiver_executable_ref(),
        }],
    };
    let mut unit = ServiceUnit::empty("remoteLlm", "0.1.0", "protocol:1");
    unit.public_instances = vec![instance.clone()];

    let value = serde_json::to_value(&unit).unwrap();
    assert_eq!(value["publicInstances"][0]["name"], "managedLlmService");
    assert_eq!(
        value["publicInstances"][0]["declaredReceiverType"],
        json!({
            "kind": "serviceSymbol",
            "symbol": {
                "modulePath": "api.llm",
                "symbol": "ManagedLlm"
            }
        })
    );
    assert_eq!(
        value["publicInstances"][0]["operations"][0]["operation"]["operationAbiId"],
        "operation:remoteLlm:0.1.0:managedLlmService.sendChat"
    );
    assert_eq!(
        value["publicInstances"][0]["operations"][0]["receiverExecutable"]["receiverCallAbi"],
        "explicitSelfFirst"
    );
    let decoded: ServiceUnit = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.public_instances, vec![instance]);

    let without_public_instances = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "remoteLlm" },
        "version": "0.1.0",
        "protocolIdentity": "protocol:1",
        "publicationAbi": publication_abi_json("remoteLlm", "0.1.0", ""),
        "files": [],
        "gateway": {},
        "config": {}
    });
    let decoded_without_public_instances: ServiceUnit =
        serde_json::from_value(without_public_instances).unwrap();
    assert!(decoded_without_public_instances.public_instances.is_empty());
}

#[test]
fn service_unit_rejects_runtime_linked_operation_fields() {
    assert_unknown_field_rejected::<ServiceOperation>(json!({
        "kind": "localExecutable",
        "operation": operation_ref(
            "operation:users:dev:createUser",
            PublicationOperationKind::PublicFunction,
            "createUser",
        ),
        "executable": operation_target_ref(
            "callable:create-user",
            OperationCallableKind::PublicFunction,
        ),
        "file": { "kind": "loadedFileIndex", "value": 0 }
    }));
}

#[test]
fn operation_target_refs_round_trip_structured_file_index_and_abi_fields() {
    let target = operation_target_ref(
        "callable:create-user",
        OperationCallableKind::PublicFunction,
    );
    let value = serde_json::to_value(&target).unwrap();

    assert_eq!(value["fileRef"]["fileIrIdentity"], "file:users");
    assert_eq!(value["executableIndex"], 0);
    assert_eq!(value["callableAbiId"], "callable:create-user");
    assert_eq!(value["callableKind"], "publicFunction");
    assert!(value.get("modulePath").is_none());
    assert!(value.get("symbol").is_none());

    assert_eq!(
        serde_json::from_value::<OperationTargetRef>(value).unwrap(),
        target
    );

    assert_unknown_field_rejected::<OperationTargetRef>(json!({
        "modulePath": "svc.users",
        "symbol": "create",
        "executableIndex": 0
    }));

    let missing_abi_id = json!({
        "fileRef": {
            "fileIrIdentity": "file:users",
            "modulePath": "svc.users"
        },
        "executableIndex": 0,
        "callableKind": "publicFunction"
    });
    let err = serde_json::from_value::<OperationTargetRef>(missing_abi_id)
        .expect_err("callableAbiId is required")
        .to_string();
    assert!(
        err.contains("callableAbiId"),
        "unexpected missing callableAbiId error: {err}"
    );
}

#[test]
fn const_receiver_refs_round_trip_structured_file_index_and_abi_fields() {
    let receiver = const_receiver_ref();
    let value = serde_json::to_value(&receiver).unwrap();

    assert_eq!(value["fileRef"]["fileIrIdentity"], "file:users");
    assert_eq!(value["constIndex"], 0);
    assert_eq!(value["constAbiId"], "const:managed-llm");
    assert_eq!(value["constTypeAbiId"], "type:managed-llm");
    assert!(value.get("modulePath").is_none());
    assert!(value.get("constName").is_none());

    assert_eq!(
        serde_json::from_value::<OperationConstReceiverRef>(value).unwrap(),
        receiver
    );

    assert_unknown_field_rejected::<OperationConstReceiverRef>(json!({
        "modulePath": "svc.users",
        "constName": "managedLlm"
    }));
}

#[test]
fn local_receiver_executable_ref_round_trips_explicit_self_first() {
    let receiver_executable = local_receiver_executable_ref();
    let value = serde_json::to_value(&receiver_executable).unwrap();

    assert_eq!(value["receiverCallAbi"], "explicitSelfFirst");
    assert_eq!(value["methodAbiId"], "method:sendChat");
    assert_eq!(value["executableTarget"]["callableKind"], "implMethod");

    assert_eq!(
        serde_json::from_value::<LocalReceiverExecutableRef>(value).unwrap(),
        receiver_executable
    );
}

#[test]
fn package_operation_targets_use_structured_operation_targets() {
    let operation = operation_ref(
        "operation:users:dev:createUser",
        PublicationOperationKind::PublicFunction,
        "createUser",
    );
    let local = PackageOperationTarget::LocalExecutable {
        operation: operation.clone(),
        target: operation_target_ref(
            "callable:create-user",
            OperationCallableKind::PublicFunction,
        ),
    };
    let local_value = serde_json::to_value(&local).unwrap();
    assert_eq!(local_value["kind"], "localExecutable");
    assert_eq!(
        local_value["operation"]["operationAbiId"],
        "operation:users:dev:createUser"
    );
    assert_eq!(local_value["target"]["callableKind"], "publicFunction");
    assert!(local_value["target"].get("signature").is_none());

    let receiver = PackageOperationTarget::LocalConstReceiverExecutable {
        operation: instance_method_operation_ref(),
        target: local_receiver_executable_ref(),
    };
    let receiver_value = serde_json::to_value(&receiver).unwrap();
    assert_eq!(receiver_value["kind"], "localConstReceiverExecutable");
    assert_eq!(
        receiver_value["target"]["receiverCallAbi"],
        "explicitSelfFirst"
    );
    assert_eq!(
        serde_json::from_value::<PackageOperationTarget>(receiver_value).unwrap(),
        receiver
    );

    let old_export_target_error = serde_json::from_value::<PackageOperationTarget>(json!({
        "kind": "localExecutable",
        "target": {
            "file": {
                "fileIrIdentity": "file:users",
                "modulePath": "svc.users"
            },
            "executableIndex": 0,
            "symbol": "create",
            "signature": {
                "params": [],
                "returnType": { "kind": "builtin", "name": "string" },
                "maySuspend": false
            }
        }
    }))
    .expect_err("ExecutableExport-shaped package operation targets must fail closed")
    .to_string();
    assert!(
        old_export_target_error.contains("operation")
            || old_export_target_error.contains("unknown field"),
        "unexpected legacy package target error: {old_export_target_error}"
    );
}

#[test]
fn service_unit_rejects_legacy_binding_resolutions_field() {
    let unit = ServiceUnit::empty("users", "dev", "protocol:1");
    let mut value = serde_json::to_value(unit).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("bindingResolutions".to_string(), json!([]));
    assert_unknown_field_rejected::<ServiceUnit>(value);
}

#[test]
fn service_dependency_symbol_ref_uses_structured_operation_ref() {
    let symbol = ServiceDependencySymbolRef {
        dependency_ref: "remoteLlm".to_owned(),
        operation: instance_method_operation_ref(),
    };
    let value = serde_json::to_value(&symbol).unwrap();

    assert_eq!(value["dependencyRef"], "remoteLlm");
    assert_eq!(
        value["operation"]["operationAbiId"],
        "operation:remoteLlm:0.1.0:managedLlmService.sendChat"
    );
    assert!(value.get("operationAbiId").is_none());
    assert_eq!(
        serde_json::from_value::<ServiceDependencySymbolRef>(value).unwrap(),
        symbol
    );

    let old_symbol_error = serde_json::from_value::<ServiceDependencySymbolRef>(json!({
        "dependencyRef": "remoteLlm",
        "operationAbiId": "operation:old",
        "operation": "managedLlmService.sendChat"
    }))
    .expect_err("legacy service dependency symbol ref must fail closed")
    .to_string();
    assert!(
        old_symbol_error.contains("operationAbiId")
            || old_symbol_error.contains("operation")
            || old_symbol_error.contains("invalid type")
            || old_symbol_error.contains("unknown field"),
        "unexpected legacy service dependency symbol ref error: {old_symbol_error}"
    );
}

#[test]
fn type_refs_and_descriptors_reject_unknown_fields() {
    assert_unknown_field_rejected::<TypeRefIr>(json!({
        "kind": "builtin",
        "name": "string",
        "legacyName": "String"
    }));

    assert_unknown_field_rejected::<TypeDescriptorIr>(json!({
        "kind": "alias",
        "target": { "kind": "builtin", "name": "string" },
        "legacyTarget": "String"
    }));
}

#[test]
fn type_ref_union_serializes_items() {
    let value = serde_json::to_value(TypeRefIr::Union {
        items: vec![string_type(), number_type()],
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "kind": "union",
            "items": [
                { "kind": "builtin", "name": "string" },
                { "kind": "builtin", "name": "number" }
            ]
        })
    );
}

#[test]
fn type_descriptor_union_serializes_variants() {
    let value = serde_json::to_value(TypeDescriptorIr::Union {
        variants: vec![string_type(), number_type()],
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "kind": "union",
            "variants": [
                { "kind": "builtin", "name": "string" },
                { "kind": "builtin", "name": "number" }
            ]
        })
    );
}

#[test]
fn function_type_ref_round_trips_params_and_return_type() {
    let value = json!({
        "kind": "function",
        "params": [
            {
                "name": "input",
                "ty": { "kind": "builtin", "name": "string" }
            }
        ],
        "returnType": { "kind": "builtin", "name": "number" }
    });

    let decoded: TypeRefIr = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(
        decoded,
        TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: string_type(),
            }],
            return_type: Box::new(number_type()),
        }
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}

#[test]
fn any_interface_type_ref_round_trips_and_rejects_unknown_fields() {
    let interface = crate::interface_instantiation_ref_for_type_ref(&TypeRefIr::Native {
        name: "pkg.Reader".to_string(),
        args: vec![string_type()],
    });
    let value = json!({
        "kind": "anyInterface",
        "interface": interface,
    });

    let decoded: TypeRefIr = serde_json::from_value(value.clone()).unwrap();

    assert_eq!(
        decoded,
        TypeRefIr::AnyInterface {
            interface: interface.clone(),
        }
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);

    assert_unknown_field_rejected::<TypeRefIr>(json!({
        "kind": "anyInterface",
        "interface": interface,
        "legacyInterface": "Reader"
    }));
}

#[test]
fn interface_box_and_method_call_targets_round_trip() {
    let interface = crate::interface_instantiation_ref_for_type_ref(&TypeRefIr::Native {
        name: "pkg.Reader".to_string(),
        args: vec![string_type()],
    });
    let method_abi_id = crate::canonical_interface_method_abi_id(&interface, "read");
    let method_table = InterfaceMethodTablePlanIr {
        interface: interface.clone(),
        concrete_type: TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "svc".to_string(),
                symbol: "ReaderImpl".to_string(),
            },
        },
        slots: vec![InterfaceMethodSlotPlanIr {
            slot: 0,
            method_name: "read".to_string(),
            method_abi_id: method_abi_id.clone(),
            signature: InterfaceMethodSlotSignatureIr {
                params: vec![],
                return_type: string_type(),
            },
            target: InterfaceMethodSlotTargetIr {
                executable_index: 7,
                receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
            },
        }],
    };
    let expr = ExprIr::InterfaceBox {
        value: ExprRefIr { expression: 1 },
        interface: interface.clone(),
        source: BoxSourceIr::Local {
            concrete_type: method_table.concrete_type.clone(),
            method_table,
        },
    };
    let call_target = CallTargetIr::InterfaceMethod {
        interface,
        method_abi_id,
        slot: 0,
    };

    let expr_value = serde_json::to_value(&expr).unwrap();
    assert_eq!(
        serde_json::from_value::<ExprIr>(expr_value.clone()).unwrap(),
        expr
    );
    assert_eq!(serde_json::to_value(expr).unwrap(), expr_value);

    let call_value = serde_json::to_value(&call_target).unwrap();
    assert_eq!(
        serde_json::from_value::<CallTargetIr>(call_value.clone()).unwrap(),
        call_target
    );
    assert_eq!(serde_json::to_value(call_target).unwrap(), call_value);
}

#[test]
fn remote_interface_box_source_carries_operation_table_and_callee_identity() {
    let interface = crate::interface_instantiation_ref_for_type_ref(&TypeRefIr::Native {
        name: "pkg.Reader".to_string(),
        args: vec![string_type()],
    });
    let method_abi_id = crate::canonical_interface_method_abi_id(&interface, "read");
    let operation_abi_id = "operation:reader:read".to_string();
    let source = BoxSourceIr::Remote {
        dependency_ref: "readerService".to_string(),
        public_instance_key: "readers/default".to_string(),
        operations: RemoteOperationTablePlanIr {
            interface: interface.clone(),
            slots: vec![RemoteOperationSlotPlanIr {
                slot: 0,
                method_abi_id: method_abi_id.clone(),
                signature: InterfaceMethodSlotSignatureIr {
                    params: vec![FunctionTypeParamIr {
                        name: "input".to_string(),
                        ty: string_type(),
                    }],
                    return_type: string_type(),
                },
                operation_abi_id: operation_abi_id.clone(),
            }],
        },
        callee_protocol_identity: "protocol:reader".to_string(),
    };
    let value = json!({
        "kind": "remote",
        "dependencyRef": "readerService",
        "publicInstanceKey": "readers/default",
        "operations": {
            "interface": interface,
            "slots": [{
                "slot": 0,
                "methodAbiId": method_abi_id,
                "signature": {
                    "params": [{
                        "name": "input",
                        "ty": string_type()
                    }],
                    "returnType": string_type()
                },
                "operationAbiId": operation_abi_id
            }]
        },
        "calleeProtocolIdentity": "protocol:reader"
    });

    assert_eq!(
        serde_json::from_value::<BoxSourceIr>(value.clone()).unwrap(),
        source
    );
    assert_eq!(serde_json::to_value(source).unwrap(), value);
    assert_unknown_field_rejected::<BoxSourceIr>(json!({
        "kind": "remote",
        "dependencyRef": "readerService",
        "publicInstanceKey": "readers/default",
        "operations": {
            "interface": crate::interface_instantiation_ref_for_type_ref(&TypeRefIr::Native {
                name: "pkg.Reader".to_string(),
                args: vec![string_type()],
            }),
            "slots": []
        },
        "calleeProtocolIdentity": "protocol:reader",
        "payload": null
    }));
}

#[test]
fn legacy_union_shapes_fail_closed_when_canonical_field_is_missing() {
    let descriptor_error = serde_json::from_value::<TypeDescriptorIr>(json!({
        "kind": "union",
        "items": [{ "kind": "builtin", "name": "string" }]
    }))
    .expect_err("descriptor union must use variants, not items");
    assert!(
        descriptor_error.to_string().contains("variants"),
        "unexpected descriptor error: {descriptor_error}"
    );

    let type_ref_error = serde_json::from_value::<TypeRefIr>(json!({
        "kind": "union",
        "types": [{ "kind": "builtin", "name": "string" }]
    }))
    .expect_err("type-ref union must use items, not types");
    assert!(
        type_ref_error.to_string().contains("items"),
        "unexpected type-ref error: {type_ref_error}"
    );
}

#[test]
fn builtin_receiver_op_round_trips_canonical_shape() {
    let op = builtin_receiver_op_by_name("string", "concat").expect("string.concat op");
    let value = serde_json::to_value(op).unwrap();

    assert_eq!(
        value,
        json!({
            "receiver": "string",
            "method": "concat",
            "signatureVersion": 1,
            "canonicalKey": "receiver:string.concat@1"
        })
    );
    assert_eq!(
        serde_json::from_value::<BuiltinReceiverOp>(value).unwrap(),
        op
    );
}

#[test]
fn builtin_receiver_op_rejects_mismatched_canonical_key() {
    let error = serde_json::from_value::<BuiltinReceiverOp>(json!({
        "receiver": "string",
        "method": "concat",
        "signatureVersion": 1,
        "canonicalKey": "receiver:string.contains@1"
    }))
    .expect_err("mismatched canonical key should fail closed");

    assert!(
        error.to_string().contains("canonicalKey mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn builtin_receiver_op_rejects_unsupported_signature_version() {
    let error = serde_json::from_value::<BuiltinReceiverOp>(json!({
        "receiver": "string",
        "method": "concat",
        "signatureVersion": 99,
        "canonicalKey": "receiver:string.concat@99"
    }))
    .expect_err("unsupported signature version should fail closed");

    assert!(
        error
            .to_string()
            .contains("unsupported receiver builtin signatureVersion"),
        "unexpected error: {error}"
    );
}

#[test]
fn builtin_receiver_op_rejects_unknown_structured_op() {
    let error = serde_json::from_value::<BuiltinReceiverOp>(json!({
        "receiver": "Date",
        "method": "lowercase",
        "signatureVersion": 1,
        "canonicalKey": "receiver:Date.lowercase@1"
    }))
    .expect_err("unknown receiver/method pair should fail closed");

    assert!(
        error.to_string().contains("unknown receiver builtin op"),
        "unexpected error: {error}"
    );
}
