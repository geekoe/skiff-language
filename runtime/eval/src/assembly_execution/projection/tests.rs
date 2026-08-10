use std::collections::BTreeMap;

use super::*;
use crate::{
    exceptions::catch_type_leaves, recoverable_behavior::EvalRecoverableBehaviorHooks,
    type_projection::EvalTypeProjection,
};
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, DbDeclarationIr as ArtifactDbDeclarationIr,
    DbObjectKeyIr as ArtifactDbObjectKeyIr, DbObjectKindIr as ArtifactDbObjectKindIr,
    ExecutableBody, ExecutableIr, ExecutableKind, FileIrRef, FileIrUnit, PackageArtifact,
    PackageArtifactRef, PackageBuildId, PackageCodeSlot, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexRef,
    RuntimeAssembly, SlotLayout, TypeDeclIr, TypeDeclarationIr as ArtifactTypeDeclarationIr,
    TypeDescriptorIr, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_model::service_error::{
    CatchIdentity, LocalExecutionTypeIdentity, NominalTypeIdentity, PlatformBuiltinErrorIdentity,
};

fn local_identity(addr: TypeAddr) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr,
            type_arguments: Vec::new(),
        },
    ))
}

fn platform_identity(symbol: &str) -> CatchIdentity {
    PlatformBuiltinErrorIdentity::from_symbol(symbol)
        .expect("fixture symbol must be in the finite platform-error registry")
        .catch_identity()
}

#[test]
fn assembly_execution_projection_resolves_image_owned_lookup_matrix() {
    let (image, file_identity) = projection_image();
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);
    let cloned_projection = projection.clone();
    assert!(Arc::ptr_eq(&projection.image, &cloned_projection.image));
    assert!(Arc::ptr_eq(&projection.storage, &cloned_projection.storage));
    let unit = UnitAddr::Package(0);
    let identity_file = FileAddr::FileIrIdentity(file_identity.clone());
    let indexed_file = FileAddr::LoadedFileIndex(0);

    assert_eq!(
        projection
            .resolve_file(&unit, &identity_file)
            .expect("identity file lookup")
            .file_ir_identity,
        file_identity
    );
    let entry = projection
        .resolve_executable(&ExecutableAddr {
            unit: unit.clone(),
            file: identity_file.clone(),
            executable: 0,
        })
        .expect("entry executable lookup");
    assert_eq!(entry.addr.file, indexed_file);
    assert_eq!(entry.executable.symbol, "projection.entry");

    let nested = projection
        .resolve_nested_executable(&ExecutableAddr {
            unit: unit.clone(),
            file: identity_file.clone(),
            executable: 1,
        })
        .expect("nested executable lookup");
    assert_eq!(nested.executable.symbol, "projection.nested");

    let constant = projection
        .resolve_const(&ConstAddr {
            unit: unit.clone(),
            file: identity_file.clone(),
            const_index: 0,
        })
        .expect("const lookup");
    assert_eq!(constant.constant.name, "projection.value");

    assert_eq!(
        projection
            .canonical_type_addr(&TypeAddr {
                unit,
                file: identity_file,
                type_index: 0,
            })
            .expect("type lookup"),
        TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        }
    );
    assert!(std::ptr::eq(projection.types(), projection.image().types()));
}

#[test]
fn assembly_execution_projection_never_falls_back_to_legacy_service_units() {
    let (image, _) = projection_image();
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);
    let error = projection
        .resolve_file(&UnitAddr::Service, &FileAddr::LoadedFileIndex(0))
        .expect_err("assembly service-unit lookup must fail closed");
    assert!(error.to_string().contains("legacy service unit"));

    let execution = RuntimeExecutionProjection::Assembly(projection);
    let error = match execution.legacy("service dispatch") {
        Ok(_) => panic!("assembly execution must not expose a legacy projection"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("legacy service dispatch"));
}

#[test]
fn assembly_database_type_and_recoverable_views_use_the_execution_image() {
    let (image, file_identity) = projection_image();
    let execution =
        RuntimeExecutionProjection::Assembly(RuntimeAssemblyExecutionProjection::from_image(image));
    let current_addr = ExecutableAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::FileIrIdentity(file_identity.clone()),
        executable: 0,
    };
    let plan = EvalTypeProjection::from_execution_projection(execution.clone())
        .plan_from_linked_nested_ref(
            &skiff_runtime_linked_program::LinkedTypeRef::Address {
                addr: TypeAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::FileIrIdentity(file_identity),
                    type_index: 0,
                },
            },
            &current_addr,
        )
        .expect("assembly database result type must resolve from the execution image");
    assert!(matches!(
        plan.node,
        skiff_runtime_model::type_plan::RuntimeTypeNode::Record { .. }
    ));

    EvalRecoverableBehaviorHooks::new_for_execution(&execution)
        .expect("assembly recoverable DB behavior must index the execution image");
}

#[test]
fn assembly_db_target_resolution_uses_the_full_exact_identity() {
    let (image, _) = projection_image();
    let execution =
        RuntimeExecutionProjection::Assembly(RuntimeAssemblyExecutionProjection::from_image(image));
    let exact = projection_db_target();
    let resolved = execution
        .resolve_db_target(&exact)
        .expect("exact DB target must resolve");
    assert_eq!(resolved.declaration.type_name, "ProjectionType");

    let mut substituted_package = exact.clone();
    substituted_package.package_artifact_ref.package_id =
        "projection.package.substituted".to_string();
    assert!(execution.resolve_db_target(&substituted_package).is_err());

    let mut substituted_file = exact.clone();
    substituted_file.file_ir_ref.artifact_path = Some("substituted/file.ir.json".to_string());
    assert!(execution.resolve_db_target(&substituted_file).is_err());

    let mut substituted_type = exact;
    substituted_type.type_index = 1;
    assert!(execution.resolve_db_target(&substituted_type).is_err());
}

#[test]
fn assembly_db_target_accepts_compiler_qualified_declaration_symbol() {
    let (image, _) = compiler_shaped_projection_image();
    let execution =
        RuntimeExecutionProjection::Assembly(RuntimeAssemblyExecutionProjection::from_image(image));

    let resolved = execution
        .resolve_db_target(&projection_db_target())
        .expect("qualified declaration symbol must resolve through its local map key");

    assert_eq!(resolved.declaration.type_name, "ProjectionType");
    assert_eq!(
        resolved.declaration.collection_name.as_deref(),
        Some("projection_type")
    );
}

#[test]
fn db_target_declaration_alias_tampering_stays_fail_closed() {
    let (image, _) = compiler_shaped_projection_image();
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);
    let file = projection.storage.packages[0].files()[0].as_ref();
    let addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };

    let mut wrong_declaration = file.clone();
    wrong_declaration
        .declarations
        .types
        .get_mut("ProjectionType")
        .expect("fixture declaration")
        .symbol = "wrong.ProjectionType".to_string();
    assert!(
        resolve_db_declaration(&wrong_declaration, addr.clone(), 0).is_err(),
        "non-canonical declaration symbol must be rejected"
    );

    let mut duplicate_slot = file.clone();
    duplicate_slot.declarations.types.insert(
        "ProjectionAlias".to_string(),
        skiff_runtime_linked_program::linked::TypeDeclarationIr {
            type_index: 0,
            symbol: "projection.ProjectionAlias".to_string(),
            source_span: None,
        },
    );
    assert!(
        resolve_db_declaration(&duplicate_slot, addr.clone(), 0).is_err(),
        "two declarations claiming one exact type slot must be rejected"
    );

    let mut wrong_attachment = file.clone();
    wrong_attachment
        .declarations
        .db
        .get_mut("ProjectionType")
        .expect("fixture DB attachment")
        .type_ref = skiff_runtime_linked_program::LinkedTypeRef::Address {
        addr: TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: 1,
        },
    };
    assert!(
        resolve_db_declaration(&wrong_attachment, addr, 0).is_err(),
        "DB attachment targeting another type slot must be rejected"
    );
}

#[test]
fn assembly_database_type_view_rejects_missing_type_information() {
    let (image, file_identity) = projection_image();
    let execution =
        RuntimeExecutionProjection::Assembly(RuntimeAssemblyExecutionProjection::from_image(image));
    let current_addr = ExecutableAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::FileIrIdentity(file_identity.clone()),
        executable: 0,
    };
    let error = EvalTypeProjection::from_execution_projection(execution)
        .plan_from_linked_nested_ref(
            &skiff_runtime_linked_program::LinkedTypeRef::Address {
                addr: TypeAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::FileIrIdentity(file_identity),
                    type_index: 99,
                },
            },
            &current_addr,
        )
        .expect_err("missing assembly database type information must fail closed");
    assert!(
        error.to_string().contains("TypeIndexOutOfBounds")
            && error.to_string().contains("type_index: 99"),
        "unexpected missing-type error: {error}"
    );
}

#[test]
fn canonical_assembly_resolves_every_std_package_error_address_to_its_builtin_identity() {
    let (image, errors) = std_error_projection_image("skiff.run/std");
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);

    for (symbol, addr) in errors {
        let catch_type =
            skiff_runtime_linked_program::LinkedTypeRef::Address { addr: addr.clone() };
        let leaves = catch_type_leaves(&catch_type, projection.type_view())
            .unwrap_or_else(|error| panic!("{symbol} catch leaves must resolve: {error}"));
        if symbol == "std.resource.ResourceError" {
            assert_eq!(
                leaves,
                vec![local_identity(addr)],
                "ResourceError is Package-owned, not a platform builtin"
            );
            continue;
        }
        assert!(
            leaves.contains(&platform_identity(&symbol)),
            "{symbol} catch must include its registered native payload identity; got {leaves:?}"
        );
        assert!(
            !leaves.contains(&local_identity(addr)),
            "{symbol} must use only the canonical platform identity"
        );
    }
}

#[test]
fn canonical_assembly_std_error_resolution_is_exact_and_nominal() {
    let (image, errors) = std_error_projection_image("skiff.run/std");
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);
    let (json_symbol, json_addr) = errors
        .iter()
        .find(|(symbol, _)| symbol == "std.json.DecodeError")
        .expect("json error fixture");
    let leaves = catch_type_leaves(
        &skiff_runtime_linked_program::LinkedTypeRef::Address {
            addr: json_addr.clone(),
        },
        projection.type_view(),
    )
    .expect("json catch leaves");
    assert_eq!(json_symbol, "std.json.DecodeError");
    assert!(!leaves.contains(&platform_identity("std.bytes.DecodeError")));

    let (image, errors) = std_error_projection_image("example.invalid/std-lookalike");
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);
    let (_, addr) = errors
        .into_iter()
        .find(|(symbol, _)| symbol == "std.json.DecodeError")
        .expect("nominal lookalike fixture");
    let leaves = catch_type_leaves(
        &skiff_runtime_linked_program::LinkedTypeRef::Address { addr: addr.clone() },
        projection.type_view(),
    )
    .expect("nominal package catch leaves");
    assert_eq!(leaves, vec![local_identity(addr)]);
}

#[test]
fn assembly_resource_projection_rejects_implementation_only_std_lookalike() {
    let (image, errors) = std_error_projection_image("skiff.run/std");
    let projection =
        RuntimeExecutionProjection::Assembly(RuntimeAssemblyExecutionProjection::from_image(image));
    let (_, addr) = errors
        .into_iter()
        .find(|(symbol, _)| symbol == "std.resource.ResourceError")
        .expect("ResourceError implementation fixture");
    let addr = projection
        .canonical_type_addr(&addr)
        .expect("implementation address canonicalizes");
    let error = projection
        .validate_public_package_type("skiff.run/std", "std.resource.ResourceError", &addr)
        .expect_err("implementation-only ResourceError must not count as public");
    assert!(matches!(error, RuntimeError::InvalidArtifact(message)
                if message.contains("not an exact public type symbol")));
}

#[test]
fn builtin_only_registered_errors_remain_native_without_package_guessing() {
    let (image, _) = projection_image();
    let projection = RuntimeAssemblyExecutionProjection::from_image(image);
    for symbol in ["TimeoutError", "config.DecodeError"] {
        let catch_type = skiff_runtime_linked_program::LinkedTypeRef::Native {
            name: symbol.to_string(),
            args: Vec::new(),
        };
        assert_eq!(
            catch_type_leaves(&catch_type, projection.type_view())
                .expect("registered builtin catch"),
            vec![platform_identity(symbol)]
        );
    }

    let legacy_cancel = skiff_runtime_linked_program::LinkedTypeRef::Native {
        name: "CancelError".to_string(),
        args: Vec::new(),
    };
    assert!(
        catch_type_leaves(&legacy_cancel, projection.type_view()).is_err(),
        "legacy CancelError linked spelling must fail closed"
    );
}

fn std_error_projection_image(
    package_id: &str,
) -> (Arc<AssemblyExecutionImage>, Vec<(String, TypeAddr)>) {
    const ERROR_TYPES: &[(&str, &[&str])] = &[
        ("std.bytes", &["DecodeError"]),
        ("std.number", &["DecodeError"]),
        ("std.json", &["DecodeError"]),
        (
            "std.db",
            &["ConflictError", "ConstraintError", "DecodeError"],
        ),
        ("std.file", &["FileError"]),
        ("std.resource", &["ResourceError"]),
        ("std.time", &["DecodeError"]),
        (
            "std.service",
            &["ProviderUnavailableError", "ProtocolError"],
        ),
        ("std.http", &["HttpError"]),
    ];
    let mut files = Vec::new();
    let mut file_refs = Vec::new();
    let mut errors = Vec::new();
    for (file_index, (module_path, names)) in ERROR_TYPES.iter().enumerate() {
        let mut file = FileIrUnit::empty(*module_path, format!("source:{module_path}"));
        file.file_ir_identity = format!("file:{module_path}");
        for (type_index, name) in names.iter().enumerate() {
            file.type_table.push(TypeDeclIr {
                name: (*name).to_string(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            });
            errors.push((
                format!("{module_path}.{name}"),
                TypeAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::LoadedFileIndex(file_index),
                    type_index,
                },
            ));
        }
        file_refs.push(FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        });
        files.push(file);
    }
    let package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!("{package_id}:build")),
        files: file_refs,
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(format!("{package_id}:abi")),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
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
        bytecode_statement_manifest_identity:
            skiff_artifact_model::derive_bytecode_statement_manifest_identity(package_id, &[])
                .expect("empty bytecode statement manifest is canonical"),
    };
    let package_ref = PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    };
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(format!("assembly:{package_id}")),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref,
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    (
        crate::test_support::link_package_fixture(assembly, vec![(package, files)]),
        errors,
    )
}

fn projection_image() -> (Arc<AssemblyExecutionImage>, String) {
    projection_image_with_db_shape("ProjectionType", TypeRefIr::LocalType { type_index: 0 })
}

fn compiler_shaped_projection_image() -> (Arc<AssemblyExecutionImage>, String) {
    projection_image_with_db_shape(
        "projection.ProjectionType",
        TypeRefIr::DbObjectSymbol {
            symbol: skiff_artifact_model::ServiceSymbolRef {
                module_path: "projection".to_string(),
                symbol: "ProjectionType".to_string(),
            },
        },
    )
}

fn projection_image_with_db_shape(
    declaration_symbol: &str,
    db_type_ref: TypeRefIr,
) -> (Arc<AssemblyExecutionImage>, String) {
    let mut file = FileIrUnit::empty("projection", "source:projection");
    file.file_ir_identity = "file:projection".to_string();
    file.type_table.push(TypeDeclIr {
        name: "ProjectionType".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "ProjectionType".to_string(),
        ArtifactTypeDeclarationIr {
            type_index: 0,
            symbol: declaration_symbol.to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "ProjectionType".to_string(),
        ArtifactDbDeclarationIr {
            type_ref: db_type_ref,
            type_name: "ProjectionType".to_string(),
            collection_name: Some("projection_type".to_string()),
            implements: None,
            identity_fields: std::collections::BTreeMap::new(),
            kind: ArtifactDbObjectKindIr::Object,
            key: ArtifactDbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("String"),
            },
            fields: Vec::new(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    file.constants.push(skiff_artifact_model::ConstIr {
        name: "projection.value".to_string(),
        ty: TypeRefIr::builtin("bool"),
        body: ExecutableBody::default(),
        source_span: None,
    });
    for symbol in ["projection.entry", "projection.nested"] {
        file.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("bool"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            expression_types: Vec::new(),
            statement_spans: Vec::new(),
            source_span: None,
        });
    }
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "projection.package".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("projection-build"),
        files: vec![file_ref],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("projection-abi"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "projection.package".to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                "projection.package",
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
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
        bytecode_statement_manifest_identity:
            skiff_artifact_model::derive_bytecode_statement_manifest_identity(
                "projection.package",
                &[],
            )
            .expect("empty bytecode statement manifest is canonical"),
    };
    let package_ref = PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    };
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:projection"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref,
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image =
        crate::test_support::link_package_fixture(assembly, vec![(package, vec![file.clone()])]);
    (image, file.file_ir_identity)
}

fn projection_db_target() -> DbObjectTargetId {
    DbObjectTargetId {
        package_artifact_ref: PackageArtifactRef {
            package_id: "projection.package".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("projection-build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("projection-abi"),
        },
        file_ir_ref: FileIrRef {
            file_ir_identity: "file:projection".to_string(),
            module_path: "projection".to_string(),
            artifact_path: None,
            source_ast_hash: Some("source:projection".to_string()),
        },
        type_index: 0,
    }
}
