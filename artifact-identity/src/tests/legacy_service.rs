use super::*;

#[test]
fn service_unit_storage_identity_wraps_canonical_service_unit() {
    let mut publication_abi = publication_abi_fixture();
    publication_abi.abi_identity =
        publication_abi_identity(&publication_abi).expect("publication ABI identity");
    let mut unit = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
    unit.publication_abi = publication_abi;
    unit.files.push(FileIrRef {
            file_ir_identity: "skiff-file-ir-v5:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            module_path: "svc.main".to_string(),
            artifact_path: Some("units/files/svc.json".to_string()),
            source_ast_hash: Some("source".to_string()),
        });

    let value = service_unit_identity_value(&unit).expect("service unit identity value");
    assert_eq!(
        value.pointer("/identitySchema"),
        Some(&json!("skiff-service-unit-identity-v1"))
    );
    assert_eq!(
        value.pointer("/unit/service/id"),
        Some(&json!("example.com/svc"))
    );
    assert_eq!(
            value.pointer("/unit/files/0/fileIrIdentity"),
            Some(&json!("skiff-file-ir-v5:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))
        );

    let hash = service_unit_hash(&unit).expect("service unit hash");
    let identity = service_unit_identity(&unit).expect("service unit identity");
    assert_eq!(identity, format!("{SERVICE_UNIT_IDENTITY_PREFIX}:{hash}"));
    assert_eq!(
        service_unit_identity_bytes(&unit).expect("service unit identity bytes"),
        serde_json::to_vec(&value).expect("service unit identity value bytes")
    );

    let mut changed = unit;
    changed.protocol_identity = "protocol:changed".to_string();
    assert_ne!(
        identity,
        service_unit_identity(&changed).expect("changed service unit identity")
    );
}
