use super::*;
use skiff_artifact_model::{PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord};

#[test]
fn in_process_callback_resolves_only_declared_callback_contract_operations() {
    let callback_id = PackageSchemaTypeId::new("package-schema:callback");
    let callback_interface =
        ContractTypeRef::package_schema("example.callback", "api.Callback", callback_id.clone());
    let callback_ty = ContractTypeRef::AnyInterface {
        interface: Box::new(callback_interface.clone()),
        arguments: Vec::new(),
    };
    let operations = BTreeMap::from([(
        "invoke".to_string(),
        BoundaryCallbackOperation {
            parameters: Vec::new(),
            return_type: ContractTypeRef::builtin("bool"),
        },
    )]);
    let callback_record = Arc::new(PackageSchemaTypeRecord {
        package_id: "example.callback".to_string(),
        stable_schema_key: "api.Callback".to_string(),
        package_schema_type_id: callback_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::CallbackInterface {
                operations: operations.clone(),
            },
        },
    });
    let schema = BTreeMap::from([(callback_id, Arc::clone(&callback_record))]);
    let strong_count_before = Arc::strong_count(&callback_record);
    let (resolved_id, resolved_operations) = callback_contract(&callback_ty, &schema).unwrap();
    assert_eq!(resolved_id.package_id, "example.callback");
    assert_eq!(resolved_id.stable_schema_key, "api.Callback");
    assert_eq!(
        resolved_id.package_schema_type_id.as_str(),
        "package-schema:callback"
    );
    assert_eq!(resolved_operations, &operations);
    assert_eq!(Arc::strong_count(&callback_record), strong_count_before);
    assert!(matches!(
        callback_contract(&ContractTypeRef::builtin("string"), &schema),
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    ));
    assert!(matches!(
        callback_contract(&callback_interface, &schema),
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    ));
    assert!(matches!(
        callback_contract(
            &ContractTypeRef::AnyInterface {
                interface: Box::new(callback_interface.clone()),
                arguments: vec![ContractTypeRef::builtin("string")],
            },
            &schema
        ),
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    ));
    assert!(matches!(
        callback_contract(
            &ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![callback_ty],
            },
            &schema
        ),
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    ));
}

#[test]
fn in_process_callback_maps_wrong_tuple_to_stable_unavailable_error() {
    let error = callback_capability_error(CallbackCapabilityError::CapabilityUnavailable);
    assert!(matches!(
        error,
        RuntimeError::ProviderUnavailable { ref reason, .. }
            if reason == "CapabilityUnavailable"
    ));
}

#[test]
fn callback_contract_rejects_owner_key_id_descriptor_and_missing_alias_record() {
    let type_id = PackageSchemaTypeId::new("schema:callback-validation");
    let interface =
        ContractTypeRef::package_schema("example.callback", "api.Callback", type_id.clone());
    let ty = ContractTypeRef::AnyInterface {
        interface: Box::new(interface),
        arguments: Vec::new(),
    };
    let callback_record = || PackageSchemaTypeRecord {
        package_id: "example.callback".to_string(),
        stable_schema_key: "api.Callback".to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::CallbackInterface {
                operations: BTreeMap::new(),
            },
        },
    };
    for mutate in [
        |record: &mut PackageSchemaTypeRecord| record.package_id.push_str(".wrong"),
        |record: &mut PackageSchemaTypeRecord| record.stable_schema_key.push_str(".wrong"),
        |record: &mut PackageSchemaTypeRecord| {
            record.package_schema_type_id = PackageSchemaTypeId::new("wrong")
        },
    ] as [fn(&mut PackageSchemaTypeRecord); 3]
    {
        let mut record = callback_record();
        mutate(&mut record);
        assert!(
            callback_contract(&ty, &BTreeMap::from([(type_id.clone(), Arc::new(record))])).is_err()
        );
    }

    let mut non_callback = callback_record();
    non_callback.canonical_descriptor.descriptor = ContractTypeDescriptor::Enumeration {
        variants: vec!["value".to_string()],
    };
    assert!(matches!(
        callback_contract(
            &ty,
            &BTreeMap::from([(type_id.clone(), Arc::new(non_callback))])
        ),
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    ));

    let child_id = PackageSchemaTypeId::new("schema:missing-callback");
    let alias = PackageSchemaTypeRecord {
        package_id: "example.callback".to_string(),
        stable_schema_key: "api.Callback".to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Alias {
                target: ContractTypeRef::package_schema(
                    "example.callback",
                    "api.MissingCallback",
                    child_id,
                ),
            },
        },
    };
    assert!(matches!(
        callback_contract(&ty, &BTreeMap::from([(type_id, Arc::new(alias))])),
        Err(ServiceLinkableMaterializationError::MissingSchema { .. })
    ));
}

#[test]
fn callback_contract_unwraps_exact_existential_alias_representation_closure() {
    let callback_id = PackageSchemaTypeId::new("schema:callback-terminal");
    let representation_id = PackageSchemaTypeId::new("schema:callback-representation");
    let alias_id = PackageSchemaTypeId::new("schema:callback-alias");
    let package_ref = |key: &str, id: PackageSchemaTypeId| {
        ContractTypeRef::package_schema("example.callback", key, id)
    };
    let operations = BTreeMap::from([(
        "invoke".to_string(),
        BoundaryCallbackOperation {
            parameters: vec![ContractTypeRef::builtin("string")],
            return_type: ContractTypeRef::builtin("bool"),
        },
    )]);
    let record = |key: &str, id: PackageSchemaTypeId, descriptor: ContractTypeDescriptor| {
        (
            id.clone(),
            Arc::new(PackageSchemaTypeRecord {
                package_id: "example.callback".to_string(),
                stable_schema_key: key.to_string(),
                package_schema_type_id: id,
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor,
                },
            }),
        )
    };
    let schema = BTreeMap::from([
        record(
            "api.Callback",
            callback_id.clone(),
            ContractTypeDescriptor::CallbackInterface {
                operations: operations.clone(),
            },
        ),
        record(
            "api.CallbackRepresentation",
            representation_id.clone(),
            ContractTypeDescriptor::Representation {
                target: package_ref("api.Callback", callback_id.clone()),
            },
        ),
        record(
            "api.CallbackAlias",
            alias_id.clone(),
            ContractTypeDescriptor::Alias {
                target: package_ref("api.CallbackRepresentation", representation_id),
            },
        ),
    ]);
    let ty = ContractTypeRef::AnyInterface {
        interface: Box::new(package_ref("api.CallbackAlias", alias_id)),
        arguments: Vec::new(),
    };

    let (resolved, resolved_operations) =
        callback_contract(&ty, &schema).expect("closed alias chain should resolve");
    assert_eq!(resolved.package_id, "example.callback");
    assert_eq!(resolved.stable_schema_key, "api.Callback");
    assert_eq!(resolved.package_schema_type_id, callback_id);
    assert_eq!(resolved_operations, &operations);
}
