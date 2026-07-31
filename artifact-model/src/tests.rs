use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::json;

use crate::{builtin_receiver_op_by_name, BuiltinReceiverOp};
use crate::{DbFieldStorageIr, DbObjectFieldIr};

use crate::{
    validate_file_ir_db_indexes, BlockIr, BoxSourceIr, CallTargetIr, ConstIr, DbDeclarationIr,
    DbIndexDirectionIr, DbIndexFieldIr, DbIndexIr, DbMetadataIndexIr, DbObjectKeyIr,
    DbObjectKindIr, ExecutableBody, ExecutableIr, ExecutableKind, ExecutableLinkTargetIr, ExprIr,
    ExprRefIr, ExternalRefTable, FieldPathIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
    GatewayRoute, InterfaceInstantiationRef, InterfaceMethodSignature, InterfaceMethodSlotPlanIr,
    InterfaceMethodSlotSignatureIr, InterfaceMethodSlotTargetIr, InterfaceMethodTablePlanIr,
    LiteralIr, LocalReceiverExecutableRef, NamedUnionBranchIr, OperationAbiRef,
    OperationCallableKind, OperationConstReceiverRef, OperationTargetRef, PackageCallableId,
    PackageCallableRef, PackageOperationTarget, PackageRefIr, PackageSymbolRef,
    PublicationOperationKind, ReceiverCallAbi, RecoverableAdapterSchemaCompatibility,
    RecoverableArtifactMetadata, RecoverableCustomRestorePlan, RecoverableExpectedTypePlan,
    RecoverableExpectedTypeRoot, RecoverableFieldIdentityRef,
    RecoverableInterfaceMethodIdentityRef, RecoverableInterfaceProjectionIdentityRef,
    RecoverableNativeAdapterOwner, RecoverableNativeAdapterPlan, RecoverableRestoreCapability,
    RecoverableTypeIdentityRef, RecoverableUnionBranchIdentityRef, RemoteOperationSlotPlanIr,
    RemoteOperationTablePlanIr, ServiceDependencySymbolRef, ServiceSymbolRef, SlotLayout,
    SourceMapSource, SourceMapSpan, SourcePosition, SourceSpanRef, StmtIr, StmtRefIr, TypeDeclIr,
    TypeDescriptorIr, TypeLinkTargetIr, TypeRefIr, FILE_IR_FORMAT_VERSION,
    FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION,
};

#[test]
fn interface_method_signature_excludes_suspend_flag_and_is_strict() {
    let method = InterfaceMethodSignature {
        name: "load".to_string(),
        type_params: vec!["T".to_string()],
        params: vec![FunctionTypeParamIr {
            name: "value".to_string(),
            ty: TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        }],
        return_type: TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    };
    let wire = serde_json::to_value(&method).unwrap();
    assert!(wire.get("maySuspend").is_none());
    assert_eq!(
        serde_json::from_value::<InterfaceMethodSignature>(wire.clone()).unwrap(),
        method
    );

    let mut legacy = wire.clone();
    legacy
        .as_object_mut()
        .unwrap()
        .insert("maySuspend".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<InterfaceMethodSignature>(legacy).is_err());
    let mut unknown = wire;
    unknown
        .as_object_mut()
        .unwrap()
        .insert("suspends".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<InterfaceMethodSignature>(unknown).is_err());
}

fn string_type() -> TypeRefIr {
    TypeRefIr::builtin("string")
}

fn number_type() -> TypeRefIr {
    TypeRefIr::builtin("number")
}

fn reader_interface_ref() -> InterfaceInstantiationRef {
    InterfaceInstantiationRef {
        interface_abi_id: "interface:pkg.Reader".to_string(),
        canonical_type_args: vec![string_type()],
    }
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

#[test]
fn recoverable_expected_type_compatibility_matrix_fails_closed() {
    let base = recoverable_identity_plan(TypeRefIr::builtin("User"), "type:user");
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

    let other_nominal = recoverable_identity_plan(TypeRefIr::builtin("User"), "type:account");
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
        &recoverable_type_ref_plan(TypeRefIr::builtin("number")),
        &recoverable_type_ref_plan(TypeRefIr::builtin("string"))
    ));
}

#[test]
fn recoverable_custom_and_native_plans_validate_required_fields() {
    let mut metadata = RecoverableArtifactMetadata::default();
    metadata.custom_restore_plans.insert(
        "restore:user".to_string(),
        RecoverableCustomRestorePlan {
            concrete_type_identity: String::new(),
            durable_state_type_plan: recoverable_type_ref_plan(TypeRefIr::builtin("Json")),
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
            durable_state_type_plan: recoverable_type_ref_plan(TypeRefIr::builtin("Json")),
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
        "durableStateTypePlan": recoverable_type_ref_plan(TypeRefIr::builtin("Json")),
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
        "durableStateTypePlan": recoverable_type_ref_plan(TypeRefIr::builtin("Json")),
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
fn db_index_artifacts_reject_retired_partial_where_payloads() {
    let canonical = json!({
        "name": "byOwner",
        "unique": false,
        "fields": []
    });
    serde_json::from_value::<DbIndexIr>(canonical.clone())
        .expect("ordinary File IR index must decode");
    serde_json::from_value::<DbMetadataIndexIr>(canonical.clone())
        .expect("ordinary runtime metadata index must decode");

    let mut retired = canonical;
    retired["where"] = json!({"kind": "identifier", "name": "active"});
    assert_unknown_field_rejected::<DbIndexIr>(retired.clone());
    assert_unknown_field_rejected::<DbMetadataIndexIr>(retired);
}

#[test]
fn file_ir_db_indexes_require_unique_names_fields_and_ordered_specs() {
    let mut unit = FileIrUnit::empty("main", "source");
    let field = |name: &str, direction| DbIndexFieldIr {
        field: FieldPathIr {
            text: name.to_string(),
            segments: name.split('.').map(str::to_string).collect(),
        },
        direction,
    };
    unit.declarations.db.insert(
        "Thread".to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::builtin("Thread"),
            type_name: "Thread".to_string(),
            collection_name: "thread".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![DbObjectFieldIr {
                name: "owner".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: vec![
                DbIndexIr {
                    name: "byOwner".to_string(),
                    unique: false,
                    fields: vec![field("owner", DbIndexDirectionIr::Asc)],
                },
                DbIndexIr {
                    name: "ownerUnique".to_string(),
                    unique: true,
                    fields: vec![field("owner", DbIndexDirectionIr::Asc)],
                },
            ],
            source_span: None,
        },
    );

    let error = validate_file_ir_db_indexes(&unit)
        .expect_err("same ordered key under another logical name must fail");
    assert!(
        error
            .to_string()
            .contains("declare the same ordered key specification"),
        "unexpected error: {error}"
    );

    unit.declarations
        .db
        .get_mut("Thread")
        .expect("fixture DB")
        .indexes[1]
        .fields[0]
        .direction = DbIndexDirectionIr::Desc;
    validate_file_ir_db_indexes(&unit).expect("direction is part of the ordered key");

    let owner = &mut unit
        .declarations
        .db
        .get_mut("Thread")
        .expect("fixture DB")
        .fields[0];
    owner.ty = TypeRefIr::Union {
        items: vec![
            TypeRefIr::Literal {
                value: crate::types::LiteralIr::String {
                    value: "open".to_string(),
                },
            },
            TypeRefIr::Literal {
                value: crate::types::LiteralIr::String {
                    value: "closed".to_string(),
                },
            },
        ],
    };
    validate_file_ir_db_indexes(&unit)
        .expect("string literal unions have one scalar Mongo representation");

    let owner = &mut unit
        .declarations
        .db
        .get_mut("Thread")
        .expect("fixture DB")
        .fields[0];
    owner.ty = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    };
    let error = validate_file_ir_db_indexes(&unit)
        .expect_err("container-valued index fields must fail artifact admission");
    assert!(
        error.to_string().contains("indexable scalar"),
        "unexpected error: {error}"
    );
    let owner = &mut unit
        .declarations
        .db
        .get_mut("Thread")
        .expect("fixture DB")
        .fields[0];
    owner.ty = TypeRefIr::builtin("string");
    owner.storage = DbFieldStorageIr::Encrypted;
    let error = validate_file_ir_db_indexes(&unit)
        .expect_err("encrypted index fields must fail artifact admission");
    assert!(
        error.to_string().contains("encrypted storage field"),
        "unexpected error: {error}"
    );
    unit.declarations
        .db
        .get_mut("Thread")
        .expect("fixture DB")
        .fields[0]
        .storage = DbFieldStorageIr::Identity;

    unit.declarations
        .db
        .get_mut("Thread")
        .expect("fixture DB")
        .indexes[1]
        .fields
        .push(field("owner", DbIndexDirectionIr::Asc));
    let error = validate_file_ir_db_indexes(&unit)
        .expect_err("one compound index cannot repeat a logical path");
    assert!(
        error.to_string().contains("declared more than once"),
        "unexpected error: {error}"
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
fn empty_file_ir_uses_canonical_identity_versions_and_external_refs() {
    let unit = FileIrUnit::empty("svc.empty", "source:empty");

    assert_eq!(FILE_IR_SCHEMA_VERSION, "skiff-file-ir-v11");
    assert_eq!(FILE_IR_FORMAT_VERSION, "skiff-file-ir-format-v7");
    assert_eq!(FILE_IR_OPCODE_TABLE_VERSION, "skiff-opcode-table-v2");
    assert_eq!(unit.schema_version, FILE_IR_SCHEMA_VERSION);
    assert_eq!(unit.ir_format_version, FILE_IR_FORMAT_VERSION);
    assert_eq!(unit.opcode_table_version, FILE_IR_OPCODE_TABLE_VERSION);
    assert!(unit.external_refs.package_callables.is_empty());
    assert_eq!(
        serde_json::to_value(&unit.external_refs).unwrap(),
        json!({ "serviceCallRefs": [] })
    );

    let wire = serde_json::to_value(unit).unwrap();
    assert_eq!(wire["schemaVersion"], FILE_IR_SCHEMA_VERSION);
    assert_eq!(wire["irFormatVersion"], FILE_IR_FORMAT_VERSION);
    assert_eq!(wire["opcodeTableVersion"], FILE_IR_OPCODE_TABLE_VERSION);
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
                Some(TypeRefIr::Builtin {
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
fn while_round_trips_canonical_artifact_shape() {
    let value = json!({
        "kind": "while",
        "condition": { "expression": 0 },
        "body": "while_body"
    });

    let decoded: StmtIr = serde_json::from_value(value.clone()).unwrap();
    match &decoded {
        StmtIr::While { condition, body } => {
            assert_eq!(condition.expression, 0);
            assert_eq!(body, "while_body");
        }
        other => panic!("expected while statement, got {other:?}"),
    }
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}

#[test]
fn type_decl_ir_round_trips_named_union_branch_identity_input() {
    let declaration = TypeDeclIr {
        name: "Outcome".to_string(),
        descriptor: TypeDescriptorIr::Union {
            branches: vec![NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type: TypeRefIr::Record {
                    fields: BTreeMap::from([(
                        "kind".to_string(),
                        TypeRefIr::Literal {
                            value: LiteralIr::String {
                                value: "ok".to_string(),
                            },
                        },
                    )]),
                },
                discriminator_field: "kind".to_string(),
                discriminator_value: "ok".to_string(),
            }],
        },
        type_params: vec!["T".to_string()],
        implements: Vec::new(),
        source_span: None,
    };

    let value = serde_json::to_value(&declaration).unwrap();
    assert_eq!(value["descriptor"]["kind"], "union");
    assert_eq!(
        value["descriptor"]["branches"][0]["kind"],
        "syntheticDiscriminator"
    );
    assert_eq!(
        serde_json::from_value::<TypeDeclIr>(value).unwrap(),
        declaration
    );
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
fn call_target_rejects_retired_external_service_symbol() {
    let err = serde_json::from_value::<CallTargetIr>(json!({
        "kind": "externalServiceSymbol",
        "symbol": {
            "modulePath": "internal.worker",
            "symbol": "drain"
        }
    }))
    .expect_err("artifact CallTargetIr must not accept unresolved source call targets")
    .to_string();

    assert!(
        err.contains("unknown variant `externalServiceSymbol`"),
        "unexpected retired call target error: {err}"
    );
}

#[test]
fn package_callable_target_ref_and_table_use_canonical_identity_shape() {
    let package_ref = PackageRefIr::Dependency {
        dependency_ref: "tools".to_owned(),
    };
    let package_callable_id = PackageCallableId::new("callable:tools.ping");
    let target = CallTargetIr::PackageCallable {
        package_ref: package_ref.clone(),
        package_callable_id: package_callable_id.clone(),
    };
    let target_wire = json!({
        "kind": "packageCallable",
        "packageRef": {
            "kind": "dependency",
            "dependencyRef": "tools"
        },
        "packageCallableId": "callable:tools.ping"
    });

    assert_eq!(serde_json::to_value(&target).unwrap(), target_wire);
    assert_eq!(
        serde_json::from_value::<CallTargetIr>(target_wire).unwrap(),
        target
    );

    let callable_ref = PackageCallableRef {
        package_ref,
        package_callable_id,
    };
    let callable_ref_wire = json!({
        "packageRef": {
            "kind": "dependency",
            "dependencyRef": "tools"
        },
        "packageCallableId": "callable:tools.ping"
    });
    assert_eq!(
        serde_json::to_value(&callable_ref).unwrap(),
        callable_ref_wire
    );
    assert_eq!(
        serde_json::from_value::<PackageCallableRef>(callable_ref_wire.clone()).unwrap(),
        callable_ref
    );

    let table = ExternalRefTable {
        package_callables: vec![callable_ref],
        ..ExternalRefTable::default()
    };
    let table_wire = json!({
        "serviceCallRefs": [],
        "packageCallables": [callable_ref_wire]
    });
    assert_eq!(serde_json::to_value(&table).unwrap(), table_wire);
    assert_eq!(
        serde_json::from_value::<ExternalRefTable>(table_wire).unwrap(),
        table
    );
}

#[test]
fn package_callable_target_rejects_legacy_and_noncanonical_shapes() {
    let package_ref = json!({
        "kind": "dependency",
        "dependencyRef": "tools"
    });
    let operation = operation_ref(
        "operation:tools:ping",
        PublicationOperationKind::PublicFunction,
        "tools.ping",
    );

    for invalid in [
        json!({
            "kind": "packageSymbol",
            "packageRef": package_ref,
            "operation": operation
        }),
        json!({
            "kind": "packageCallable",
            "packageRef": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "packageCallableId": "callable:tools.ping",
            "expectedLocalAbi": "local-abi:tools"
        }),
        json!({
            "kind": "packageCallable",
            "packageRef": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "operation": operation_ref(
                "operation:tools:ping",
                PublicationOperationKind::PublicFunction,
                "tools.ping",
            )
        }),
        json!({
            "kind": "packageCallable",
            "packageRef": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "packageCallableId": "callable:tools.ping",
            "symbolPath": "tools.ping"
        }),
        json!({
            "kind": "packageCallable",
            "packageRef": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "packageCallableId": "callable:tools.ping",
            "operationTargetRef": {
                "fileRef": {
                    "fileIrIdentity": "file:tools",
                    "modulePath": "tools"
                },
                "executableIndex": 0,
                "callableAbiId": "abi:tools.ping",
                "callableKind": "freeFunction"
            }
        }),
    ] {
        assert!(
            serde_json::from_value::<CallTargetIr>(invalid).is_err(),
            "noncanonical package call target must be rejected"
        );
    }
}

#[test]
fn package_callable_ref_and_table_reject_legacy_and_unknown_fields() {
    for forbidden_field in [
        "expectedLocalAbi",
        "operation",
        "symbolPath",
        "operationTargetRef",
    ] {
        let mut value = json!({
            "packageRef": {
                "kind": "dependency",
                "dependencyRef": "tools"
            },
            "packageCallableId": "callable:tools.ping"
        });
        value
            .as_object_mut()
            .unwrap()
            .insert(forbidden_field.to_owned(), json!("legacy"));
        assert_unknown_field_rejected::<PackageCallableRef>(value);
    }

    assert_unknown_field_rejected::<ExternalRefTable>(json!({
        "serviceCallRefs": [],
        "packageOperationSymbols": []
    }));
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
fn gateway_route_identity_is_method_and_path() {
    let route = GatewayRoute {
        operation: "http.route.internal.session.read".to_string(),
        operation_abi_id: "operation:1".to_string(),
        method: "get".to_string(),
        path: "/session".to_string(),
    };

    assert_eq!(route.route_identity(), "GET /session");
}

#[test]
fn db_field_storage_uses_compact_identity_and_explicit_encrypted_json() {
    let identity = DbObjectFieldIr {
        name: "name".to_string(),
        ty: string_type(),
        storage: DbFieldStorageIr::Identity,
    };
    let encrypted = DbObjectFieldIr {
        storage: DbFieldStorageIr::Encrypted,
        ..identity.clone()
    };

    let identity_json = serde_json::to_value(&identity).unwrap();
    let encrypted_json = serde_json::to_value(&encrypted).unwrap();
    assert!(identity_json.get("storage").is_none());
    assert_eq!(encrypted_json["storage"], "encrypted");
    assert_eq!(
        serde_json::from_value::<DbObjectFieldIr>(identity_json).unwrap(),
        identity
    );
    assert_eq!(
        serde_json::from_value::<DbObjectFieldIr>(encrypted_json).unwrap(),
        encrypted
    );
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

    serde_json::from_value::<TypeRefIr>(json!({
        "kind": "native",
        "name": "string"
    }))
    .expect_err("legacy native type-ref wire must fail closed");

    serde_json::from_value::<TypeDescriptorIr>(json!({
        "kind": "external",
        "symbol": "opaque.Handle"
    }))
    .expect_err("removed native type descriptor wire must fail closed");
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
        branches: vec![
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::LocalType { type_index: 1 },
            },
            NamedUnionBranchIr::Literal {
                value: LiteralIr::String {
                    value: "other".to_string(),
                },
            },
        ],
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "kind": "union",
            "branches": [
                {
                    "kind": "concreteNominal",
                    "nominalType": { "kind": "localType", "typeIndex": 1 }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "other" }
                }
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
    let interface = reader_interface_ref();
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
    let interface = reader_interface_ref();
    let method_abi_id = "method:interface:pkg.Reader:read".to_string();
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
    let interface = reader_interface_ref();
    let method_abi_id = "method:interface:pkg.Reader:read".to_string();
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
            "interface": reader_interface_ref(),
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
    .expect_err("descriptor union must use branches, not items");
    assert!(
        descriptor_error.to_string().contains("branches"),
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
