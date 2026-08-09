use skiff_artifact_identity::{
    assign_file_ir_identity, assign_package_artifact_identities, package_schema_index_identity,
};
use skiff_artifact_model::{
    DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, FileIrRef, PackageBuildId,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    ServiceSymbolRef, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeExport,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn same_source_db_path_in_two_dependencies_keeps_exact_primary_identities() {
    let (first_artifact, first_file) = provider("example.com/first");
    let (second_artifact, second_file) = provider("example.com/second");
    let index = foreign_package_db_metadata_index(&[
        ForeignPackageDbDependency {
            primary_alias: "first",
            top_level_alias: "firstImpl",
            contracts_only: false,
            artifact: &first_artifact,
            files: std::slice::from_ref(&first_file),
        },
        ForeignPackageDbDependency {
            primary_alias: "second",
            top_level_alias: "secondImpl",
            contracts_only: false,
            artifact: &second_artifact,
            files: std::slice::from_ref(&second_file),
        },
    ])
    .expect("both exact provider DB attachments should be selected");

    for (visible, primary, artifact) in [
        ("firstImpl/model.Session", "first", &first_artifact),
        ("secondImpl/model.Session", "second", &second_artifact),
    ] {
        let metadata = index
            .resolve_qualified(visible)
            .expect("visible source DB target");
        assert_eq!(
            metadata.canonical_type_ref,
            Some(TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: primary.to_string(),
                    },
                    symbol_path: "model.Session".to_string(),
                    abi_expectation: Some(
                        artifact
                            .package_local_abi
                            .local_abi_identity
                            .as_str()
                            .to_string()
                    ),
                },
            })
        );
    }
}

#[test]
fn foreign_db_selection_fails_closed_for_missing_file_link_or_db_attachment() {
    let (artifact, _file) = provider("example.com/provider");
    let error = foreign_package_db_metadata_index(&[ForeignPackageDbDependency {
        primary_alias: "provider",
        top_level_alias: "providerImpl",
        contracts_only: false,
        artifact: &artifact,
        files: &[],
    }])
    .unwrap_err();
    assert!(error.contains("exact loaded File IR"), "{error}");

    let (mut missing_link, _file) = provider("example.com/provider");
    missing_link
        .implementation_links
        .types
        .remove("model.Session");
    let error = implementation_type_symbols(&missing_link, &[]).unwrap_err();
    assert!(error.contains("no implementation link"), "{error}");

    let (mut non_db_artifact, mut non_db_file) = provider("example.com/provider");
    non_db_file.declarations.db.clear();
    assign_file_ir_identity(&mut non_db_file).unwrap();
    let file_ref = file_ref(&non_db_file);
    non_db_artifact.files = vec![file_ref.clone()];
    non_db_artifact
        .implementation_links
        .types
        .get_mut("model.Session")
        .unwrap()
        .file = file_ref;
    assign_package_artifact_identities(&mut non_db_artifact).unwrap();
    let index = foreign_package_db_metadata_index(&[ForeignPackageDbDependency {
        primary_alias: "provider",
        top_level_alias: "providerImpl",
        contracts_only: false,
        artifact: &non_db_artifact,
        files: std::slice::from_ref(&non_db_file),
    }])
    .expect("a non-DB type does not fabricate metadata");
    assert!(index
        .resolve_qualified("providerImpl/model.Session")
        .is_none());
}

#[test]
fn foreign_db_selection_rejects_stale_package_identity_and_same_named_file_substitution() {
    let (mut stale_artifact, file) = provider("example.com/provider");
    stale_artifact.package_build_id = PackageBuildId::new("stale-build");
    let error = foreign_package_db_metadata_index(&[ForeignPackageDbDependency {
        primary_alias: "provider",
        top_level_alias: "providerImpl",
        contracts_only: false,
        artifact: &stale_artifact,
        files: std::slice::from_ref(&file),
    }])
    .unwrap_err();
    assert!(error.contains("identity validation failed"), "{error}");

    let (artifact, _) = provider("example.com/provider");
    let (_, replacement) = provider("example.com/replacement");
    let error = foreign_package_db_metadata_index(&[ForeignPackageDbDependency {
        primary_alias: "provider",
        top_level_alias: "providerImpl",
        contracts_only: false,
        artifact: &artifact,
        files: std::slice::from_ref(&replacement),
    }])
    .unwrap_err();
    assert!(error.contains("exact loaded File IR"), "{error}");
}

fn provider(package_id: &str) -> (PackageArtifact, FileIrUnit) {
    let mut file = FileIrUnit::empty("model", format!("{package_id}:source"));
    file.type_table.push(TypeDeclIr {
        name: "Session".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("value".to_string(), TypeRefIr::builtin("string")),
            ]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "Session".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "model.Session".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "Session".to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::DbObjectSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "model".to_string(),
                    symbol: "Session".to_string(),
                },
            },
            type_name: "model.Session".to_string(),
            collection_name: Some("sessions".to_string()),
            implements: None,
            identity_fields: std::collections::BTreeMap::new(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![DbObjectFieldIr {
                name: "value".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    assign_file_ir_identity(&mut file).unwrap();
    let file_ref = file_ref(&file);
    let descriptor = file.type_table[0].descriptor.clone();
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::from([(
                "model.Session".to_string(),
                PackageLocalAbiSymbol::Type {
                    local_type_id: format!("type:{package_id}:top-level:model.Session"),
                    descriptor: descriptor.clone(),
                    is_alias: false,
                    is_interface: false,
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("unassigned"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::from([(
                "model.Session".to_string(),
                TypeExport {
                    file: file_ref,
                    type_index: 0,
                    symbol: "model.Session".to_string(),
                    is_interface: false,
                    descriptor: Some(descriptor),
                    type_params: Vec::new(),
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
        bytecode: None,
    };
    artifact.package_schema_index.package_schema_index_identity =
        package_schema_index_identity(package_id, &BTreeMap::new()).unwrap();
    assign_package_artifact_identities(&mut artifact).unwrap();
    (artifact, file)
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}
