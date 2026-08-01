use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, ContractTypeNameability, FileIrRef, FileIrUnit,
    NamedUnionBranchIr, PackageArtifact, PackageArtifactRef, PackageBuildId, PackageCodeSlot,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
    PackageSchemaIndexEntry, PackageSchemaIndexRef, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSchemaTypeRecordRef, RuntimeAssembly, TypeDeclIr as ArtifactTypeDecl, TypeDeclarationIr,
    TypeDescriptorIr, TypeExport, TypeLinkTargetIr, TypeRefIr, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_linked_program::{
    FileAddr, HydratedPackageCode, LinkedTypeDescriptor, PackageCodeSlotIndex, PackageSymbolKey,
    PublicationResourceTable, TypeDeclIr, UnitAddr,
};

use super::*;

#[test]
fn exact_package_schema_reference_matches_linked_address_only_by_full_owner_identity() {
    let addr = TypeAddr {
        unit: skiff_runtime_linked_program::UnitAddr::Package(0),
        file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let type_id = PackageSchemaTypeId::new("type:fault");
    let identities = HashMap::from([(
        addr.clone(),
        ServiceErrorPublicIdentity::new("example/a", "Fault", type_id.clone()),
    )]);
    validate_type_matches_schema(
        &LinkedTypeRef::Address { addr: addr.clone() },
        &ContractTypeRef::package_schema("example/a", "Fault", type_id.clone()),
        &identities,
    )
    .unwrap();
    assert!(validate_type_matches_schema(
        &LinkedTypeRef::Address { addr },
        &ContractTypeRef::package_schema("example/b", "Fault", type_id),
        &identities,
    )
    .is_err());
}

#[test]
fn builtin_schema_matching_remains_exact_without_alias_tolerance() {
    let public_identities = HashMap::new();
    let schema = ContractTypeRef::builtin("bool");
    validate_type_matches_schema(
        &LinkedTypeRef::Native {
            name: "bool".to_string(),
            args: Vec::new(),
        },
        &schema,
        &public_identities,
    )
    .expect("canonical builtin names should match exactly");

    let error = validate_type_matches_schema(
        &LinkedTypeRef::Native {
            name: "boolean".to_string(),
            args: Vec::new(),
        },
        &schema,
        &public_identities,
    )
    .expect_err("linker must reject an artificially noncanonical builtin pair");
    assert!(error.to_string().contains("differ"));

    let (shared, types) = image([package(
        "example/builtin",
        "api.BoolFault",
        TypeDescriptorIr::Representation {
            representation: TypeRefIr::builtin("bool"),
        },
        LinkedTypeDescriptor::Representation {
            representation: LinkedTypeRef::Native {
                name: "bool".to_string(),
                args: Vec::new(),
            },
        },
        ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin("bool"),
        },
        Vec::new(),
    )]);
    let root_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let error = validate_artifact_descriptor_matches_linked(
        &ExactTypeCoordinateResolver::new(&shared, &types),
        0,
        0,
        &TypeDescriptorIr::Representation {
            representation: TypeRefIr::builtin("boolean"),
        },
        &types.declaration(&root_addr).unwrap().descriptor,
    )
    .expect_err("export/execution comparison must also retain exact builtin spelling");
    assert!(error.to_string().contains("differ"));
}

#[test]
fn public_service_symbol_and_execution_local_type_share_exact_coordinate() {
    let (shared, types) = image([http_client_request_package()]);

    let index = build_service_error_type_index(&shared, &types)
        .expect("public ServiceSymbol and execution LocalType target the same declaration");

    assert_eq!(index.public_identity_len(), 2);
    assert!(index
        .public_identities()
        .any(|identity| identity.stable_schema_key() == HTTP_REQUEST_KEY));
    assert!(index
        .public_identities()
        .any(|identity| identity.stable_schema_key() == HTTP_HEADER_KEY));
}

#[test]
fn wrong_package_file_and_type_coordinates_fail_closed() {
    let (shared, mut types) = image([
        http_client_request_package(),
        record_package("example/other", "api.Other"),
    ]);
    let root_key = PackageSymbolKey::new(0, HTTP_REQUEST_KEY);

    for (wrong, expected) in [
        (
            TypeAddr {
                unit: UnitAddr::Package(1),
                file: FileAddr::LoadedFileIndex(0),
                type_index: 0,
            },
            "multiple public Package schema identities",
        ),
        (
            TypeAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::LoadedFileIndex(1),
                type_index: 0,
            },
            "exact export coordinate",
        ),
        (
            TypeAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::LoadedFileIndex(0),
                type_index: 1,
            },
            "multiple public Package schema identities",
        ),
    ] {
        types.exported_types.insert_package(root_key.clone(), wrong);
        let error = build_service_error_type_index(&shared, &types)
            .expect_err("non-exact root coordinate must be rejected");
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

#[test]
fn wrong_local_index_and_missing_coordinate_fail_closed() {
    let (shared, types) = image([http_client_request_package()]);
    let coordinates = ExactTypeCoordinateResolver::new(&shared, &types);
    let root_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let error = validate_artifact_descriptor_matches_linked(
        &coordinates,
        0,
        0,
        &TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "headers".to_string(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::LocalType { type_index: 9 }],
                },
            )]),
        },
        &types.declaration(&root_addr).unwrap().descriptor,
    )
    .expect_err("wrong execution-local index must be rejected");
    let evidence = format!("{error:#}");
    assert!(evidence.contains("type index 9"));

    let mut missing = http_client_request_package();
    service_symbol_in_root_export(&mut missing).symbol = "MissingHeader".to_string();
    let (shared, types) = image([missing]);
    let error = build_service_error_type_index(&shared, &types)
        .expect_err("missing public symbol coordinate must be rejected");
    assert!(format!("{error:#}").contains("MissingHeader is unresolved"));
}

#[test]
fn ambiguous_and_resolved_wrong_target_descriptors_fail_closed() {
    let mut ambiguous = http_client_request_package();
    Arc::make_mut(&mut ambiguous.files[0])
        .link_targets
        .types
        .insert("HttpHeader".to_string(), TypeLinkTargetIr { type_index: 0 });
    refresh_fixture_file_identity(&mut ambiguous, 0);
    let (shared, types) = image([ambiguous]);
    let error = build_service_error_type_index(&shared, &types)
        .expect_err("ambiguous symbol coordinate must be rejected");
    assert!(format!("{error:#}").contains("HttpHeader is ambiguous"));

    let mut wrong_target = http_client_request_package();
    Arc::make_mut(&mut wrong_target.files[0])
        .declarations
        .types
        .get_mut("HttpHeader")
        .unwrap()
        .type_index = 0;
    refresh_fixture_file_identity(&mut wrong_target, 0);
    let (shared, types) = image([wrong_target]);
    let error = build_service_error_type_index(&shared, &types)
        .expect_err("resolved but different target coordinate must be rejected");
    assert!(format!("{error:#}").contains("exact type coordinates differ"));
}

#[test]
fn full_owner_indexes_build_without_operation_error_roots() {
    let own = record_package("example/service", "api.ServiceFault");
    let dependency = record_package("example/dependency", "api.DependencyFault");
    let (shared, types) = image([own, dependency]);

    let index = build_service_error_type_index(&shared, &types).unwrap();
    assert_eq!(index.public_identity_len(), 2);
    assert!(index.public_identities().any(|identity| {
        identity.package_id() == "example/service"
            && identity.stable_schema_key() == "api.ServiceFault"
    }));
    assert!(index.public_identities().any(|identity| {
        identity.package_id() == "example/dependency"
            && identity.stable_schema_key() == "api.DependencyFault"
    }));
}

#[test]
fn representation_and_named_union_branch_context_are_retained() {
    let representation = package(
        "example/representation",
        "api.CodeFault",
        TypeDescriptorIr::Representation {
            representation: skiff_artifact_model::TypeRefIr::builtin("string"),
        },
        LinkedTypeDescriptor::Representation {
            representation: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        },
        ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin("string"),
        },
        Vec::new(),
    );
    let union = package(
        "example/union",
        "api.UnionFault",
        TypeDescriptorIr::Union {
            branches: vec![
                NamedUnionBranchIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "left".to_string(),
                    },
                },
                NamedUnionBranchIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "right".to_string(),
                    },
                },
            ],
        },
        LinkedTypeDescriptor::Union {
            branches: vec![
                LinkedNamedUnionBranch::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "left".to_string(),
                    },
                },
                LinkedNamedUnionBranch::Literal {
                    value: skiff_artifact_model::LiteralIr::String {
                        value: "right".to_string(),
                    },
                },
            ],
        },
        ContractTypeDescriptor::Enumeration {
            variants: vec!["left".to_string(), "right".to_string()],
        },
        Vec::new(),
    );
    let (shared, types) = image([representation, union]);

    let index = build_service_error_type_index(&shared, &types).unwrap();
    let representation_identity = index
        .public_identities()
        .find(|identity| identity.package_id() == "example/representation")
        .unwrap()
        .clone();
    assert!(matches!(
        index.by_public_identity(&representation_identity).unwrap()[0].context(),
        ServiceErrorExecutionContext::Declaration {
            kind: ServiceErrorDeclarationKind::Representation,
            ..
        }
    ));
    let union_identity = index
        .public_identities()
        .find(|identity| identity.package_id() == "example/union")
        .unwrap()
        .clone();
    let union_links = index.by_public_identity(&union_identity).unwrap();
    assert_eq!(union_links.len(), 2);
    assert!(matches!(
        union_links[1].context(),
        ServiceErrorExecutionContext::NamedUnionBranch {
            branch_index: 1,
            branch: LinkedNamedUnionBranch::Literal { .. },
            ..
        }
    ));

    let representation_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 7,
    };
    let mut branch_types = RuntimeTypeContext::default();
    branch_types.descriptors.insert(
        representation_addr.clone(),
        TypeDeclIr {
            name: "BranchRepresentation".to_string(),
            descriptor: LinkedTypeDescriptor::Representation {
                representation: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    );
    assert_eq!(
        representation_owner(
            &branch_types,
            &LinkedNamedUnionBranch::ConcreteNominal {
                nominal_type: LinkedTypeRef::Address {
                    addr: representation_addr.clone(),
                },
            },
        ),
        Some(representation_addr)
    );
}

#[test]
fn missing_public_link_descriptor_mismatch_and_generic_public_fail_closed() {
    let mut missing_link = record_package("example/missing", "api.Missing");
    Arc::make_mut(&mut missing_link.artifact)
        .implementation_links
        .types
        .clear();
    let (shared, types) = image([missing_link]);
    assert!(build_service_error_type_index(&shared, &types)
        .unwrap_err()
        .to_string()
        .contains("implementation type link"));

    let mut descriptor_mismatch = record_package("example/mismatch", "api.Mismatch");
    let record_id = descriptor_mismatch.index.types["api.Mismatch"]
        .package_schema_type_id
        .clone();
    Arc::make_mut(
        descriptor_mismatch
            .records
            .get_mut(&record_id)
            .expect("fixture root record"),
    )
    .canonical_descriptor
    .descriptor = ContractTypeDescriptor::Enumeration {
        variants: vec!["wrong".to_string()],
    };
    let (shared, types) = image([descriptor_mismatch]);
    assert!(build_service_error_type_index(&shared, &types).is_err());

    let generic = package(
        "example/generic",
        "api.GenericFault",
        TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
        vec!["T".to_string()],
    );
    let (shared, types) = image([generic]);
    assert!(build_service_error_type_index(&shared, &types)
        .unwrap_err()
        .to_string()
        .contains("generic public"));

    let dependency_type_id = PackageSchemaTypeId::new("schema:dependency");
    assert!(
        reject_applied_or_unresolved_type(&LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::PackageSchema {
                package_id: "example/dependency".to_string(),
                stable_schema_key: "api.Box".to_string(),
                package_schema_type_id: dependency_type_id,
            },
            arguments: vec![LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            }],
        })
        .unwrap_err()
        .to_string()
        .contains("applied PackageSchema")
    );
}

struct PackageFixture {
    artifact_ref: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    index: Arc<PackageSchemaIndex>,
    records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    files: Vec<Arc<FileIrUnit>>,
    linked_descriptors: Vec<Vec<LinkedTypeDescriptor>>,
}

const HTTP_PACKAGE_ID: &str = "skiff.run/std";
const HTTP_REQUEST_KEY: &str = "std.http.HttpClientRequest";
const HTTP_HEADER_KEY: &str = "std.http.HttpHeader";

fn http_client_request_package() -> PackageFixture {
    let header_canonical = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    };
    let header_type_id = skiff_artifact_identity::package_schema_type_id(
        HTTP_PACKAGE_ID,
        HTTP_HEADER_KEY,
        &header_canonical,
    )
    .unwrap();
    let header_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 1,
    };
    let source_descriptor = TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "headers".to_string(),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::LocalType { type_index: 1 }],
            },
        )]),
    };
    let linked_headers = LinkedTypeRef::Native {
        name: "Array".to_string(),
        args: vec![LinkedTypeRef::Address { addr: header_addr }],
    };
    let schema_headers = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::package_schema(
            HTTP_PACKAGE_ID,
            HTTP_HEADER_KEY,
            header_type_id.clone(),
        )],
    };
    let mut fixture = package(
        HTTP_PACKAGE_ID,
        HTTP_REQUEST_KEY,
        TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([("headers".to_string(), linked_headers)]),
        },
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("headers".to_string(), schema_headers)]),
        },
        Vec::new(),
    );

    {
        let file = Arc::make_mut(&mut fixture.files[0]);
        file.module_path = "std.http".to_string();
        file.type_table[0].descriptor = source_descriptor;
        file.type_table.push(ArtifactTypeDecl {
            name: "HttpHeader".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "HttpHeader".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "HttpHeader".to_string(),
                source_span: None,
            },
        );
    }
    refresh_fixture_file_identity(&mut fixture, 0);
    let file_ref = fixture.artifact.files[0].clone();
    let public_headers = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "std.http".to_string(),
                symbol: "HttpHeader".to_string(),
            },
        }],
    };
    let header_record = Arc::new(PackageSchemaTypeRecord {
        package_id: HTTP_PACKAGE_ID.to_string(),
        stable_schema_key: HTTP_HEADER_KEY.to_string(),
        package_schema_type_id: header_type_id.clone(),
        canonical_descriptor: header_canonical,
    });

    let index = Arc::make_mut(&mut fixture.index);
    index.types.insert(
        HTTP_HEADER_KEY.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: header_type_id.clone(),
            public_path: Some(HTTP_HEADER_KEY.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    );
    index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(HTTP_PACKAGE_ID, &index.types)
            .unwrap();
    fixture
        .records
        .insert(header_type_id.clone(), Arc::clone(&header_record));
    fixture.linked_descriptors[0].push(LinkedTypeDescriptor::Record {
        fields: BTreeMap::new(),
    });

    let artifact = Arc::make_mut(&mut fixture.artifact);
    artifact
        .implementation_links
        .types
        .get_mut(HTTP_REQUEST_KEY)
        .unwrap()
        .descriptor = Some(TypeDescriptorIr::Record {
        fields: BTreeMap::from([("headers".to_string(), public_headers)]),
    });
    artifact.implementation_links.types.insert(
        HTTP_HEADER_KEY.to_string(),
        TypeExport {
            file: file_ref,
            type_index: 1,
            symbol: "HttpHeader".to_string(),
            is_interface: false,
            descriptor: Some(TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            }),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    artifact.package_schema_index.package_schema_index_identity =
        fixture.index.package_schema_index_identity.clone();
    artifact.package_schema_type_records.insert(
        header_type_id.clone(),
        PackageSchemaTypeRecordRef {
            package_id: HTTP_PACKAGE_ID.to_string(),
            package_schema_type_id: header_type_id,
        },
    );
    fixture
}

fn refresh_fixture_file_identity(fixture: &mut PackageFixture, file_index: usize) {
    let old_identity = fixture.files[file_index].file_ir_identity.clone();
    skiff_artifact_identity::assign_file_ir_identity(Arc::make_mut(&mut fixture.files[file_index]))
        .unwrap();
    let file = &fixture.files[file_index];
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let artifact = Arc::make_mut(&mut fixture.artifact);
    let artifact_file = artifact
        .files
        .iter_mut()
        .find(|candidate| candidate.file_ir_identity == old_identity)
        .expect("fixture artifact must reference the refreshed file");
    *artifact_file = file_ref.clone();
    for export in artifact.implementation_links.types.values_mut() {
        if export.file.file_ir_identity == old_identity {
            export.file = file_ref.clone();
        }
    }
}

fn service_symbol_in_root_export(fixture: &mut PackageFixture) -> &mut ServiceSymbolRef {
    let descriptor = Arc::make_mut(&mut fixture.artifact)
        .implementation_links
        .types
        .get_mut(HTTP_REQUEST_KEY)
        .and_then(|export| export.descriptor.as_mut())
        .expect("fixture request export descriptor");
    let TypeDescriptorIr::Record { fields } = descriptor else {
        unreachable!()
    };
    let TypeRefIr::Builtin { args, .. } = fields.get_mut("headers").unwrap() else {
        unreachable!()
    };
    let TypeRefIr::ServiceSymbol { symbol } = &mut args[0] else {
        unreachable!()
    };
    symbol
}

fn record_package(package_id: &str, stable_key: &str) -> PackageFixture {
    package(
        package_id,
        stable_key,
        TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
        Vec::new(),
    )
}

fn package(
    package_id: &str,
    stable_key: &str,
    source_descriptor: TypeDescriptorIr,
    linked_descriptor: LinkedTypeDescriptor,
    schema_descriptor: ContractTypeDescriptor,
    type_params: Vec<String>,
) -> PackageFixture {
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: type_params.clone(),
        descriptor: schema_descriptor,
    };
    let type_id = skiff_artifact_identity::package_schema_type_id(
        package_id,
        stable_key,
        &canonical_descriptor,
    )
    .unwrap();
    let record = Arc::new(PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor,
    });
    let index_types = BTreeMap::from([(
        stable_key.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id.clone(),
            public_path: Some(stable_key.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    let index = Arc::new(PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            package_id,
            &index_types,
        )
        .unwrap(),
        types: index_types,
    });
    let mut file = FileIrUnit::empty("errors", "source-hash");
    file.type_table.push(ArtifactTypeDecl {
        name: stable_key.to_string(),
        descriptor: source_descriptor.clone(),
        type_params: type_params.clone(),
        implements: Vec::new(),
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let artifact = Arc::new(PackageArtifact {
        schema_version: skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!("build:{package_id}")),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: index.package_schema_index_identity.clone(),
        },
        package_schema_type_records: BTreeMap::from([(
            type_id.clone(),
            PackageSchemaTypeRecordRef {
                package_id: package_id.to_string(),
                package_schema_type_id: type_id,
            },
        )]),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::from([(
                stable_key.to_string(),
                TypeExport {
                    file: file_ref,
                    type_index: 0,
                    symbol: stable_key.to_string(),
                    is_interface: false,
                    descriptor: Some(source_descriptor),
                    type_params,
                    interface_methods: Vec::new(),
                    actor: None,
                },
            )]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    });
    let artifact_ref = PackageArtifactRef {
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    };
    PackageFixture {
        artifact_ref,
        artifact,
        index,
        records: BTreeMap::from([(record.package_schema_type_id.clone(), record)]),
        files: vec![Arc::new(file)],
        linked_descriptors: vec![vec![linked_descriptor]],
    }
}

fn image<const N: usize>(
    fixtures: [PackageFixture; N],
) -> (SharedPackageLinkedImage, RuntimeTypeContext) {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:error-index"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: fixtures
            .iter()
            .map(|fixture| fixture.artifact_ref.clone())
            .collect(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: fixtures
                .iter()
                .map(|fixture| PackageCodeSlot {
                    package: fixture.artifact_ref.clone(),
                })
                .collect(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let hydrated = fixtures
        .iter()
        .map(|fixture| {
            HydratedPackageCode::new(
                Arc::clone(&fixture.artifact),
                fixture.files.iter().map(Arc::clone).collect(),
                PublicationResourceTable::default(),
            )
            .with_schema_index(Arc::clone(&fixture.index))
            .with_schema_records(fixture.records.clone())
        })
        .collect::<Vec<_>>();
    let shared = SharedPackageLinkedImage::from_runtime_assembly(&assembly, hydrated).unwrap();
    let mut types = RuntimeTypeContext::default();
    for (code_slot, fixture) in fixtures.into_iter().enumerate() {
        assert_eq!(fixture.files.len(), fixture.linked_descriptors.len());
        for (file_index, (file, linked_descriptors)) in fixture
            .files
            .iter()
            .zip(fixture.linked_descriptors)
            .enumerate()
        {
            assert_eq!(file.type_table.len(), linked_descriptors.len());
            for (type_index, (source, descriptor)) in
                file.type_table.iter().zip(linked_descriptors).enumerate()
            {
                types.descriptors.insert(
                    TypeAddr {
                        unit: UnitAddr::Package(code_slot),
                        file: FileAddr::LoadedFileIndex(file_index),
                        type_index,
                    },
                    TypeDeclIr {
                        name: source.name.clone(),
                        descriptor,
                        type_params: source.type_params.clone(),
                        implements: Vec::new(),
                        source_span: None,
                    },
                );
            }
        }
        for (symbol, export) in &fixture.artifact.implementation_links.types {
            let file_index = fixture
                .files
                .iter()
                .position(|file| file.file_ir_identity == export.file.file_ir_identity)
                .expect("fixture export file must be loaded");
            types.exported_types.insert_package(
                PackageSymbolKey::new(code_slot, symbol.clone()),
                TypeAddr {
                    unit: UnitAddr::Package(code_slot),
                    file: FileAddr::LoadedFileIndex(file_index),
                    type_index: export.type_index as usize,
                },
            );
        }
        assert_eq!(
            shared
                .code_by_slot(PackageCodeSlotIndex::new(code_slot))
                .unwrap()
                .code_slot(),
            PackageCodeSlotIndex::new(code_slot)
        );
    }
    (shared, types)
}
