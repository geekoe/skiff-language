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
