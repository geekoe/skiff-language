use super::*;
use skiff_artifact_model::FileIrPackageCallValidationError;

#[test]
fn file_ir_identity_omits_storage_identity_and_source_hashes() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    unit.file_ir_identity = "stale-file-ir-identity".to_string();
    unit.source_map.sources.push(SourceMapSource {
        id: 0,
        path: "internal/example.skiff".to_string(),
        module_path: "internal.example".to_string(),
        source_ast_hash: Some("source-map-ast-hash-a".to_string()),
    });

    let value = canonical_file_ir_identity_value(&unit).expect("identity value");

    assert!(value.get("fileIrIdentity").is_none());
    assert!(value.get("sourceAstHash").is_none());
    assert!(value
        .pointer("/sourceMap/sources/0/sourceAstHash")
        .is_none());
    assert_eq!(value["modulePath"], "internal.example");
    assert_eq!(
        value.pointer("/sourceMap/sources/0/path"),
        Some(&json!("internal/example.skiff"))
    );
}

#[test]
fn file_ir_identity_validation_rejects_stale_identity() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    unit.file_ir_identity = "stale-file-ir-identity".to_string();

    let error = validate_file_ir_identity(&unit).expect_err("stale identity must fail");

    assert!(matches!(
        error,
        ArtifactIdentityError::FileIrIdentityMismatch { .. }
    ));
    let computed = file_ir_identity(&unit).expect("computed identity");
    unit.file_ir_identity = computed;
    validate_file_ir_identity(&unit).expect("computed identity should validate");
}

#[test]
fn encrypted_db_field_storage_participates_in_file_ir_identity() {
    let mut identity = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    identity.declarations.db.insert(
        "Credential".to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::builtin("Credential"),
            type_name: "Credential".to_string(),
            collection_name: "credential".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![DbObjectFieldIr {
                name: "apiKey".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    let identity_hash = file_ir_hash(&identity).expect("identity field storage hash");
    identity
        .declarations
        .db
        .get_mut("Credential")
        .unwrap()
        .fields[0]
        .storage = DbFieldStorageIr::Encrypted;
    let encrypted_hash = file_ir_hash(&identity).expect("encrypted field storage hash");

    assert_ne!(identity_hash, encrypted_hash);
}

#[test]
fn actor_declaration_abi_participates_in_file_ir_identity() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    let abi = ActorAbiInput {
        actor_name: "DocHub".to_string(),
        actor_id_type: TypeRefIr::builtin("string"),
        fields: vec![ActorFieldIr {
            name: "nextSeq".to_string(),
            ty: TypeRefIr::builtin("number"),
            encoding: ActorFieldEncodingIr::CanonicalValueV1,
        }],
        public_methods: Vec::new(),
        actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
    };
    unit.actor_declarations.push(ActorDeclarationIr {
        actor_abi_identity: actor_abi_identity(&abi).expect("actor ABI identity"),
        actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity::new(
            "skiff-actor-implementation-v1:sha256:placeholder",
        ),
        abi,
        method_implementations: Default::default(),
    });
    let baseline = file_ir_identity(&unit).expect("actor File IR identity");

    unit.actor_declarations[0].abi.actor_id_type = TypeRefIr::builtin("integer");
    unit.actor_declarations[0].actor_abi_identity =
        actor_abi_identity(&unit.actor_declarations[0].abi).expect("changed actor ABI identity");
    let changed_id = file_ir_identity(&unit).expect("changed actor id File IR identity");
    assert_ne!(baseline, changed_id);

    unit.actor_declarations[0].abi.fields[0].ty = TypeRefIr::builtin("integer");
    unit.actor_declarations[0].actor_abi_identity =
        actor_abi_identity(&unit.actor_declarations[0].abi).expect("changed actor ABI identity");
    let changed_field = file_ir_identity(&unit).expect("changed actor field File IR identity");
    assert_ne!(changed_id, changed_field);
}

#[test]
fn service_call_table_and_instruction_indices_participate_in_file_ir_identity() {
    let base = service_call_file_ir_fixture();
    let baseline = file_ir_identity(&base).expect("valid service-call File IR identity");
    assert_eq!(
        baseline,
        "skiff-file-ir-v5:sha256:173750cd47164b1509d4e237bdc49dbc6382d6ebe6826c46aaf945b838ff37b6"
    );

    let mut changed_ref = base.clone();
    changed_ref.external_refs.service_call_refs[0].contract_operation_id =
        ContractOperationId::new("operation:echo-v2");
    assert_ne!(
        file_ir_identity(&changed_ref).unwrap(),
        file_ir_identity(&base).unwrap()
    );

    let mut changed_indices = base.clone();
    let expressions = &mut changed_indices.constants[0].body.expressions;
    for (expression, index) in expressions.iter_mut().zip([1, 0]) {
        let ExprIr::Call { call } = expression else {
            panic!("fixture call expression")
        };
        call.target = CallTargetIr::ServiceCall {
            service_call_ref_index: ServiceCallRefIndex::new(index),
        };
    }
    assert_ne!(file_ir_identity(&changed_indices).unwrap(), baseline);
}

#[test]
fn package_call_target_and_ref_fields_participate_in_file_ir_identity() {
    let base = package_call_file_ir_fixture();
    let baseline = file_ir_identity(&base).expect("valid package-call File IR identity");

    let mut changed_package_ref = base.clone();
    let replacement_package_ref = PackageRefIr::Dependency {
        dependency_ref: "tools-v2".to_string(),
    };
    let ExprIr::Call { call } = &mut changed_package_ref.constants[0].body.expressions[0] else {
        panic!("fixture call expression")
    };
    let CallTargetIr::PackageCallable { package_ref, .. } = &mut call.target else {
        panic!("fixture package-call target")
    };
    *package_ref = replacement_package_ref.clone();
    changed_package_ref.external_refs.package_callables[0].package_ref = replacement_package_ref;
    assert_ne!(file_ir_identity(&changed_package_ref).unwrap(), baseline);

    let mut changed_callable = base.clone();
    let replacement_callable_id = PackageCallableId::new("callable:other.echo");
    let ExprIr::Call { call } = &mut changed_callable.constants[0].body.expressions[0] else {
        panic!("fixture call expression")
    };
    let CallTargetIr::PackageCallable {
        package_callable_id,
        ..
    } = &mut call.target
    else {
        panic!("fixture package-call target")
    };
    *package_callable_id = replacement_callable_id.clone();
    changed_callable.external_refs.package_callables[0].package_callable_id =
        replacement_callable_id;
    assert_ne!(
        file_ir_identity(&changed_callable).unwrap(),
        baseline,
        "owner-qualified callable identities must not collapse on the shared display suffix"
    );
}

#[test]
fn file_ir_identity_reuses_canonical_package_call_validation() {
    let mut missing = package_call_file_ir_fixture();
    missing.external_refs.package_callables.clear();
    assert!(matches!(
        package_call_identity_validation_error(&missing),
        FileIrPackageCallValidationError::MissingRef { .. }
    ));

    let mut orphan = package_call_file_ir_fixture();
    orphan.constants[0].body.expressions.clear();
    assert!(matches!(
        package_call_identity_validation_error(&orphan),
        FileIrPackageCallValidationError::OrphanRef { .. }
    ));

    let mut mismatch = package_call_file_ir_fixture();
    let ExprIr::Call { call } = &mut mismatch.constants[0].body.expressions[0] else {
        panic!("fixture call expression")
    };
    let CallTargetIr::PackageCallable { package_ref, .. } = &mut call.target else {
        panic!("fixture package-call target")
    };
    *package_ref = PackageRefIr::PackageId {
        package_id: "example.com/tools".to_string(),
    };
    assert!(matches!(
        package_call_identity_validation_error(&mismatch),
        FileIrPackageCallValidationError::FieldMismatch { .. }
    ));

    let mut duplicate = package_call_file_ir_fixture();
    duplicate
        .external_refs
        .package_callables
        .push(duplicate.external_refs.package_callables[0].clone());
    assert!(matches!(
        package_call_identity_validation_error(&duplicate),
        FileIrPackageCallValidationError::DuplicateRef { .. }
    ));
}

#[test]
fn file_ir_identity_reuses_canonical_service_call_validation() {
    let mut orphan = service_call_file_ir_fixture();
    orphan.constants[0].body.expressions.pop();

    assert!(matches!(
        file_ir_identity(&orphan),
        Err(ArtifactIdentityError::InvalidFileIrServiceCalls(
            skiff_artifact_model::FileIrServiceCallValidationError::OrphanRef { .. }
        ))
    ));
}

fn service_call_file_ir_fixture() -> FileIrUnit {
    let mut unit = FileIrUnit::empty("consumer.main", "source-ast-hash");
    unit.external_refs.service_call_refs = vec![
        ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: ContractOperationId::new("operation:echo"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        },
        ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: ContractOperationId::new("operation:health"),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
        },
    ];
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "calls".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: skiff_artifact_model::ExecutableBody {
            blocks: Vec::new(),
            statements: Vec::new(),
            expressions: [0, 1]
                .into_iter()
                .map(|index| ExprIr::Call {
                    call: CallIr {
                        target: CallTargetIr::ServiceCall {
                            service_call_ref_index: ServiceCallRefIndex::new(index),
                        },
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                })
                .collect(),
        },
        source_span: None,
    });
    unit
}

fn package_call_file_ir_fixture() -> FileIrUnit {
    let package_ref = PackageRefIr::Dependency {
        dependency_ref: "tools".to_string(),
    };
    let package_callable_id = PackageCallableId::new("callable:tools.echo");
    let mut unit = FileIrUnit::empty("consumer.main", "source-ast-hash");
    unit.external_refs.package_callables = vec![PackageCallableRef {
        package_ref: package_ref.clone(),
        package_callable_id: package_callable_id.clone(),
    }];
    unit.constants.push(skiff_artifact_model::ConstIr {
        name: "package-call".to_string(),
        ty: TypeRefIr::builtin("void"),
        body: skiff_artifact_model::ExecutableBody {
            blocks: Vec::new(),
            statements: Vec::new(),
            expressions: vec![ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::PackageCallable {
                        package_ref,
                        package_callable_id,
                    },
                    args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    });
    unit
}

fn package_call_identity_validation_error(unit: &FileIrUnit) -> FileIrPackageCallValidationError {
    match file_ir_identity(unit) {
        Err(ArtifactIdentityError::InvalidFileIrPackageCalls(error)) => error,
        result => panic!("expected canonical package-call validation error, got {result:?}"),
    }
}
