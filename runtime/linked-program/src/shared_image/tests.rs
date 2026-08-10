use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, AssemblyIdentity, CallIr, CallTargetIr,
    CanonicalPackageLinkPlan, ConstExport, ConstIr, ContractOperationId, ContractRequirement,
    DbDeclarationIr, DbObjectKeyIr, DbObjectKindIr, ExecutableBody, ExecutableIr, ExecutableKind,
    ExprIr, FileIrRef, FileIrUnit, InstructionSourceSite, OperationCallableKind, PackageArtifact,
    PackageArtifactRef, PackageBinding, PackageBuildId, PackageCallableId, PackageCallableLinkFact,
    PackageCallableRef, PackageCodeSlot, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRefIr, PackageRequirement,
    PackageRequirementKey, PackageRuntimeRequirements, PackageSchemaIndex, PackageSchemaIndexRef,
    PackageSymbolRef, PublicationResourceRef, RuntimeAssembly, ServiceCallRef,
    ServiceProtocolIdentity, ServiceRequirement, SlotLayout, SyntheticInstructionSiteReason,
    TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeExport, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_model::resource::LoadedPublicationResource;

use super::*;

#[test]
fn empty_link_plan_builds_empty_image_and_all_lookups_fail_closed() {
    let assembly = assembly(Vec::new(), Vec::new());
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        Vec::<HydratedPackageCode>::new(),
    )
    .expect("canonical empty image");

    assert!(image.is_empty());
    assert_eq!(image.assembly_identity(), &assembly.assembly_identity);
    assert!(image.package_link_plan().code_slots.is_empty());
    assert_eq!(
        image
            .resolve_package_direct_call_by_alias(
                &build_id("missing"),
                "tools",
                &callable_id("ping"),
            )
            .unwrap_err(),
        SharedPackageImageError::PackageBuildNotLoaded {
            build_id: build_id("missing"),
        }
    );
    assert_eq!(
        image
            .resolve_activation_relative_service_call(
                &build_id("missing"),
                "file:missing",
                ServiceCallRefIndex::new(0),
            )
            .unwrap_err(),
        SharedPackageImageError::PackageBuildNotLoaded {
            build_id: build_id("missing"),
        }
    );
}

#[test]
fn assembly_execution_direct_calls_are_scoped_by_caller_build_even_when_aliases_match() {
    let callable = callable_id("ping");
    let mut caller_a_file = file("file:caller-a", "caller.a");
    add_package_call(&mut caller_a_file, "tools", callable.clone());
    let mut caller_b_file = file("file:caller-b", "caller.b");
    add_package_call(&mut caller_b_file, "tools", callable.clone());
    let dependency_a_file = file("file:dependency-a", "dependency.a");
    let dependency_b_file = file("file:dependency-b", "dependency.b");

    let mut caller_a = artifact("caller.a", "caller-a", "caller-a-abi", &caller_a_file);
    caller_a.package_requirements.push(package_requirement(
        "tools",
        "dependency.a",
        "dependency-a-abi",
    ));
    let mut caller_b = artifact("caller.b", "caller-b", "caller-b-abi", &caller_b_file);
    caller_b.package_requirements.push(package_requirement(
        "tools",
        "dependency.b",
        "dependency-b-abi",
    ));
    let mut dependency_a = artifact(
        "dependency.a",
        "dependency-a",
        "dependency-a-abi",
        &dependency_a_file,
    );
    add_callable(&mut dependency_a, &dependency_a_file, callable.clone());
    let mut dependency_b = artifact(
        "dependency.b",
        "dependency-b",
        "dependency-b-abi",
        &dependency_b_file,
    );
    add_callable(&mut dependency_b, &dependency_b_file, callable.clone());
    add_static_resource(&mut dependency_a, "prompts/a.txt", b"a");

    let refs = vec![
        artifact_ref(&caller_a),
        artifact_ref(&caller_b),
        artifact_ref(&dependency_a),
        artifact_ref(&dependency_b),
    ];
    let links = vec![
        package_binding(&caller_a, "tools", &dependency_a),
        package_binding(&caller_b, "tools", &dependency_b),
    ];
    let assembly = assembly(refs, links);
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![
            hydration(caller_b, caller_b_file),
            hydration_with_resources(
                dependency_a,
                dependency_a_file,
                [("prompts/a.txt", b"a".as_slice())],
            ),
            hydration(caller_a, caller_a_file),
            hydration(dependency_b, dependency_b_file),
        ],
    )
    .expect("caller-qualified aliases link independently");

    let resolved_a = image
        .resolve_package_direct_call_by_alias(&build_id("caller-a"), "tools", &callable)
        .unwrap();
    let resolved_b = image
        .resolve_package_direct_call_by_alias(&build_id("caller-b"), "tools", &callable)
        .unwrap();

    assert_eq!(resolved_a.dependency_code_slot().index(), 2);
    assert_eq!(
        resolved_a.dependency_package_build_id(),
        &build_id("dependency-a")
    );
    assert_eq!(resolved_b.dependency_code_slot().index(), 3);
    assert_eq!(
        resolved_b.dependency_package_build_id(),
        &build_id("dependency-b")
    );
    assert_ne!(
        resolved_a.dependency_package_build_id(),
        resolved_b.dependency_package_build_id()
    );
    assert_eq!(resolved_a.target().callable_abi_id, callable.as_str());

    let dependency_a_code = image.code_by_build(&build_id("dependency-a")).unwrap();
    assert_eq!(
        dependency_a_code.local_abi_identity(),
        &local_abi("dependency-a-abi")
    );
    assert_eq!(
        dependency_a_code
            .static_resources()
            .get("prompts/a.txt")
            .unwrap()
            .bytes
            .as_ref(),
        b"a"
    );
}

#[test]
fn assembly_execution_package_diamond_has_one_dependency_code_owner() {
    let callable = callable_id("shared");
    let mut left_file = file("file:left", "left.main");
    add_package_call(&mut left_file, "shared", callable.clone());
    let mut right_file = file("file:right", "right.main");
    add_package_call(&mut right_file, "shared", callable.clone());
    let dependency_file = file("file:shared", "shared.main");

    let mut left = artifact("left", "left-build", "left-abi", &left_file);
    left.package_requirements
        .push(package_requirement("shared", "shared", "shared-abi"));
    let mut right = artifact("right", "right-build", "right-abi", &right_file);
    right
        .package_requirements
        .push(package_requirement("shared", "shared", "shared-abi"));
    let mut dependency = artifact("shared", "shared-build", "shared-abi", &dependency_file);
    add_callable(&mut dependency, &dependency_file, callable.clone());

    let assembly = assembly(
        vec![
            artifact_ref(&left),
            artifact_ref(&right),
            artifact_ref(&dependency),
        ],
        vec![
            package_binding(&left, "shared", &dependency),
            package_binding(&right, "shared", &dependency),
        ],
    );
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![
            hydration(right, right_file),
            hydration(dependency, dependency_file),
            hydration(left, left_file),
        ],
    )
    .unwrap();

    let from_left = image
        .resolve_package_direct_call_by_alias(&build_id("left-build"), "shared", &callable)
        .unwrap();
    let from_right = image
        .resolve_package_direct_call_by_alias(&build_id("right-build"), "shared", &callable)
        .unwrap();
    assert_eq!(
        from_left.dependency_code_slot(),
        from_right.dependency_code_slot()
    );
    assert_eq!(image.code_slots().len(), 3);
    let by_build = Arc::clone(image.code_by_build(&build_id("shared-build")).unwrap());
    let by_slot = Arc::clone(
        image
            .code_by_slot(from_left.dependency_code_slot())
            .unwrap(),
    );
    assert!(Arc::ptr_eq(&by_build, &by_slot));
}

#[test]
fn public_instance_method_links_exact_receiver_while_ordinary_callables_keep_none() {
    let mut package_file = file("file:receiver", "api");
    add_const(&mut package_file, "WORKER");
    let method = callable_id("worker.run");
    let ordinary = callable_id("ordinary");
    let top_level = callable_id("top-level:api.Worker.run");
    let mut package = artifact(
        "receiver.pkg",
        "receiver-build",
        "receiver-abi",
        &package_file,
    );
    add_callable_kind(
        &mut package,
        &package_file,
        method.clone(),
        OperationCallableKind::ImplMethod,
    );
    add_callable(&mut package, &package_file, ordinary.clone());
    add_callable_kind(
        &mut package,
        &package_file,
        top_level.clone(),
        OperationCallableKind::ImplMethod,
    );
    add_public_instance(
        &mut package,
        &package_file,
        "worker",
        0,
        [("run", method.clone())],
        TypeRefIr::builtin("Worker"),
        vec![TypeRefIr::builtin("Runnable")],
    );

    let image = one_package_image(package, package_file).expect("public instance should link");
    let code = &image.code_slots()[0];
    assert_eq!(
        code.linked_callable_target(&method)
            .unwrap()
            .receiver_const(),
        Some(&ConstAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            const_index: 0,
        })
    );
    assert_eq!(
        code.linked_callable_target(&ordinary)
            .unwrap()
            .receiver_const(),
        None
    );
    assert_eq!(
        code.linked_callable_target(&top_level)
            .unwrap()
            .receiver_const(),
        None,
        "an implementation/topLevelAlias callable is not a bound public instance method"
    );
}

#[test]
fn two_public_instances_sharing_one_executable_keep_distinct_receivers() {
    let mut package_file = file("file:two-receivers", "api");
    add_const(&mut package_file, "LEFT");
    add_const(&mut package_file, "RIGHT");
    let left = callable_id("left.run");
    let right = callable_id("right.run");
    let mut package = artifact("receiver.pkg", "two-build", "two-abi", &package_file);
    add_callable_kind(
        &mut package,
        &package_file,
        left.clone(),
        OperationCallableKind::ImplMethod,
    );
    add_callable_kind(
        &mut package,
        &package_file,
        right.clone(),
        OperationCallableKind::ImplMethod,
    );
    add_public_instance(
        &mut package,
        &package_file,
        "left",
        0,
        [("run", left.clone())],
        TypeRefIr::builtin("Worker"),
        vec![TypeRefIr::builtin("Runnable")],
    );
    add_public_instance(
        &mut package,
        &package_file,
        "right",
        1,
        [("run", right.clone())],
        TypeRefIr::builtin("Worker"),
        vec![TypeRefIr::builtin("Runnable")],
    );

    let image = one_package_image(package, package_file).expect("two receivers should link");
    let code = &image.code_slots()[0];
    let left_target = code.linked_callable_target(&left).unwrap();
    let right_target = code.linked_callable_target(&right).unwrap();
    assert_eq!(
        left_target.executable_addr(),
        right_target.executable_addr()
    );
    assert_eq!(left_target.receiver_const().unwrap().const_index, 0);
    assert_eq!(right_target.receiver_const().unwrap().const_index, 1);
}

#[test]
fn generic_multi_interface_instance_uses_declared_callable_ids_without_name_guessing() {
    let mut package_file = file("file:generic-receiver", "api");
    add_const(&mut package_file, "BOX");
    let read = callable_id("box.read");
    let write = callable_id("box.write");
    let mut package = artifact(
        "receiver.pkg",
        "generic-build",
        "generic-abi",
        &package_file,
    );
    for callable in [&read, &write] {
        add_callable_kind(
            &mut package,
            &package_file,
            callable.clone(),
            OperationCallableKind::ImplMethod,
        );
    }
    add_public_instance(
        &mut package,
        &package_file,
        "box",
        0,
        [("read", read.clone()), ("write", write.clone())],
        TypeRefIr::AppliedNominal {
            base: skiff_artifact_model::NominalTypeRefBaseIr::ServiceSymbol {
                symbol: skiff_artifact_model::ServiceSymbolRef {
                    module_path: "api".to_string(),
                    symbol: "Box".to_string(),
                },
            },
            arguments: vec![TypeRefIr::builtin("string")],
        },
        vec![TypeRefIr::builtin("Reader"), TypeRefIr::builtin("Writer")],
    );

    let image = one_package_image(package, package_file).expect("generic receiver should link");
    let code = &image.code_slots()[0];
    assert_eq!(
        code.linked_callable_target(&read)
            .unwrap()
            .receiver_const()
            .unwrap()
            .const_index,
        0
    );
    assert_eq!(
        code.linked_callable_target(&write)
            .unwrap()
            .receiver_const()
            .unwrap()
            .const_index,
        0
    );
}

#[test]
fn malformed_public_instance_receiver_facts_fail_closed() {
    let mut missing_receiver_file = file("file:missing-receiver", "api");
    add_const(&mut missing_receiver_file, "WORKER");
    let missing_receiver_callable = callable_id("missing.run");
    let mut missing_receiver = artifact(
        "receiver.pkg",
        "missing-receiver-build",
        "missing-receiver-abi",
        &missing_receiver_file,
    );
    add_callable_kind(
        &mut missing_receiver,
        &missing_receiver_file,
        missing_receiver_callable.clone(),
        OperationCallableKind::ImplMethod,
    );
    missing_receiver.package_local_abi.public_symbols.insert(
        "worker".to_string(),
        public_instance_symbol(
            "worker",
            [("run", missing_receiver_callable)],
            TypeRefIr::builtin("Worker"),
            vec![TypeRefIr::builtin("Runnable")],
        ),
    );
    assert!(matches!(
        one_package_image(missing_receiver, missing_receiver_file),
        Err(SharedPackageImageError::MissingPublicInstanceReceiverLink { .. })
    ));

    let mut missing_callable_file = file("file:missing-callable", "api");
    add_const(&mut missing_callable_file, "WORKER");
    let mut missing_callable = artifact(
        "receiver.pkg",
        "missing-callable-build",
        "missing-callable-abi",
        &missing_callable_file,
    );
    add_public_instance(
        &mut missing_callable,
        &missing_callable_file,
        "worker",
        0,
        [("run", callable_id("missing.run"))],
        TypeRefIr::builtin("Worker"),
        vec![TypeRefIr::builtin("Runnable")],
    );
    assert!(matches!(
        one_package_image(missing_callable, missing_callable_file),
        Err(SharedPackageImageError::MissingPublicInstanceCallableLink { .. })
    ));
}

#[test]
fn duplicate_and_conflicting_public_instance_callable_ownership_fail_closed() {
    let mut duplicate_file = file("file:duplicate-receiver", "api");
    add_const(&mut duplicate_file, "WORKER");
    let duplicate_callable = callable_id("worker.run");
    let mut duplicate = artifact(
        "receiver.pkg",
        "duplicate-build",
        "duplicate-abi",
        &duplicate_file,
    );
    add_callable_kind(
        &mut duplicate,
        &duplicate_file,
        duplicate_callable.clone(),
        OperationCallableKind::ImplMethod,
    );
    add_public_instance(
        &mut duplicate,
        &duplicate_file,
        "worker",
        0,
        [
            ("run", duplicate_callable.clone()),
            ("repeat", duplicate_callable),
        ],
        TypeRefIr::builtin("Worker"),
        vec![TypeRefIr::builtin("Runnable")],
    );
    assert!(matches!(
        one_package_image(duplicate, duplicate_file),
        Err(SharedPackageImageError::DuplicatePublicInstanceCallableReceiver { .. })
    ));

    let mut conflict_file = file("file:conflicting-receivers", "api");
    add_const(&mut conflict_file, "LEFT");
    add_const(&mut conflict_file, "RIGHT");
    let conflict_callable = callable_id("shared.run");
    let mut conflict = artifact(
        "receiver.pkg",
        "conflict-build",
        "conflict-abi",
        &conflict_file,
    );
    add_callable_kind(
        &mut conflict,
        &conflict_file,
        conflict_callable.clone(),
        OperationCallableKind::ImplMethod,
    );
    for (path, index) in [("left", 0), ("right", 1)] {
        add_public_instance(
            &mut conflict,
            &conflict_file,
            path,
            index,
            [("run", conflict_callable.clone())],
            TypeRefIr::builtin("Worker"),
            vec![TypeRefIr::builtin("Runnable")],
        );
    }
    assert!(matches!(
        one_package_image(conflict, conflict_file),
        Err(SharedPackageImageError::ConflictingPublicInstanceCallableReceiver { .. })
    ));
}

#[test]
fn foreign_db_targets_with_identical_names_keep_exact_dependency_identity() {
    let caller_a_file = file("file:caller-a-db", "caller.a");
    let caller_b_file = file("file:caller-b-db", "caller.b");
    let mut dependency_a_file = file("file:dependency-a-db", "models");
    let mut dependency_b_file = file("file:dependency-b-db", "models");
    add_db_declaration(&mut dependency_a_file, "User");
    add_db_declaration(&mut dependency_b_file, "User");

    let mut caller_a = artifact("caller.a", "caller-a-db", "caller-a-abi", &caller_a_file);
    let mut caller_b = artifact("caller.b", "caller-b-db", "caller-b-abi", &caller_b_file);
    let mut dependency_a = artifact(
        "dependency.a",
        "dependency-a-db",
        "dependency-a-abi",
        &dependency_a_file,
    );
    let mut dependency_b = artifact(
        "dependency.b",
        "dependency-b-db",
        "dependency-b-abi",
        &dependency_b_file,
    );
    add_db_export(&mut dependency_a, &dependency_a_file, "models.User", "User");
    add_db_export(&mut dependency_b, &dependency_b_file, "models.User", "User");
    let mut requirement_a = package_requirement("models", "dependency.a", "dependency-a-abi");
    requirement_a.expected_package_build = Some(dependency_a.package_build_id.clone());
    caller_a.package_requirements.push(requirement_a);
    let mut requirement_b = package_requirement("models", "dependency.b", "dependency-b-abi");
    requirement_b.expected_package_build = Some(dependency_b.package_build_id.clone());
    caller_b.package_requirements.push(requirement_b);

    let assembly = assembly(
        vec![
            artifact_ref(&caller_a),
            artifact_ref(&caller_b),
            artifact_ref(&dependency_a),
            artifact_ref(&dependency_b),
        ],
        vec![
            package_binding(&caller_a, "models", &dependency_a),
            package_binding(&caller_b, "models", &dependency_b),
        ],
    );
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![
            hydration(caller_a, caller_a_file),
            hydration(caller_b, caller_b_file),
            hydration(dependency_a, dependency_a_file),
            hydration(dependency_b, dependency_b_file),
        ],
    )
    .unwrap();
    let symbol = PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: "models".to_string(),
        },
        symbol_path: "models.User".to_string(),
        abi_expectation: None,
    };

    let target_a = image
        .resolve_package_db_object_target(&build_id("caller-a-db"), &symbol)
        .unwrap();
    let target_b = image
        .resolve_package_db_object_target(&build_id("caller-b-db"), &symbol)
        .unwrap();

    assert_ne!(target_a, target_b);
    assert_eq!(
        target_a.package_artifact_ref,
        artifact_ref(
            image
                .code_by_build(&build_id("dependency-a-db"))
                .unwrap()
                .artifact()
        )
    );
    assert_eq!(
        target_b.package_artifact_ref,
        artifact_ref(
            image
                .code_by_build(&build_id("dependency-b-db"))
                .unwrap()
                .artifact()
        )
    );
    assert_eq!(
        target_a.file_ir_ref.file_ir_identity,
        "file:dependency-a-db"
    );
    assert_eq!(
        target_b.file_ir_ref.file_ir_identity,
        "file:dependency-b-db"
    );
    assert_eq!(target_a.type_index, 0);
    assert_eq!(target_b.type_index, 0);
    image.validate_db_object_target_id(&target_a).unwrap();
    image.validate_db_object_target_id(&target_b).unwrap();

    let mut substituted = target_a.clone();
    substituted.package_artifact_ref = target_b.package_artifact_ref.clone();
    assert!(matches!(
        image.validate_db_object_target_id(&substituted),
        Err(SharedPackageImageError::DbTargetFileRefOutsideArtifact { .. })
    ));

    let wrong_abi = PackageSymbolRef {
        abi_expectation: Some("wrong-abi".to_string()),
        ..symbol.clone()
    };
    assert!(matches!(
        image.resolve_package_db_object_target(&build_id("caller-a-db"), &wrong_abi),
        Err(SharedPackageImageError::DbTargetAbiExpectationMismatch { .. })
    ));

    let package_id_target = PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: "dependency.a".to_string(),
        },
        ..symbol
    };
    assert!(matches!(
        image.resolve_package_db_object_target(&build_id("caller-a-db"), &package_id_target),
        Err(SharedPackageImageError::DbTargetRequiresDependencyAlias { .. })
    ));

    let missing_export = PackageSymbolRef {
        symbol_path: "models.Missing".to_string(),
        ..wrong_abi.clone()
    };
    let missing_export = PackageSymbolRef {
        abi_expectation: None,
        ..missing_export
    };
    assert!(matches!(
        image.resolve_package_db_object_target(&build_id("caller-a-db"), &missing_export),
        Err(SharedPackageImageError::MissingDbTargetTypeExport { .. })
    ));

    let missing_binding = PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: "missing".to_string(),
        },
        symbol_path: "models.User".to_string(),
        abi_expectation: None,
    };
    assert!(matches!(
        image.resolve_package_db_object_target(&build_id("caller-a-db"), &missing_binding),
        Err(SharedPackageImageError::MissingPackageRequirement { .. })
    ));

    let mut missing_file = target_a.clone();
    missing_file.file_ir_ref.file_ir_identity = "file:missing".to_string();
    assert!(matches!(
        image.validate_db_object_target_id(&missing_file),
        Err(SharedPackageImageError::DbTargetFileRefOutsideArtifact { .. })
    ));

    let mut missing_type = target_a;
    missing_type.type_index = 99;
    assert!(matches!(
        image.validate_db_object_target_id(&missing_type),
        Err(SharedPackageImageError::DbTargetTypeOutOfBounds { .. })
    ));
}

#[test]
fn foreign_db_target_without_provider_attachment_fails_closed() {
    let caller_file = file("file:caller-db", "caller");
    let mut dependency_file = file("file:dependency-db", "models");
    add_db_declaration(&mut dependency_file, "User");
    dependency_file.declarations.db.clear();

    let mut caller = artifact("caller", "caller-db", "caller-abi", &caller_file);
    let mut dependency = artifact(
        "dependency",
        "dependency-db",
        "dependency-abi",
        &dependency_file,
    );
    add_db_export(&mut dependency, &dependency_file, "models.User", "User");
    let mut requirement = package_requirement("models", "dependency", "dependency-abi");
    requirement.expected_package_build = Some(dependency.package_build_id.clone());
    caller.package_requirements.push(requirement);
    let assembly = assembly(
        vec![artifact_ref(&caller), artifact_ref(&dependency)],
        vec![package_binding(&caller, "models", &dependency)],
    );
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![
            hydration(caller, caller_file),
            hydration(dependency, dependency_file),
        ],
    )
    .unwrap();

    let error = image
        .resolve_package_db_object_target(
            &build_id("caller-db"),
            &PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "models".to_string(),
                },
                symbol_path: "models.User".to_string(),
                abi_expectation: None,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        SharedPackageImageError::MissingDbTargetAttachment { .. }
    ));
}

#[test]
fn foreign_db_target_accepts_compiler_qualified_declaration_shape() {
    let caller_file = file("file:caller-real-db", "caller");
    let mut dependency_file = file("file:dependency-real-db", "model");
    add_compiler_shaped_db_declaration(&mut dependency_file, "Session");

    let mut caller = artifact("caller", "caller-real-db", "caller-abi", &caller_file);
    let mut dependency = artifact(
        "dependency",
        "dependency-real-db",
        "dependency-abi",
        &dependency_file,
    );
    add_db_export(
        &mut dependency,
        &dependency_file,
        "model.Session",
        "Session",
    );
    let mut requirement = package_requirement("model", "dependency", "dependency-abi");
    requirement.expected_package_build = Some(dependency.package_build_id.clone());
    caller.package_requirements.push(requirement);
    let assembly = assembly(
        vec![artifact_ref(&caller), artifact_ref(&dependency)],
        vec![package_binding(&caller, "model", &dependency)],
    );
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![
            hydration(caller, caller_file),
            hydration(dependency, dependency_file),
        ],
    )
    .unwrap();

    let target = image
        .resolve_package_db_object_target(
            &build_id("caller-real-db"),
            &PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "model".to_string(),
                },
                symbol_path: "model.Session".to_string(),
                abi_expectation: Some("dependency-abi".to_string()),
            },
        )
        .unwrap();

    assert_eq!(target.type_index, 0);
    image.validate_db_object_target_id(&target).unwrap();
}

#[test]
fn compiler_shaped_db_attachment_rejects_symbol_and_attachment_tampering() {
    let package_build_id = build_id("dependency-real-db");
    let mut file = file("file:dependency-real-db", "model");
    add_compiler_shaped_db_declaration(&mut file, "Session");

    validate_db_attachment(
        &package_build_id,
        &file,
        0,
        Some("model.Session"),
        Some("Session"),
    )
    .unwrap();

    for (target_symbol, link_symbol) in [("wrong.Session", "Session"), ("model.Session", "Wrong")] {
        let error = validate_db_attachment(
            &package_build_id,
            &file,
            0,
            Some(target_symbol),
            Some(link_symbol),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SharedPackageImageError::DbTargetCanonicalSymbolMismatch { .. }
        ));
    }

    let mut wrong_declaration = file.clone();
    wrong_declaration
        .declarations
        .types
        .get_mut("Session")
        .unwrap()
        .symbol = "wrong.Session".to_string();
    assert!(matches!(
        validate_db_attachment(
            &package_build_id,
            &wrong_declaration,
            0,
            Some("model.Session"),
            Some("Session"),
        )
        .unwrap_err(),
        SharedPackageImageError::DbTargetCanonicalSymbolMismatch { .. }
    ));

    for symbol in [
        skiff_artifact_model::ServiceSymbolRef {
            module_path: "wrong".to_string(),
            symbol: "Session".to_string(),
        },
        skiff_artifact_model::ServiceSymbolRef {
            module_path: "model".to_string(),
            symbol: "Wrong".to_string(),
        },
    ] {
        let mut wrong_attachment = file.clone();
        wrong_attachment
            .declarations
            .db
            .get_mut("Session")
            .unwrap()
            .type_ref = TypeRefIr::DbObjectSymbol { symbol };
        assert!(matches!(
            validate_db_attachment(
                &package_build_id,
                &wrong_attachment,
                0,
                Some("model.Session"),
                Some("Session"),
            )
            .unwrap_err(),
            SharedPackageImageError::DbTargetAttachmentTypeMismatch { .. }
        ));
    }

    let mut wrong_local_index = file;
    wrong_local_index
        .declarations
        .db
        .get_mut("Session")
        .unwrap()
        .type_ref = TypeRefIr::LocalType { type_index: 1 };
    assert!(matches!(
        validate_db_attachment(
            &package_build_id,
            &wrong_local_index,
            0,
            Some("model.Session"),
            Some("Session"),
        )
        .unwrap_err(),
        SharedPackageImageError::DbTargetAttachmentTypeMismatch { .. }
    ));
}

#[test]
fn assembly_execution_service_calls_keep_caller_relative_tuple_and_never_select_provider_code() {
    let service_call = ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: ContractOperationId::new("operation:echo"),
        expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
    };
    let mut caller_a_file = file("file:caller-a", "caller.a");
    add_service_call(&mut caller_a_file, service_call.clone());
    let mut caller_b_file = file("file:caller-b", "caller.b");
    add_service_call(&mut caller_b_file, service_call.clone());
    let provider_file = file("file:provider", "provider.main");

    let mut caller_a = artifact("caller.a", "caller-a", "caller-a-abi", &caller_a_file);
    add_service_requirement(&mut caller_a, &service_call);
    let mut caller_b = artifact("caller.b", "caller-b", "caller-b-abi", &caller_b_file);
    add_service_requirement(&mut caller_b, &service_call);
    let mut provider = artifact("provider", "provider", "provider-abi", &provider_file);
    add_callable(&mut provider, &provider_file, callable_id("provider-echo"));

    let assembly = assembly(
        vec![
            artifact_ref(&caller_a),
            artifact_ref(&caller_b),
            artifact_ref(&provider),
        ],
        Vec::new(),
    );
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![
            hydration(provider, provider_file),
            hydration(caller_a, caller_a_file),
            hydration(caller_b, caller_b_file),
        ],
    )
    .unwrap();

    let linked_a = image
        .resolve_activation_relative_service_call(
            &build_id("caller-a"),
            "file:caller-a",
            ServiceCallRefIndex::new(0),
        )
        .unwrap();
    let linked_b = image
        .resolve_activation_relative_service_call(
            &build_id("caller-b"),
            "file:caller-b",
            ServiceCallRefIndex::new(0),
        )
        .unwrap();

    assert_eq!(linked_a.caller_package_build_id(), &build_id("caller-a"));
    assert_eq!(linked_b.caller_package_build_id(), &build_id("caller-b"));
    assert_eq!(linked_a.service_requirement_slot(), 0);
    assert_eq!(linked_b.service_requirement_slot(), 0);
    assert_eq!(
        linked_a.operation_id(),
        &ContractOperationId::new("operation:echo")
    );
    assert_eq!(
        linked_a.expected_protocol_identity(),
        &ServiceProtocolIdentity::new("protocol:echo")
    );
    assert!(!format!("{linked_a:?}").contains("provider"));
    assert!(!format!("{linked_a:?}").contains("executable"));
}

#[test]
fn assembly_execution_one_code_owner_is_shared_without_activation_owned_state() {
    let package_file = file("file:shared", "shared.main");
    let package = artifact("shared", "shared-build", "shared-abi", &package_file);
    let assembly = assembly(vec![artifact_ref(&package)], Vec::new());
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![hydration(package, package_file)],
    )
    .unwrap();

    let by_build = Arc::clone(image.code_by_build(&build_id("shared-build")).unwrap());
    let by_slot = Arc::clone(image.code_by_slot(PackageCodeSlotIndex::new(0)).unwrap());
    let activation_a_code = Arc::clone(&by_build);
    let activation_b_code = Arc::clone(&by_build);

    assert!(Arc::ptr_eq(&by_build, &by_slot));
    assert!(Arc::ptr_eq(&activation_a_code, &activation_b_code));
    assert_eq!(image.code_slots().len(), 1);
}

#[test]
fn duplicate_build_with_different_content_is_rejected() {
    let file_a = file("file:a", "duplicate.a");
    let file_b = file("file:b", "duplicate.b");
    let package_a = artifact("duplicate", "same-build", "abi-a", &file_a);
    let package_b = artifact("duplicate", "same-build", "abi-b", &file_b);
    let assembly = assembly(
        vec![artifact_ref(&package_a), artifact_ref(&package_b)],
        Vec::new(),
    );

    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            vec![hydration(package_a, file_a), hydration(package_b, file_b)],
        ),
        Err(SharedPackageImageError::DuplicateHydratedPackage { build_id: actual_build_id })
            if actual_build_id == build_id("same-build")
    ));
}

#[test]
fn assembly_execution_wrong_expected_local_abi_is_rejected_before_image_is_returned() {
    let caller_file = file("file:caller", "caller.main");
    let dependency_file = file("file:dependency", "dependency.main");
    let mut caller = artifact("caller", "caller", "caller-abi", &caller_file);
    caller
        .package_requirements
        .push(package_requirement("tools", "dependency", "wrong-abi"));
    let dependency = artifact("dependency", "dependency", "actual-abi", &dependency_file);
    let assembly = assembly(
        vec![artifact_ref(&caller), artifact_ref(&dependency)],
        vec![package_binding(&caller, "tools", &dependency)],
    );

    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            vec![
                hydration(caller, caller_file),
                hydration(dependency, dependency_file),
            ],
        ),
        Err(SharedPackageImageError::PackageRequirementLocalAbiMismatch { expected, actual, .. })
            if expected == local_abi("wrong-abi") && actual == local_abi("actual-abi")
    ));
}

#[test]
fn assembly_execution_wrong_expected_implementation_build_is_rejected_before_image_is_returned() {
    let caller_file = file("file:caller", "caller.main");
    let dependency_file = file("file:dependency", "dependency.main");
    let mut caller = artifact("caller", "caller", "caller-abi", &caller_file);
    let mut requirement = package_requirement("tools", "dependency", "dependency-abi");
    requirement.expected_package_build = Some(build_id("stale-dependency-build"));
    caller.package_requirements.push(requirement);
    let dependency = artifact(
        "dependency",
        "current-dependency-build",
        "dependency-abi",
        &dependency_file,
    );
    let assembly = assembly(
        vec![artifact_ref(&caller), artifact_ref(&dependency)],
        vec![package_binding(&caller, "tools", &dependency)],
    );

    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            vec![
                hydration(caller, caller_file),
                hydration(dependency, dependency_file),
            ],
        ),
        Err(SharedPackageImageError::PackageRequirementBuildMismatch {
            expected,
            actual,
            ..
        }) if expected == build_id("stale-dependency-build")
            && actual == build_id("current-dependency-build")
    ));
}

#[test]
fn assembly_execution_missing_callable_is_rejected_while_validating_linked_call_sites() {
    let missing = callable_id("missing");
    let mut caller_file = file("file:caller", "caller.main");
    add_package_call(&mut caller_file, "tools", missing.clone());
    let dependency_file = file("file:dependency", "dependency.main");
    let mut caller = artifact("caller", "caller", "caller-abi", &caller_file);
    caller
        .package_requirements
        .push(package_requirement("tools", "dependency", "dependency-abi"));
    let dependency = artifact(
        "dependency",
        "dependency",
        "dependency-abi",
        &dependency_file,
    );
    let assembly = assembly(
        vec![artifact_ref(&caller), artifact_ref(&dependency)],
        vec![package_binding(&caller, "tools", &dependency)],
    );

    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            vec![
                hydration(caller, caller_file),
                hydration(dependency, dependency_file),
            ],
        ),
        Err(SharedPackageImageError::MissingPackageCallable {
            dependency_package_build_id,
            package_callable_id,
        }) if dependency_package_build_id == build_id("dependency") && package_callable_id == missing
    ));
}

#[test]
fn storage_locator_is_ignored_while_executable_target_semantics_stay_strict() {
    let callable = callable_id("entry");
    let package_file = file("file:package", "package.main");
    let mut package = artifact("package", "package-build", "package-abi", &package_file);
    add_callable(&mut package, &package_file, callable.clone());
    let target = package.callable_links[&callable].target.clone();
    package.files[0].artifact_path = Some("records/packages/package/file-ir.json".to_string());
    let assembly = assembly(vec![artifact_ref(&package)], Vec::new());
    let image = SharedPackageLinkedImage::from_runtime_assembly(
        &assembly,
        vec![hydration(package, package_file)],
    )
    .expect("storage locator is not part of target semantics");
    let code = image.code_by_build(&build_id("package-build")).unwrap();

    assert_eq!(
        code.executable_addr(&target).unwrap(),
        ExecutableAddr::package(0, 0, 0)
    );

    let mut without_source_hash = target.clone();
    without_source_hash.file_ref.source_ast_hash = None;
    assert_eq!(
        code.executable_addr(&without_source_hash).unwrap(),
        ExecutableAddr::package(0, 0, 0)
    );

    let mut wrong_identity = target.clone();
    wrong_identity.file_ref.file_ir_identity = "file:missing".to_string();
    assert!(matches!(
        code.executable_addr(&wrong_identity),
        Err(SharedPackageImageError::ExecutableTargetFileNotLoaded { .. })
    ));

    let mut wrong_module = target.clone();
    wrong_module.file_ref.module_path = "tampered.module".to_string();
    assert!(matches!(
        code.executable_addr(&wrong_module),
        Err(SharedPackageImageError::ExecutableTargetFileRefMismatch { .. })
    ));

    let mut wrong_source_hash = target.clone();
    wrong_source_hash.file_ref.source_ast_hash = Some("source:tampered".to_string());
    assert!(matches!(
        code.executable_addr(&wrong_source_hash),
        Err(SharedPackageImageError::ExecutableTargetFileRefMismatch { .. })
    ));

    let mut wrong_index = target;
    wrong_index.executable_index = 99;
    assert!(matches!(
        code.executable_addr(&wrong_index),
        Err(SharedPackageImageError::ExecutableTargetOutOfBounds {
            executable_index: 99,
            ..
        })
    ));
}

#[test]
fn assembly_execution_tampered_callable_identity_module_hash_and_index_fail_closed() {
    let callable = callable_id("entry");
    let package_file = file("file:package", "package.main");
    let mut package = artifact("package", "package-build", "package-abi", &package_file);
    add_callable(&mut package, &package_file, callable.clone());

    let mut wrong_identity = package.clone();
    wrong_identity
        .callable_links
        .get_mut(&callable)
        .unwrap()
        .target
        .file_ref
        .file_ir_identity = "file:missing".to_string();
    let wrong_identity_assembly = assembly(vec![artifact_ref(&wrong_identity)], Vec::new());
    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &wrong_identity_assembly,
            vec![hydration(wrong_identity, package_file.clone())],
        ),
        Err(SharedPackageImageError::CallableTargetFileNotLoaded { .. })
    ));

    let mut wrong_file_ref = package.clone();
    wrong_file_ref
        .callable_links
        .get_mut(&callable)
        .unwrap()
        .target
        .file_ref
        .module_path = "tampered.module".to_string();
    let wrong_file_assembly = assembly(vec![artifact_ref(&wrong_file_ref)], Vec::new());
    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &wrong_file_assembly,
            vec![hydration(wrong_file_ref, package_file.clone())],
        ),
        Err(SharedPackageImageError::CallableTargetFileRefMismatch { .. })
    ));

    let mut wrong_source_hash = package.clone();
    wrong_source_hash
        .callable_links
        .get_mut(&callable)
        .unwrap()
        .target
        .file_ref
        .source_ast_hash = Some("source:tampered".to_string());
    let wrong_source_hash_assembly = assembly(vec![artifact_ref(&wrong_source_hash)], Vec::new());
    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &wrong_source_hash_assembly,
            vec![hydration(wrong_source_hash, package_file.clone())],
        ),
        Err(SharedPackageImageError::CallableTargetFileRefMismatch { .. })
    ));

    package
        .callable_links
        .get_mut(&callable)
        .unwrap()
        .target
        .executable_index = 99;
    let assembly = assembly(vec![artifact_ref(&package)], Vec::new());
    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            vec![hydration(package, package_file)],
        ),
        Err(
            SharedPackageImageError::CallableTargetExecutableOutOfBounds {
                executable_index: 99,
                ..
            }
        )
    ));
}

#[test]
fn assembly_execution_wrong_service_protocol_is_rejected_without_provider_patching() {
    let service_call = ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: ContractOperationId::new("operation:echo"),
        expected_protocol_identity: ServiceProtocolIdentity::new("protocol:wrong"),
    };
    let mut caller_file = file("file:caller", "caller.main");
    add_service_call(&mut caller_file, service_call.clone());
    let mut caller = artifact("caller", "caller", "caller-abi", &caller_file);
    caller.service_call_refs.push(service_call.clone());
    caller.service_requirements.push(ServiceRequirement {
        contract_requirement: ContractRequirement {
            alias: "echo".to_string(),
            service_id: "echo.service".to_string(),
            contract_version: "1.0.0".to_string(),
            expected_protocol_identity: ServiceProtocolIdentity::new("protocol:expected"),
        },
        service_binding_slot: 0,
        used_operations: BTreeSet::from([ContractOperationId::new("operation:echo")]),
    });
    let assembly = assembly(vec![artifact_ref(&caller)], Vec::new());

    assert!(matches!(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            vec![hydration(caller, caller_file)],
        ),
        Err(SharedPackageImageError::ServiceCallProtocolMismatch { expected, actual, .. })
            if expected == ServiceProtocolIdentity::new("protocol:expected")
                && actual == ServiceProtocolIdentity::new("protocol:wrong")
    ));
}

fn assembly(
    package_refs: Vec<PackageArtifactRef>,
    package_links: Vec<PackageBinding>,
) -> RuntimeAssembly {
    RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:test"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: package_refs
                .iter()
                .cloned()
                .map(|package| PackageCodeSlot { package })
                .collect(),
            package_links,
        },
        resolved_packages: package_refs,
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    }
}

fn artifact(
    package_id: &str,
    build: &str,
    local_abi_identity: &str,
    file: &FileIrUnit,
) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: build_id(build),
        files: vec![file_ref(file)],
        static_resources: Vec::new(),
        bytecode: None,
        bytecode_statement_manifest_identity: derive_bytecode_statement_manifest_identity(
            package_id,
            &[],
        )
        .expect("empty package statement manifest should be canonical"),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: local_abi(local_abi_identity),
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
    }
}

fn artifact_ref(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn file(identity: &str, module_path: &str) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{identity}"));
    file.file_ir_identity = identity.to_string();
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: format!("{module_path}.entry"),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("unknown"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    });
    file
}

fn add_package_call(file: &mut FileIrUnit, alias: &str, callable: PackageCallableId) {
    let package_ref = PackageRefIr::Dependency {
        dependency_ref: alias.to_string(),
    };
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: package_ref.clone(),
            package_callable_id: callable.clone(),
        });
    file.executables[0].body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::PackageCallable {
                package_ref,
                package_callable_id: callable,
            },
            concrete_receiver: None,
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
}

fn add_service_call(file: &mut FileIrUnit, service_call: ServiceCallRef) {
    let index = ServiceCallRefIndex::try_from(file.external_refs.service_call_refs.len()).unwrap();
    file.external_refs.service_call_refs.push(service_call);
    file.executables[0].body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::ServiceCall {
                service_call_ref_index: index,
            },
            concrete_receiver: None,
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
}

fn add_callable(artifact: &mut PackageArtifact, file: &FileIrUnit, callable: PackageCallableId) {
    add_callable_kind(
        artifact,
        file,
        callable,
        OperationCallableKind::PublicFunction,
    );
}

fn add_callable_kind(
    artifact: &mut PackageArtifact,
    file: &FileIrUnit,
    callable: PackageCallableId,
    callable_kind: OperationCallableKind,
) {
    artifact.callable_links.insert(
        callable.clone(),
        PackageCallableLinkFact {
            callable_id: callable.clone(),
            target: OperationTargetRef {
                file_ref: file_ref(file),
                executable_index: 0,
                callable_abi_id: callable.to_string(),
                callable_kind,
            },
        },
    );
}

fn add_const(file: &mut FileIrUnit, name: &str) {
    file.constants.push(ConstIr {
        name: name.to_string(),
        ty: TypeRefIr::builtin("unknown"),
        body: ExecutableBody::default(),
        source_span: None,
    });
}

fn public_instance_symbol<const N: usize>(
    public_path: &str,
    methods: [(&str, PackageCallableId); N],
    declared_receiver_type: TypeRefIr,
    interfaces: Vec<TypeRefIr>,
) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::PublicInstance {
        instance_id: public_path.to_string(),
        declared_receiver_type,
        interfaces,
        methods: methods
            .into_iter()
            .map(|(name, callable)| (name.to_string(), callable))
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
fn add_public_instance<const N: usize>(
    artifact: &mut PackageArtifact,
    file: &FileIrUnit,
    public_path: &str,
    const_index: u32,
    methods: [(&str, PackageCallableId); N],
    declared_receiver_type: TypeRefIr,
    interfaces: Vec<TypeRefIr>,
) {
    artifact.implementation_links.constants.insert(
        public_path.to_string(),
        ConstExport {
            file: file_ref(file),
            const_index,
            symbol: public_path.to_string(),
            ty: declared_receiver_type.clone(),
        },
    );
    artifact.package_local_abi.public_symbols.insert(
        public_path.to_string(),
        public_instance_symbol(public_path, methods, declared_receiver_type, interfaces),
    );
}

fn one_package_image(
    package: PackageArtifact,
    file: FileIrUnit,
) -> SharedPackageImageResult<SharedPackageLinkedImage> {
    let package_ref = artifact_ref(&package);
    SharedPackageLinkedImage::from_runtime_assembly(
        &assembly(vec![package_ref], Vec::new()),
        vec![hydration(package, file)],
    )
}

fn add_service_requirement(artifact: &mut PackageArtifact, service_call: &ServiceCallRef) {
    artifact.service_call_refs.push(service_call.clone());
    artifact.service_requirements.push(ServiceRequirement {
        contract_requirement: ContractRequirement {
            alias: "echo".to_string(),
            service_id: "echo.service".to_string(),
            contract_version: "1.0.0".to_string(),
            expected_protocol_identity: service_call.expected_protocol_identity.clone(),
        },
        service_binding_slot: service_call.service_requirement_slot,
        used_operations: BTreeSet::from([service_call.contract_operation_id.clone()]),
    });
}

fn add_db_declaration(file: &mut FileIrUnit, symbol: &str) {
    let type_index = file.type_table.len() as u32;
    file.type_table.push(TypeDeclIr {
        name: symbol.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        symbol.to_string(),
        TypeDeclarationIr {
            type_index,
            symbol: symbol.to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        symbol.to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index },
            type_name: symbol.to_string(),
            collection_name: Some(symbol.to_ascii_lowercase()),
            implements: None,
            identity_fields: std::collections::BTreeMap::new(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: Vec::new(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
}

fn add_compiler_shaped_db_declaration(file: &mut FileIrUnit, symbol: &str) {
    let type_index = file.type_table.len() as u32;
    let qualified = format!("{}.{}", file.module_path, symbol);
    file.type_table.push(TypeDeclIr {
        name: symbol.to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        symbol.to_string(),
        TypeDeclarationIr {
            type_index,
            symbol: qualified,
            source_span: None,
        },
    );
    file.declarations.db.insert(
        symbol.to_string(),
        DbDeclarationIr {
            type_ref: TypeRefIr::DbObjectSymbol {
                symbol: skiff_artifact_model::ServiceSymbolRef {
                    module_path: file.module_path.clone(),
                    symbol: symbol.to_string(),
                },
            },
            type_name: symbol.to_string(),
            collection_name: Some(symbol.to_ascii_lowercase()),
            implements: None,
            identity_fields: std::collections::BTreeMap::new(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: Vec::new(),
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
}

fn add_db_export(
    artifact: &mut PackageArtifact,
    file: &FileIrUnit,
    symbol_path: &str,
    symbol: &str,
) {
    let type_index = file.declarations.types[symbol].type_index;
    artifact.implementation_links.types.insert(
        symbol_path.to_string(),
        TypeExport {
            file: file_ref(file),
            type_index,
            symbol: symbol.to_string(),
            is_interface: false,
            descriptor: Some(file.type_table[type_index as usize].descriptor.clone()),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
}

fn package_requirement(
    alias: &str,
    package_id: &str,
    local_abi_identity: &str,
) -> PackageRequirement {
    PackageRequirement {
        alias: alias.to_string(),
        package_id: package_id.to_string(),
        exact_version: "1.0.0".to_string(),
        expected_local_abi: local_abi(local_abi_identity),
        expected_package_build: None,
    }
}

fn package_binding(
    caller: &PackageArtifact,
    alias: &str,
    dependency: &PackageArtifact,
) -> PackageBinding {
    PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: caller.package_build_id.clone(),
            package_requirement_alias: alias.to_string(),
        },
        package: artifact_ref(dependency),
    }
}

fn hydration(artifact: PackageArtifact, file: FileIrUnit) -> HydratedPackageCode {
    let schema_index = empty_schema_index(&artifact);
    HydratedPackageCode::new(
        Arc::new(artifact),
        vec![Arc::new(file)],
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::new(schema_index))
}

fn hydration_with_resources<const N: usize>(
    artifact: PackageArtifact,
    file: FileIrUnit,
    resources: [(&str, &[u8]); N],
) -> HydratedPackageCode {
    let schema_index = empty_schema_index(&artifact);
    let mut table = PublicationResourceTable::default();
    for (path, bytes) in resources {
        let meta = artifact
            .static_resources
            .iter()
            .find(|resource| resource.path == path)
            .unwrap()
            .clone();
        table.insert(
            path.to_string(),
            LoadedPublicationResource {
                meta,
                bytes: Arc::from(bytes),
            },
        );
    }
    HydratedPackageCode::new(Arc::new(artifact), vec![Arc::new(file)], table)
        .with_schema_index(Arc::new(schema_index))
}

fn empty_schema_index(artifact: &PackageArtifact) -> PackageSchemaIndex {
    PackageSchemaIndex {
        package_id: artifact.package_id.clone(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    }
}

fn add_static_resource(artifact: &mut PackageArtifact, path: &str, bytes: &[u8]) {
    artifact.static_resources.push(PublicationResourceRef {
        path: path.to_string(),
        sha256: "sha256:test".to_string(),
        byte_len: bytes.len() as u64,
        content_type: Some("text/plain".to_string()),
        artifact_path: None,
    });
}

fn build_id(value: &str) -> PackageBuildId {
    PackageBuildId::new(value)
}

fn local_abi(value: &str) -> PackageLocalAbiIdentity {
    PackageLocalAbiIdentity::new(value)
}

fn callable_id(value: &str) -> PackageCallableId {
    PackageCallableId::new(format!("callable:{value}"))
}
