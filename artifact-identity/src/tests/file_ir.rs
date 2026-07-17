use super::*;

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
            type_ref: TypeRefIr::native("Credential"),
            type_name: "Credential".to_string(),
            collection_name: "credential".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::native("string"),
            },
            fields: vec![DbObjectFieldIr {
                name: "apiKey".to_string(),
                ty: TypeRefIr::native("string"),
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
fn service_call_table_and_instruction_indices_participate_in_file_ir_identity() {
    let base = service_call_file_ir_fixture();
    let baseline = file_ir_identity(&base).expect("valid service-call File IR identity");
    assert_eq!(
        baseline,
        "skiff-file-ir-v4:sha256:4b361e3f2a72ce1afe32eab0524070d616957b41bac4c437a0a3423667d85d5f"
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
        ty: TypeRefIr::native("void"),
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
