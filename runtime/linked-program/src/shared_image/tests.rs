use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    AssemblyIdentity, CallIr, CallTargetIr, CanonicalPackageLinkPlan, ContractOperationId,
    ContractRequirement, ExecutableBody, ExecutableIr, ExecutableKind, ExprIr, FileIrRef,
    FileIrUnit, OperationCallableKind, PackageArtifact, PackageArtifactRef, PackageBinding,
    PackageBuildId, PackageCallableId, PackageCallableLinkFact, PackageCallableRef,
    PackageCodeSlot, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRefIr, PackageRequirement, PackageRequirementKey, PackageRuntimeRequirements,
    PackageSchemaIndexRef, PublicationResourceRef, RuntimeAssembly, ServiceCallRef,
    ServiceProtocolIdentity, ServiceRequirement, SlotLayout, TypeRefIr,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
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
        global_ingress: Vec::new(),
    }
}

fn artifact(
    package_id: &str,
    build: &str,
    local_abi_identity: &str,
    file: &FileIrUnit,
) -> PackageArtifact {
    PackageArtifact {
        schema_version: "skiff-package-artifact-v2".to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: build_id(build),
        files: vec![file_ref(file)],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: local_abi(local_abi_identity),
            public_symbols: BTreeMap::new(),
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
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
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
            args: Vec::new(),
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
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
}

fn add_callable(artifact: &mut PackageArtifact, file: &FileIrUnit, callable: PackageCallableId) {
    artifact.callable_links.insert(
        callable.clone(),
        PackageCallableLinkFact {
            callable_id: callable.clone(),
            target: OperationTargetRef {
                file_ref: file_ref(file),
                executable_index: 0,
                callable_abi_id: callable.to_string(),
                callable_kind: OperationCallableKind::PublicFunction,
            },
        },
    );
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
    HydratedPackageCode::new(
        Arc::new(artifact),
        vec![Arc::new(file)],
        PublicationResourceTable::default(),
    )
}

fn hydration_with_resources<const N: usize>(
    artifact: PackageArtifact,
    file: FileIrUnit,
    resources: [(&str, &[u8]); N],
) -> HydratedPackageCode {
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
