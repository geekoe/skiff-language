use super::*;

fn valid_publication() -> PublicationAbiUnit {
    let signature = CanonicalPublicCallableSignature {
        params: Vec::new(),
        return_type: TypeRefIr::native("string"),
        may_suspend: false,
    };
    let operation_id = public_function_operation_abi_id("run", &signature, &[], &BTreeMap::new())
        .expect("operation identity");
    let operation = OperationAbiRef {
        operation_abi_id: operation_id,
        kind: PublicationOperationKind::PublicFunction,
        public_path: "run".to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: "Run".to_string(),
    };
    let mut publication = PublicationAbiUnit::empty("example.com/pkg", "1.0.0", "");
    publication.operation_exports.push(operation.clone());
    publication.operation_abi.push(PublicationOperationAbi {
        operation: operation.clone(),
        public_signature: signature,
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    });
    publication
        .source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path: "run".to_string(),
            operation,
        });
    assign_publication_abi_identity(&mut publication).expect("valid publication");
    publication
}

#[test]
fn declared_publication_identity_is_recomputed() {
    let mut publication = valid_publication();
    validate_publication_abi_identity(&publication).expect("valid identity");
    publication.abi_identity = "skiff-publication-abi-v1:sha256:tampered".to_string();

    let error = validate_publication_abi_identity(&publication)
        .expect_err("tampered identity must fail")
        .to_string();
    assert!(error.contains("declared abiIdentity"), "{error}");
}

#[test]
fn duplicate_and_dangling_operation_refs_fail_closed() {
    let mut duplicate = valid_publication();
    duplicate
        .operation_exports
        .push(duplicate.operation_exports[0].clone());
    let error = validate_publication_abi_surface(&duplicate)
        .expect_err("duplicate operation must fail")
        .to_string();
    assert!(error.contains("duplicates operationAbiId"), "{error}");

    let mut dangling = valid_publication();
    dangling.source_call_operation_index[0]
        .operation
        .operation_abi_id = "skiff-operation-abi-v1:sha256:dangling".to_string();
    let error = validate_publication_abi_surface(&dangling)
        .expect_err("dangling source-call target must fail")
        .to_string();
    assert!(error.contains("targets dangling operationAbiId"), "{error}");
}

#[test]
fn descriptor_operation_identity_is_recomputed() {
    let mut publication = valid_publication();
    publication.operation_abi[0].public_signature.return_type = TypeRefIr::native("number");

    let error = validate_publication_abi_surface(&publication)
        .expect_err("descriptor tampering must fail")
        .to_string();
    assert!(
        error.contains("does not match descriptor identity"),
        "{error}"
    );
}

#[test]
fn schema_closure_keys_are_unique_and_closed() {
    let mut duplicate = valid_publication();
    let schema = PublicationSchemaType {
        abi_type_id: "type:Payload".to_string(),
        nameability: PublicationSchemaTypeNameability::PublicNameable,
        ty: TypeRefIr::native("Payload"),
        descriptor: None,
    };
    duplicate.schema_closure = vec![schema.clone(), schema.clone()];
    let error = validate_publication_abi_surface(&duplicate)
        .expect_err("duplicate schema key must fail")
        .to_string();
    assert!(error.contains("duplicates abiTypeId"), "{error}");

    let mut dangling = valid_publication();
    dangling.operation_abi[0].schema_closure.push(schema);
    let error = validate_publication_abi_surface(&dangling)
        .expect_err("operation schema key must close through publication schema")
        .to_string();
    assert!(
        error.contains("missing from publication schemaClosure"),
        "{error}"
    );
}

#[test]
fn public_instance_method_targets_are_validated() {
    let mut publication = valid_publication();
    let interface = interface_instantiation_ref(TypeRefIr::native("pkg.Reader"), Vec::new());
    let method_id = canonical_interface_method_abi_id(&interface, "read");
    let signature = CanonicalPublicCallableSignature {
        params: Vec::new(),
        return_type: TypeRefIr::native("string"),
        may_suspend: false,
    };
    let operation_id = public_instance_method_operation_abi_id(
        "reader.read",
        "reader",
        &interface,
        &method_id,
        &signature,
        &[],
        &BTreeMap::new(),
    )
    .expect("method operation identity");
    let operation = OperationAbiRef {
        operation_abi_id: operation_id,
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: "reader.read".to_string(),
        public_instance_key: Some("reader".to_string()),
        interface: Some(interface.clone()),
        method_abi_id: Some(method_id),
        display_name: "read".to_string(),
    };
    publication.operation_exports.push(operation.clone());
    publication.operation_abi.push(PublicationOperationAbi {
        operation: operation.clone(),
        public_signature: signature,
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    });
    publication
        .public_instances
        .push(PublicationPublicInstanceExport {
            public_instance_key: "reader".to_string(),
            interfaces: vec![interface],
            source_call_method_index: vec![SourceCallMethodIndexEntry {
                method_name: "read".to_string(),
                operation: operation.clone(),
            }],
            method_operations: vec![operation],
        });
    validate_publication_abi_surface(&publication).expect("valid method target");

    publication.public_instances[0].method_operations[0].operation_abi_id =
        "skiff-operation-abi-v1:sha256:dangling".to_string();
    let error = validate_publication_abi_surface(&publication)
        .expect_err("dangling method target must fail")
        .to_string();
    assert!(error.contains("targets dangling operationAbiId"), "{error}");
}
