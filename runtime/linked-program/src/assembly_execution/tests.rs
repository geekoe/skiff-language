use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model as artifact;

use super::*;
use crate::{
    linked::{
        DbDeclarationIr, DbLeaseClaimIr, DbLeaseReadIr, DbObjectKeyIr, DbObjectKindIr,
        DbObjectTargetId, DbOpKindIr, DbOperationIr, DbQueryIr, FileDeclarations, FileLinkTargets,
        LinkedTypeDescriptor, SourceMapDto, TypeDeclIr, TypeDeclarationIr,
    },
    ExecutableKind, ExternalRefTable, HydratedPackageCode, LinkedExecutable,
    PublicationResourceTable, ServiceErrorTypeIndex,
};

const PACKAGE_ID: &str = "example.db-model";
const PACKAGE_BUILD: &str =
    "skiff-package-build-v10:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FORGED_PACKAGE_BUILD: &str =
    "skiff-package-build-v10:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PACKAGE_LOCAL_ABI: &str =
    "skiff-package-local-abi-v7:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FILE_ID: &str =
    "skiff-file-ir-v11:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const SOURCE_HASH: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

#[test]
fn assembly_task_routes_are_exact_and_unknown_targets_do_not_fallback() {
    let addr = ExecutableAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        executable: 0,
    };
    let image = admit(Vec::new(), Vec::new())
        .unwrap()
        .with_task_routes(BTreeMap::from([(
            "function:model.entry".to_string(),
            addr.clone(),
        )]))
        .unwrap();

    assert_eq!(image.task_route("function:model.entry"), Some(&addr));
    assert_eq!(image.task_route("model.entry"), None);
    assert_eq!(image.task_route("function:model.missing"), None);
}

#[test]
fn runtime_execution_package_binds_exact_artifact_files_and_resources() {
    let source = artifact_file();
    let artifact = Arc::new(package_artifact(&source));
    let linked = Arc::new(linked_file());

    let package = RuntimeExecutionPackage::try_new(
        PackageCodeSlotIndex::new(0),
        Arc::clone(&artifact),
        vec![Arc::clone(&linked)],
        PublicationResourceTable::default(),
    )
    .expect("exact package context should bind");

    assert_eq!(package.artifact(), artifact.as_ref());
    assert_eq!(package.package_id(), PACKAGE_ID);
    assert!(Arc::ptr_eq(package.file(FILE_ID).unwrap(), &linked));
    assert!(package.static_resources().is_empty());
}

#[test]
fn runtime_execution_package_rejects_file_fact_mismatch() {
    let source = artifact_file();
    let artifact = Arc::new(package_artifact(&source));
    let mut linked = linked_file();
    linked.module_path = "substituted".to_string();

    let error = RuntimeExecutionPackage::try_new(
        PackageCodeSlotIndex::new(0),
        artifact,
        vec![Arc::new(linked)],
        PublicationResourceTable::default(),
    )
    .expect_err("module mismatch must fail closed");

    assert!(matches!(
        error,
        AssemblyExecutionImageError::ExecutionFileMismatch { file_index: 0, .. }
    ));
}

#[test]
fn runtime_execution_package_rejects_ambiguous_artifact_file_identity() {
    let source = artifact_file();
    let mut artifact = package_artifact(&source);
    artifact.files.push(artifact.files[0].clone());
    let linked = Arc::new(linked_file());

    let error = RuntimeExecutionPackage::try_new(
        PackageCodeSlotIndex::new(0),
        Arc::new(artifact),
        vec![Arc::clone(&linked), linked],
        PublicationResourceTable::default(),
    )
    .expect_err("duplicate artifact File IR identity must fail closed");

    assert!(matches!(
        error,
        AssemblyExecutionImageError::DuplicateArtifactFileRef {
            ref file_ir_identity,
            ..
        } if file_ir_identity == FILE_ID
    ));
}
const FORGED_SOURCE_HASH: &str =
    "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

#[derive(Clone, Copy)]
enum DbCarrier {
    Operation,
    Query,
    LeaseClaim,
    LeaseRead,
}

#[test]
fn execution_admission_rejects_valid_hash_package_mismatch() {
    let mut target = exact_target(0);
    target.target_id.package_artifact_ref.package_build_id =
        artifact::PackageBuildId::new(FORGED_PACKAGE_BUILD);

    let error = admit(vec![carrier(DbCarrier::Operation, target)], Vec::new()).unwrap_err();

    assert!(matches!(
        error,
        AssemblyExecutionImageError::SharedImage(
            SharedPackageImageError::PackageBuildNotLoaded { build_id }
        ) if build_id == artifact::PackageBuildId::new(FORGED_PACKAGE_BUILD)
    ));
}

#[test]
fn execution_admission_checks_every_db_expression_carrier() {
    for carrier_kind in [
        DbCarrier::Operation,
        DbCarrier::Query,
        DbCarrier::LeaseClaim,
        DbCarrier::LeaseRead,
    ] {
        let mut target = exact_target(0);
        target.target_id.package_artifact_ref.package_build_id =
            artifact::PackageBuildId::new(FORGED_PACKAGE_BUILD);

        assert!(matches!(
            admit(vec![carrier(carrier_kind, target)], Vec::new()),
            Err(AssemblyExecutionImageError::SharedImage(
                SharedPackageImageError::PackageBuildNotLoaded { .. }
            ))
        ));
    }
}

#[test]
fn execution_admission_checks_db_targets_in_constant_bodies() {
    let mut target = exact_target(0);
    target.target_id.package_artifact_ref.package_build_id =
        artifact::PackageBuildId::new(FORGED_PACKAGE_BUILD);

    assert!(matches!(
        admit(Vec::new(), vec![carrier(DbCarrier::Operation, target)]),
        Err(AssemblyExecutionImageError::SharedImage(
            SharedPackageImageError::PackageBuildNotLoaded { .. }
        ))
    ));
}

#[test]
fn execution_admission_rejects_valid_hash_file_mismatch() {
    let mut target = exact_target(0);
    target.target_id.file_ir_ref.source_ast_hash = Some(FORGED_SOURCE_HASH.to_string());

    assert!(matches!(
        admit(vec![carrier(DbCarrier::Query, target)], Vec::new()),
        Err(AssemblyExecutionImageError::SharedImage(
            SharedPackageImageError::DbTargetFileRefOutsideArtifact { .. }
        ))
    ));
}

#[test]
fn execution_admission_rejects_type_without_db_attachment() {
    let target = exact_target(1);

    assert!(matches!(
        admit(vec![carrier(DbCarrier::LeaseRead, target)], Vec::new()),
        Err(AssemblyExecutionImageError::SharedImage(
            SharedPackageImageError::MissingDbTargetAttachment { type_index: 1, .. }
        ))
    ));
}

#[test]
fn execution_admission_rejects_every_symbolic_db_target_type_ref() {
    let service_symbol = artifact::ServiceSymbolRef {
        module_path: "model".to_string(),
        symbol: "Record".to_string(),
    };
    for symbolic in [
        LinkedTypeRef::DbObjectSymbol {
            symbol: service_symbol.clone(),
        },
        LinkedTypeRef::ServiceSymbol {
            symbol: service_symbol,
        },
        LinkedTypeRef::PackageSymbol {
            symbol: artifact::PackageSymbolRef {
                package: artifact::PackageRefIr::Dependency {
                    dependency_ref: "models".to_string(),
                },
                symbol_path: "model.Record".to_string(),
                abi_expectation: None,
            },
        },
    ] {
        let mut target = exact_target(0);
        target.type_ref = symbolic;

        assert!(matches!(
            admit(vec![carrier(DbCarrier::LeaseClaim, target)], Vec::new()),
            Err(AssemblyExecutionImageError::DbTargetTypeRefNotAddress {
                expression_index: 0,
                ..
            })
        ));
    }
}

#[test]
fn execution_admission_rejects_address_that_disagrees_with_target_id() {
    let mut target = exact_target(0);
    target.type_ref = LinkedTypeRef::Address {
        addr: TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: 1,
        },
    };

    assert!(matches!(
        admit(vec![carrier(DbCarrier::Operation, target)], Vec::new()),
        Err(AssemblyExecutionImageError::DbTargetAddressMismatch {
            expected: TypeAddr { type_index: 0, .. },
            actual: TypeAddr { type_index: 1, .. },
            ..
        })
    ));
}

#[test]
fn execution_admission_accepts_all_exact_db_target_carriers() {
    let expressions = [
        DbCarrier::Operation,
        DbCarrier::Query,
        DbCarrier::LeaseClaim,
        DbCarrier::LeaseRead,
    ]
    .into_iter()
    .map(|kind| carrier(kind, exact_target(0)))
    .collect();

    admit(expressions, Vec::new()).expect("exact DB targets should be admitted");
}

fn carrier(kind: DbCarrier, target: crate::DbTargetIr) -> LinkedExprIr {
    match kind {
        DbCarrier::Operation => LinkedExprIr::DbOperation {
            operation: DbOperationIr {
                op: DbOpKindIr::Count,
                many: false,
                target,
                selector: None,
                query: None,
                projection: None,
                body: None,
                insert_body: None,
                change: None,
                result_type: LinkedTypeRef::Native {
                    name: "number".to_string(),
                    args: Vec::new(),
                },
                source_span: None,
            },
        },
        DbCarrier::Query => LinkedExprIr::DbQuery {
            target,
            query: DbQueryIr::default(),
            projection: None,
            result_type: None,
        },
        DbCarrier::LeaseClaim => LinkedExprIr::DbLeaseClaim {
            claim: DbLeaseClaimIr {
                target,
                key: crate::ExprRefIr { expression: 0 },
                slot: "writer".to_string(),
                binding_slot: None,
                body: "claim".to_string(),
                result_type: LinkedTypeRef::Native {
                    name: "void".to_string(),
                    args: Vec::new(),
                },
                source_span: None,
            },
        },
        DbCarrier::LeaseRead => LinkedExprIr::DbLeaseRead {
            read: DbLeaseReadIr {
                target,
                key: crate::ExprRefIr { expression: 0 },
                slot: "writer".to_string(),
                result_type: LinkedTypeRef::Native {
                    name: "unknown".to_string(),
                    args: Vec::new(),
                },
                source_span: None,
            },
        },
    }
}

fn exact_target(type_index: usize) -> crate::DbTargetIr {
    crate::DbTargetIr {
        target_id: DbObjectTargetId {
            package_artifact_ref: package_ref(),
            file_ir_ref: artifact::FileIrRef {
                file_ir_identity: FILE_ID.to_string(),
                module_path: "model".to_string(),
                artifact_path: None,
                source_ast_hash: Some(SOURCE_HASH.to_string()),
            },
            type_index,
        },
        type_ref: LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::LoadedFileIndex(0),
                type_index,
            },
        },
        type_name: if type_index == 0 {
            "model.Record".to_string()
        } else {
            "model.Plain".to_string()
        },
    }
}

fn admit(
    executable_expressions: Vec<LinkedExprIr>,
    constant_expressions: Vec<LinkedExprIr>,
) -> AssemblyExecutionResult<AssemblyExecutionImage> {
    let artifact_file = artifact_file();
    let artifact = package_artifact(&artifact_file);
    let assembly = runtime_assembly();
    let schema_index = artifact::PackageSchemaIndex {
        package_id: PACKAGE_ID.to_string(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    };
    let hydrated = HydratedPackageCode::new(
        Arc::new(artifact),
        vec![Arc::new(artifact_file)],
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(schema_index));
    let shared = Arc::new(
        SharedPackageLinkedImage::from_runtime_assembly(&assembly, vec![hydrated])
            .expect("shared package fixture should hydrate"),
    );
    let mut linked_file = linked_file();
    linked_file.executables[0].body.expressions = executable_expressions;
    linked_file.constants[0].body.expressions = constant_expressions;
    let code = Arc::new(RuntimeExecutionPackage::try_from_shared(
        Arc::clone(&shared.code_slots()[0]),
        vec![Arc::new(linked_file)],
    )?);

    AssemblyExecutionImage::try_new(
        shared,
        vec![code],
        RuntimeTypeContext::default(),
        Arc::new(ServiceErrorTypeIndex::default()),
    )
}

fn artifact_file() -> artifact::FileIrUnit {
    let mut file = artifact::FileIrUnit::empty("model", SOURCE_HASH);
    file.file_ir_identity = FILE_ID.to_string();
    for name in ["Record", "Plain"] {
        file.type_table.push(artifact::TypeDeclIr {
            name: name.to_string(),
            descriptor: artifact::TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
    }
    file.declarations.types.insert(
        "Record".to_string(),
        artifact::TypeDeclarationIr {
            type_index: 0,
            symbol: "Record".to_string(),
            source_span: None,
        },
    );
    file.declarations.types.insert(
        "Plain".to_string(),
        artifact::TypeDeclarationIr {
            type_index: 1,
            symbol: "Plain".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "Record".to_string(),
        artifact::DbDeclarationIr {
            type_ref: artifact::TypeRefIr::LocalType { type_index: 0 },
            type_name: "model.Record".to_string(),
            collection_name: "record".to_string(),
            kind: artifact::DbObjectKindIr::Object,
            key: artifact::DbObjectKeyIr {
                name: "id".to_string(),
                ty: artifact::TypeRefIr::builtin("string"),
            },
            fields: Vec::new(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    file
}

fn linked_file() -> LinkedFileUnit {
    let mut declarations = FileDeclarations::default();
    declarations.types.insert(
        "Record".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Record".to_string(),
            source_span: None,
        },
    );
    declarations.types.insert(
        "Plain".to_string(),
        TypeDeclarationIr {
            type_index: 1,
            symbol: "Plain".to_string(),
            source_span: None,
        },
    );
    declarations.db.insert(
        "Record".to_string(),
        DbDeclarationIr {
            type_ref: LinkedTypeRef::Address {
                addr: TypeAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::LoadedFileIndex(0),
                    type_index: 0,
                },
            },
            type_name: "model.Record".to_string(),
            collection_name: "record".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            },
            fields: Vec::new(),
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    LinkedFileUnit {
        schema_version: artifact::FILE_IR_SCHEMA_VERSION.to_string(),
        file_ir_identity: FILE_ID.to_string(),
        source_ast_hash: SOURCE_HASH.to_string(),
        module_path: "model".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations,
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: ["Record", "Plain"]
            .into_iter()
            .map(|name| TypeDeclIr {
                name: name.to_string(),
                descriptor: LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            })
            .collect(),
        constants: vec![crate::ConstIr {
            name: "seed".to_string(),
            ty: LinkedTypeRef::Native {
                name: "unknown".to_string(),
                args: Vec::new(),
            },
            body: LinkedExecutableBody::default(),
            source_span: None,
        }],
        executables: vec![LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "model.entry".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: crate::SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }],
        external_refs: ExternalRefTable::default(),
    }
}

fn package_artifact(file: &artifact::FileIrUnit) -> artifact::PackageArtifact {
    artifact::PackageArtifact {
        schema_version: "skiff-package-artifact-v2".to_string(),
        package_id: PACKAGE_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: artifact::PackageBuildId::new(PACKAGE_BUILD),
        files: vec![artifact::FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }],
        static_resources: Vec::new(),
        package_local_abi: artifact::PackageLocalAbi {
            local_abi_identity: artifact::PackageLocalAbiIdentity::new(PACKAGE_LOCAL_ABI),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: artifact::PackageSchemaIndexRef {
            package_id: PACKAGE_ID.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                PACKAGE_ID,
                &BTreeMap::new(),
            )
            .expect("empty package schema index should be canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: artifact::PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: artifact::PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

fn package_ref() -> artifact::PackageArtifactRef {
    artifact::PackageArtifactRef {
        package_id: PACKAGE_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: artifact::PackageBuildId::new(PACKAGE_BUILD),
        package_local_abi_identity: artifact::PackageLocalAbiIdentity::new(PACKAGE_LOCAL_ABI),
    }
}

fn runtime_assembly() -> artifact::RuntimeAssembly {
    artifact::RuntimeAssembly {
        schema_version: artifact::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: artifact::AssemblyIdentity::new("assembly:db-admission-test"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        package_link_plan: artifact::CanonicalPackageLinkPlan {
            code_slots: vec![artifact::PackageCodeSlot {
                package: package_ref(),
            }],
            package_links: Vec::new(),
        },
        resolved_packages: vec![package_ref()],
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    }
}
