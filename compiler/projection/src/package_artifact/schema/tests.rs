use skiff_artifact_model::{
    FileIrRef, NominalTypeRefBaseIr, PackageBuildId, PackageExportIndex, PackageLocalAbiIdentity,
    PackageRefIr, ServiceSymbolRef, TypeExport,
};

use super::*;
use crate::package_artifact::export_links::ProjectedPackageExportLinks;

fn exports(descriptor: TypeDescriptorIr) -> ProjectedPackageExportLinks {
    exports_with_descriptor(Some(descriptor))
}

fn alias_exports(descriptor: TypeDescriptorIr) -> ProjectedPackageExportLinks {
    let mut exports = exports(descriptor);
    exports.alias_types.insert("example.pkg/User".to_string());
    exports
}

fn exports_with_descriptor(descriptor: Option<TypeDescriptorIr>) -> ProjectedPackageExportLinks {
    ProjectedPackageExportLinks {
        exports: PackageExportIndex {
            types: BTreeMap::from([(
                "example.pkg/User".to_string(),
                TypeExport {
                    file: FileIrRef::new("file", "models"),
                    type_index: 0,
                    symbol: "User".to_string(),
                    is_interface: false,
                    descriptor,
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
            ..PackageExportIndex::default()
        },
        public_instances: Vec::new(),
        alias_types: BTreeSet::new(),
    }
}

#[test]
fn public_path_owns_the_emitted_package_schema_record() {
    let projected = project_package_schema(
        "example.pkg",
        &exports(TypeDescriptorIr::Record {
            fields: BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
        }),
        &[],
    )
    .unwrap();
    let entry = &projected.index.types["User"];
    let record = &projected.records[&entry.package_schema_type_id];
    assert_eq!(record.package_id, "example.pkg");
    assert_eq!(record.stable_schema_key, "User");
    assert_eq!(entry.public_path.as_deref(), Some("User"));
}

#[test]
fn non_generic_interface_keeps_callback_schema_operations() {
    let mut interface = exports(TypeDescriptorIr::Interface);
    let export = interface.exports.types.get_mut("example.pkg/User").unwrap();
    export.is_interface = true;
    export.interface_methods = vec![InterfaceMethodSignature {
        name: "read".to_string(),
        type_params: Vec::new(),
        params: vec![skiff_artifact_model::FunctionTypeParamIr {
            name: "key".to_string(),
            ty: TypeRefIr::builtin("string"),
        }],
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: Some(TypeRefIr::builtin("Self")),
    }];

    let projected = project_package_schema("example.pkg", &interface, &[]).unwrap();
    let entry = &projected.index.types["User"];
    let record = &projected.records[&entry.package_schema_type_id];
    let ContractTypeDescriptor::CallbackInterface { operations } =
        &record.canonical_descriptor.descriptor
    else {
        panic!("interface must retain callback-interface schema kind");
    };
    assert_eq!(
        operations["read"],
        BoundaryCallbackOperation {
            parameters: vec![ContractTypeRef::builtin("string")],
            return_type: ContractTypeRef::builtin("string"),
        }
    );
}

#[test]
fn explicit_source_self_is_not_a_callback_payload_parameter() {
    let mut interface = exports(TypeDescriptorIr::Interface);
    let export = interface.exports.types.get_mut("example.pkg/User").unwrap();
    export.is_interface = true;
    export.interface_methods = vec![InterfaceMethodSignature {
        name: "read".to_string(),
        type_params: Vec::new(),
        params: vec![
            skiff_artifact_model::FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::builtin("Self"),
            },
            skiff_artifact_model::FunctionTypeParamIr {
                name: "key".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
        ],
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }];

    let projected = project_package_schema("example.pkg", &interface, &[]).unwrap();
    let entry = &projected.index.types["User"];
    let record = &projected.records[&entry.package_schema_type_id];
    let ContractTypeDescriptor::CallbackInterface { operations } =
        &record.canonical_descriptor.descriptor
    else {
        panic!("interface must retain callback-interface schema kind");
    };
    assert_eq!(
        operations["read"].parameters,
        vec![ContractTypeRef::builtin("string")]
    );
}

#[test]
fn package_schema_interface_with_applied_implicit_receiver_is_omitted() {
    let mut interface = exports(TypeDescriptorIr::Interface);
    let export = interface.exports.types.get_mut("example.pkg/User").unwrap();
    export.is_interface = true;
    export.interface_methods = vec![InterfaceMethodSignature {
        name: "read".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: Some(TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "models".to_string(),
                    symbol: "Box".to_string(),
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        }),
    }];

    let projected = project_package_schema("example.pkg", &interface, &[]).unwrap();

    assert!(projected.index.types.is_empty());
    assert!(projected.records.is_empty());
    assert!(projected.refs_by_source.is_empty());
}

#[test]
fn package_schema_public_generic_declaration_is_omitted_without_partial_records() {
    let mut generic = exports(TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "value".to_string(),
            TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        )]),
    });
    generic
        .exports
        .types
        .get_mut("example.pkg/User")
        .unwrap()
        .type_params = vec!["T".to_string()];

    let projected = project_package_schema("example.pkg", &generic, &[]).unwrap();

    assert!(projected.index.types.is_empty());
    assert!(projected.records.is_empty());
    assert!(projected.refs_by_source.is_empty());
}

#[test]
fn package_schema_public_generic_declaration_kinds_are_uniformly_omitted() {
    let file = FileIrRef::new("file", "models");
    let export = |type_index, symbol: &str, descriptor, type_params| TypeExport {
        file: file.clone(),
        type_index,
        symbol: symbol.to_string(),
        is_interface: false,
        descriptor: Some(descriptor),
        type_params,
        interface_methods: Vec::new(),
        actor: None,
    };
    let mut interface = export(
        3,
        "Reader",
        TypeDescriptorIr::Interface,
        vec!["Item".to_string()],
    );
    interface.is_interface = true;
    interface.interface_methods = vec![InterfaceMethodSignature {
        name: "read".to_string(),
        type_params: Vec::new(),
        params: vec![skiff_artifact_model::FunctionTypeParamIr {
            name: "fallback".to_string(),
            ty: TypeRefIr::TypeParam {
                name: "Item".to_string(),
            },
        }],
        return_type: TypeRefIr::TypeParam {
            name: "Item".to_string(),
        },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }];
    let exports = ProjectedPackageExportLinks {
        exports: PackageExportIndex {
            types: BTreeMap::from([
                (
                    "example.pkg/Box".to_string(),
                    export(
                        0,
                        "Box",
                        TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "value".to_string(),
                                TypeRefIr::TypeParam {
                                    name: "Item".to_string(),
                                },
                            )]),
                        },
                        vec!["Item".to_string()],
                    ),
                ),
                (
                    "example.pkg/Token".to_string(),
                    export(
                        1,
                        "Token",
                        TypeDescriptorIr::Representation {
                            representation: TypeRefIr::TypeParam {
                                name: "Item".to_string(),
                            },
                        },
                        vec!["Item".to_string()],
                    ),
                ),
                (
                    "example.pkg/Choice".to_string(),
                    export(
                        2,
                        "Choice",
                        TypeDescriptorIr::Union {
                            branches: vec![NamedUnionBranchIr::SyntheticDiscriminator {
                                payload_type: TypeRefIr::TypeParam {
                                    name: "Item".to_string(),
                                },
                                discriminator_field: "tag".to_string(),
                                discriminator_value: "value".to_string(),
                            }],
                        },
                        vec!["Item".to_string()],
                    ),
                ),
                ("example.pkg/Reader".to_string(), interface),
                (
                    "example.pkg/Closed".to_string(),
                    export(
                        4,
                        "Closed",
                        TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "value".to_string(),
                                TypeRefIr::builtin("string"),
                            )]),
                        },
                        Vec::new(),
                    ),
                ),
            ]),
            ..PackageExportIndex::default()
        },
        public_instances: Vec::new(),
        alias_types: BTreeSet::new(),
    };

    let projected = project_package_schema("example.pkg", &exports, &[]).unwrap();

    assert_eq!(
        projected
            .index
            .types
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["Closed"]
    );
    assert_eq!(projected.records.len(), 1);
    assert_eq!(projected.refs_by_source.len(), 1);
}

#[test]
fn package_schema_applied_nominal_owner_is_omitted_without_partial_index() {
    let projected = project_package_schema(
        "example.pkg",
        &exports(TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "boxed".to_string(),
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "models".to_string(),
                            symbol: "Box".to_string(),
                        },
                    },
                    arguments: vec![TypeRefIr::builtin("string")],
                },
            )]),
        }),
        &[],
    )
    .unwrap();

    assert!(projected.index.types.is_empty());
    assert!(projected.records.is_empty());
    assert!(projected.refs_by_source.is_empty());
}

#[test]
fn package_schema_transitive_generic_owners_are_omitted_as_a_whole() {
    let file = FileIrRef::new("file", "models");
    let generic_ref = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "models".to_string(),
            symbol: "Cell".to_string(),
        },
    };
    let exports = ProjectedPackageExportLinks {
        exports: PackageExportIndex {
            types: BTreeMap::from([
                (
                    "example.pkg/Cell".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 0,
                        symbol: "Cell".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "value".to_string(),
                                TypeRefIr::TypeParam {
                                    name: "T".to_string(),
                                },
                            )]),
                        }),
                        type_params: vec!["T".to_string()],
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
                (
                    "example.pkg/DirectEnvelope".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 1,
                        symbol: "DirectEnvelope".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([("value".to_string(), generic_ref.clone())]),
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
                (
                    "example.pkg/AppliedEnvelope".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 2,
                        symbol: "AppliedEnvelope".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "value".to_string(),
                                TypeRefIr::AppliedNominal {
                                    base: NominalTypeRefBaseIr::ServiceSymbol {
                                        symbol: ServiceSymbolRef {
                                            module_path: "models".to_string(),
                                            symbol: "Cell".to_string(),
                                        },
                                    },
                                    arguments: vec![TypeRefIr::builtin("string")],
                                },
                            )]),
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
                (
                    "example.pkg/FreeParamEnvelope".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 3,
                        symbol: "FreeParamEnvelope".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Representation {
                            representation: TypeRefIr::TypeParam {
                                name: "Unbound".to_string(),
                            },
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
                (
                    "example.pkg/Closed".to_string(),
                    TypeExport {
                        file,
                        type_index: 4,
                        symbol: "Closed".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "value".to_string(),
                                TypeRefIr::builtin("string"),
                            )]),
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
            ]),
            ..PackageExportIndex::default()
        },
        public_instances: Vec::new(),
        alias_types: BTreeSet::new(),
    };

    let projected = project_package_schema("example.pkg", &exports, &[]).unwrap();

    assert_eq!(
        projected
            .index
            .types
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["Closed"]
    );
    assert_eq!(projected.records.len(), 1);
    assert_eq!(
        projected
            .refs_by_source
            .keys()
            .map(|(module, symbol)| (module.as_str(), symbol.as_str()))
            .collect::<Vec<_>>(),
        vec![("models", "Closed")]
    );
}

#[test]
fn package_schema_websocket_types_have_no_builtin_name_based_admission() {
    for name in [
        "std.websocket.WebSocketConnectRequest",
        "std.websocket.WebSocketConnectResult",
    ] {
        let projected = project_package_schema(
            "example.pkg",
            &exports(TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::Builtin {
                        name: name.to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    },
                )]),
            }),
            &[],
        )
        .unwrap();

        assert!(projected.index.types.is_empty(), "{name}");
        assert!(projected.records.is_empty(), "{name}");
        assert!(projected.refs_by_source.is_empty(), "{name}");
    }
}

#[test]
fn non_public_named_child_fails_closed() {
    let error = project_package_schema(
        "example.pkg",
        &exports(TypeDescriptorIr::Alias {
            target: TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "models".to_string(),
                    symbol: "Hidden".to_string(),
                },
            },
        }),
        &[],
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("not explicitly public in api.yml"));
}

fn external_schema(
    alias: &str,
    package_id: &str,
    stable_key: &str,
) -> (ResolvedPackageSchema, PackageSchemaTypeId) {
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("id".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let type_id = package_schema_type_id(package_id, stable_key, &descriptor).unwrap();
    let record = PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: descriptor,
    };
    let types = BTreeMap::from([(
        stable_key.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id.clone(),
            public_path: Some(stable_key.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    (
        ResolvedPackageSchema::new(
            alias.to_string(),
            package_id.to_string(),
            "1.2.3".to_string(),
            PackageBuildId::new(format!("build:{package_id}")),
            PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
            PackageSchemaIndex {
                package_id: package_id.to_string(),
                package_schema_index_identity: package_schema_index_identity(package_id, &types)
                    .unwrap(),
                types,
            },
            BTreeMap::from([(type_id.clone(), record)]),
        )
        .unwrap(),
        type_id,
    )
}

#[test]
fn external_alias_emits_no_local_schema_identity() {
    let (dependency, _external_id) = external_schema("types", "example.types", "User");
    let projected = project_package_schema(
        "example.pkg",
        &alias_exports(TypeDescriptorIr::Alias {
            target: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: "types".to_string(),
                    },
                    symbol_path: "User".to_string(),
                    abi_expectation: None,
                },
            },
        }),
        &[dependency],
    )
    .unwrap();

    assert!(projected.records.is_empty());
    assert!(projected.index.types.is_empty());
    assert!(projected.refs_by_source.is_empty());
}

#[test]
fn record_field_contains_expanded_literal_alias_without_alias_schema() {
    let file = FileIrRef::new("file", "types");
    let role_variants = vec![
        TypeRefIr::Literal {
            value: LiteralIr::String {
                value: "user".to_string(),
            },
        },
        TypeRefIr::Literal {
            value: LiteralIr::String {
                value: "assistant".to_string(),
            },
        },
    ];
    let exports = ProjectedPackageExportLinks {
        exports: PackageExportIndex {
            types: BTreeMap::from([
                (
                    "example.pkg/Role".to_string(),
                    TypeExport {
                        file: file.clone(),
                        type_index: 0,
                        symbol: "Role".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Alias {
                            target: TypeRefIr::Union {
                                items: role_variants.clone(),
                            },
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
                (
                    "example.pkg/Message".to_string(),
                    TypeExport {
                        file,
                        type_index: 1,
                        symbol: "Message".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "role".to_string(),
                                TypeRefIr::Union {
                                    items: role_variants,
                                },
                            )]),
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
            ]),
            ..PackageExportIndex::default()
        },
        public_instances: Vec::new(),
        alias_types: BTreeSet::from(["example.pkg/Role".to_string()]),
    };
    let projected = project_package_schema("example.pkg", &exports, &[]).unwrap();
    assert!(!projected.index.types.contains_key("Role"));
    let message = projected
        .records
        .get(&projected.index.types["Message"].package_schema_type_id)
        .unwrap();
    let ContractTypeDescriptor::Record { fields } = &message.canonical_descriptor.descriptor else {
        panic!("Message must remain a record");
    };
    assert_eq!(
        fields["role"],
        ContractTypeRef::structural_union(vec![
            ContractTypeRef::string_literal("assistant"),
            ContractTypeRef::string_literal("user"),
        ])
    );
}

#[test]
fn package_id_ref_with_multiple_exact_bindings_fails_closed() {
    let (first, _) = external_schema("types1", "example.types", "User");
    let (second, _) = external_schema("types2", "example.types", "User");
    let error = project_package_schema(
        "example.pkg",
        &exports(TypeDescriptorIr::Alias {
            target: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "example.types".to_string(),
                    },
                    symbol_path: "User".to_string(),
                    abi_expectation: None,
                },
            },
        }),
        &[first, second],
    )
    .unwrap_err();
    assert!(error.to_string().contains("matches multiple exact"));
}

#[test]
fn stream_and_actor_handle_exports_do_not_create_schema_records() {
    let stream = project_package_schema(
        "example.pkg",
        &exports(TypeDescriptorIr::Alias {
            target: TypeRefIr::Builtin {
                name: "Stream".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            },
        }),
        &[],
    )
    .unwrap();
    assert!(stream.records.is_empty());
    assert!(stream.index.types.is_empty());

    let actor = project_package_schema("example.pkg", &exports_with_descriptor(None), &[]).unwrap();
    assert!(actor.records.is_empty());
    assert!(actor.index.types.is_empty());

    let record_with_actor_field = ProjectedPackageExportLinks {
        exports: PackageExportIndex {
            types: BTreeMap::from([
                (
                    "example.pkg/UserActor".to_string(),
                    TypeExport {
                        file: FileIrRef::new("file", "models"),
                        type_index: 0,
                        symbol: "UserActor".to_string(),
                        is_interface: false,
                        descriptor: None,
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
                (
                    "example.pkg/Envelope".to_string(),
                    TypeExport {
                        file: FileIrRef::new("file", "models"),
                        type_index: 1,
                        symbol: "Envelope".to_string(),
                        is_interface: false,
                        descriptor: Some(TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "actor".to_string(),
                                TypeRefIr::ServiceSymbol {
                                    symbol: ServiceSymbolRef {
                                        module_path: "models".to_string(),
                                        symbol: "UserActor".to_string(),
                                    },
                                },
                            )]),
                        }),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: None,
                    },
                ),
            ]),
            ..PackageExportIndex::default()
        },
        public_instances: Vec::new(),
        alias_types: BTreeSet::new(),
    };
    let containing_actor =
        project_package_schema("example.pkg", &record_with_actor_field, &[]).unwrap();
    assert!(containing_actor.records.is_empty());
    assert!(containing_actor.index.types.is_empty());
}
