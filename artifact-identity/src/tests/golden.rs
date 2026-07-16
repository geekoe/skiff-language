use super::*;

#[test]
fn complex_artifact_identity_outputs_match_exact_golden() {
    let schema_type = PublicationSchemaType {
        abi_type_id: "type:Payload".to_string(),
        nameability: PublicationSchemaTypeNameability::PublicNameable,
        ty: TypeRefIr::native("Payload"),
        descriptor: Some(skiff_artifact_model::TypeDescriptorIr::Alias {
            target: TypeRefIr::native("string"),
        }),
    };
    let public_signature = CanonicalPublicCallableSignature {
        params: vec![FunctionTypeParamIr {
            name: "input".to_string(),
            ty: TypeRefIr::native("Payload"),
        }],
        return_type: TypeRefIr::native("string"),
        may_suspend: true,
    };
    let interface = InterfaceInstantiationRef {
        interface_abi_id: "interface:Runner".to_string(),
        canonical_type_args: vec![TypeRefIr::native("Payload")],
    };
    let mut operation_metadata = BTreeMap::new();
    operation_metadata.insert(
        "effect".to_string(),
        MetadataValue::String("network".to_string()),
    );
    let function_id = public_function_operation_abi_id(
        "run",
        &public_signature,
        std::slice::from_ref(&schema_type),
        &operation_metadata,
    )
    .expect("public function operation identity");
    let instance_id = public_instance_method_operation_abi_id(
        "runner.execute",
        "runner",
        &interface,
        "method:execute",
        &public_signature,
        std::slice::from_ref(&schema_type),
        &operation_metadata,
    )
    .expect("public instance operation identity");
    let function = OperationAbiRef {
        operation_abi_id: function_id.clone(),
        kind: PublicationOperationKind::PublicFunction,
        public_path: "run".to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: "run".to_string(),
    };
    let instance = OperationAbiRef {
        operation_abi_id: instance_id.clone(),
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: "runner.execute".to_string(),
        public_instance_key: Some("runner".to_string()),
        interface: Some(interface.clone()),
        method_abi_id: Some("method:execute".to_string()),
        display_name: "execute".to_string(),
    };
    let mut publication = PublicationAbiUnit::empty("example.com/pkg", "1.2.3", "");
    publication.schema_closure.push(schema_type.clone());
    for operation in [&function, &instance] {
        publication.operation_exports.push(operation.clone());
        publication.operation_abi.push(PublicationOperationAbi {
            operation: operation.clone(),
            public_signature: public_signature.clone(),
            schema_closure: vec![schema_type.clone()],
            stream_effect_throw_config: operation_metadata.clone(),
        });
    }
    publication
        .source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path: "run".to_string(),
            operation: function.clone(),
        });
    publication
        .source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path: "runner.execute".to_string(),
            operation: instance.clone(),
        });
    publication
        .public_instances
        .push(PublicationPublicInstanceExport {
            public_instance_key: "runner".to_string(),
            interfaces: vec![interface],
            source_call_method_index: vec![SourceCallMethodIndexEntry {
                method_name: "execute".to_string(),
                operation: instance.clone(),
            }],
            method_operations: vec![instance],
        });
    publication.abi_identity =
        publication_abi_identity(&publication).expect("publication identity");

    let file = FileIrRef {
            file_ir_identity: "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            module_path: "pkg.main".to_string(),
            artifact_path: Some("units/files/pkg.json".to_string()),
            source_ast_hash: Some("source-hash".to_string()),
        };
    let resource = PublicationResourceRef {
        path: "prompts/system.md".to_string(),
        sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        byte_len: 128,
        content_type: Some("text/markdown".to_string()),
        artifact_path: Some("resources/sha256/bbbbbbbb".to_string()),
    };
    let mut package = PackageUnit::empty("example.com/pkg", "1.2.3", "", "");
    package.publication_abi = publication.clone();
    package.files.push(file.clone());
    package.resources.push(resource.clone());
    package.dependencies.push(PackageDependencyConstraint {
        id: "example.com/dependency".to_string(),
        version: "2.0.0".to_string(),
        alias: "dependency".to_string(),
        config: json!({ "mode": "strict" }),
    });
    package.config_and_effect_metadata.config.insert(
        "secret.token".to_string(),
        MetadataValue::String("configured".to_string()),
    );

    let mut service = ServiceUnit::empty("example.com/service", "3.0.0", "protocol:v3");
    service.service.display_name = Some("Identity Golden Service".to_string());
    service.service.metadata.insert(
        "tier".to_string(),
        MetadataValue::String("internal".to_string()),
    );
    service.publication_abi = publication;
    service.files.push(file.clone());
    service.resources.push(resource);
    service.operations.push(ServiceOperation::LocalExecutable(
        skiff_artifact_model::ServiceOperationTarget {
            operation: function,
            executable: OperationTargetRef {
                file_ref: file,
                executable_index: 2,
                callable_abi_id: "callable:run".to_string(),
                callable_kind: OperationCallableKind::PublicFunction,
            },
        },
    ));
    service.config.values.insert(
        "region".to_string(),
        MetadataValue::String("test".to_string()),
    );

    let actual = [
        function_id,
        instance_id,
        publication_abi_identity(&package.publication_abi).expect("publication identity golden"),
        package_build_identity(&package).expect("package build identity golden"),
        package_abi_identity(&package).expect("package ABI identity golden"),
        service_unit_identity(&service).expect("service unit identity golden"),
    ];
    assert_eq!(
            actual,
            [
                "skiff-operation-abi-v1:sha256:9892d2509d863917a3e61934a7b3b86600e2bc0283a1055c6c11c1cae9bf1561".to_string(),
                "skiff-operation-abi-v1:sha256:53c0d16e4d2cf8a8060438bd1c194e3e9da67b08612ec48ba138dcff0fb79c91".to_string(),
                "skiff-publication-abi-v1:sha256:7ed04a2c64c00aea4b06bc0b7917a6deb984990315b4f7c92909e03853ea6d15".to_string(),
                "skiff-package-build-v1:sha256:bec1c849738f962038d05479a80da88c4085c63dcd17173cc2975b1a8d2856d2".to_string(),
                "skiff-package-abi-v1:sha256:7ed04a2c64c00aea4b06bc0b7917a6deb984990315b4f7c92909e03853ea6d15".to_string(),
                "skiff-service-unit-v1:sha256:495cc9eb389e15e6ef30baf26c15606229973de823135aaaf9d06c814367268d".to_string(),
            ],
        );
}

#[test]
fn module_split_file_runtime_and_package_test_outputs_match_exact_golden() {
    let mut file = FileIrUnit::empty("identity.golden", "excluded-source-hash");
    file.source_map.sources.push(SourceMapSource {
        id: 7,
        path: "identity/golden.skiff".to_string(),
        module_path: "identity.golden".to_string(),
        source_ast_hash: Some("excluded-source-map-hash".to_string()),
    });

    let mut service = ServiceUnit::empty("example.com/golden", "1.0.0", "protocol:v1");
    service.publication_abi.abi_identity =
        publication_abi_identity(&service.publication_abi).expect("publication ABI identity");
    let runtime_bytes = runtime_program_service_unit_identity_bytes(&service)
        .expect("runtime program identity bytes");

    let actual = [
        file_ir_identity(&file).expect("File IR identity"),
        runtime_program_dynamic_build_id(
            &runtime_bytes,
            [
                "skiff-package-build-v1:sha256:aaaaaaaa",
                "skiff-package-build-v1:sha256:bbbbbbbb",
            ],
        ),
        package_test_build_identity(&package_test_assembly_fixture())
            .expect("package test build identity"),
    ];
    assert_eq!(
            actual,
            [
                "skiff-file-ir-v3:sha256:fe27d55db78af32190393edf73afe4f23ed771ebd9a21754e20ec95af51de523".to_string(),
                "skiff-service-build-v1:sha256:7ca79fb0d44e42415cd821fdd1f603187a1e77a30c81901b2758278561da350d".to_string(),
                "skiff-package-test-build-v1:sha256:dd5ac05cec9d68e3919a302dfc6485896db8b7a7b3bc3899456a316edbdb2a02".to_string(),
            ],
        );
}
