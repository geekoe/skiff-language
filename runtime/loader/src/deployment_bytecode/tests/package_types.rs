use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorAbiInput, InterfaceMethodSignature, PackageActorAbi, PackageBinding,
    PackageBuildId, PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRefIr,
    PackageRequirement, PackageRequirementKey, PackageSymbolRef, TypeDescriptorIr, TypeRefIr,
};

use super::*;

const DEPENDENCY_ALIAS: &str = "dependency";
const TYPE_PATH: &str = "model.Value";

#[derive(Clone, Copy)]
enum TypeReferenceKind {
    DependencyAlias,
    ExactPackageId,
}

#[derive(Clone, Copy)]
enum RequirementMode {
    Missing,
    Unpinned,
    Exact,
    ExactWithoutBinding,
    WrongVersion,
    WrongAbi,
    WrongBuild,
    DuplicateExact,
}

fn package_type_symbol() -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Type {
        local_type_id: "type:example.type-provider:model.Value".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        is_alias: false,
        is_interface: false,
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: None,
    }
}

fn requirement_for(
    target: &skiff_artifact_model::PackageArtifactRef,
    alias: &str,
    expected_package_build: Option<PackageBuildId>,
) -> PackageRequirement {
    PackageRequirement {
        alias: alias.to_string(),
        package_id: target.package_id.clone(),
        exact_version: target.package_version.clone(),
        expected_local_abi: target.package_local_abi_identity.clone(),
        expected_package_build,
    }
}

fn validate_dependency_type(
    reference_kind: TypeReferenceKind,
    requirement_mode: RequirementMode,
    public: Option<PackageLocalAbiSymbol>,
    implementation: Option<PackageLocalAbiSymbol>,
) -> Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError> {
    let target_bytecode = admitted_bytecode("type-provider");
    let mut target_artifact = package_artifact(
        "example.type-provider",
        "build:type-provider",
        Some(target_bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    if let Some(public) = public {
        target_artifact
            .package_local_abi
            .public_symbols
            .insert(TYPE_PATH.to_string(), public);
    }
    if let Some(implementation) = implementation {
        target_artifact
            .package_local_abi
            .implementation_symbols
            .insert(TYPE_PATH.to_string(), implementation);
    }
    let target_artifact = Arc::new(target_artifact);
    let target_reference = package_reference(&target_artifact);
    let target = HydratedBytecodePackage::checked(
        target_reference.clone(),
        target_artifact,
        target_bytecode,
    )
    .unwrap();

    let package_ref = match reference_kind {
        TypeReferenceKind::DependencyAlias => PackageRefIr::Dependency {
            dependency_ref: DEPENDENCY_ALIAS.to_string(),
        },
        TypeReferenceKind::ExactPackageId => PackageRefIr::PackageId {
            package_id: target_reference.package_id.clone(),
        },
    };
    let caller_bytecode = bytecode_with_type_root(
        "type-consumer",
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: package_ref,
                symbol_path: TYPE_PATH.to_string(),
                abi_expectation: Some(
                    target_reference
                        .package_local_abi_identity
                        .as_str()
                        .to_string(),
                ),
            },
        },
    );
    let mut caller_artifact = package_artifact(
        "example.type-consumer",
        "build:type-consumer",
        Some(caller_bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    caller_artifact.package_requirements = match requirement_mode {
        RequirementMode::Missing => Vec::new(),
        RequirementMode::Unpinned => {
            vec![requirement_for(&target_reference, DEPENDENCY_ALIAS, None)]
        }
        RequirementMode::Exact => vec![requirement_for(
            &target_reference,
            DEPENDENCY_ALIAS,
            Some(target_reference.package_build_id.clone()),
        )],
        RequirementMode::ExactWithoutBinding => vec![requirement_for(
            &target_reference,
            DEPENDENCY_ALIAS,
            Some(target_reference.package_build_id.clone()),
        )],
        RequirementMode::WrongVersion => {
            let mut requirement = requirement_for(
                &target_reference,
                DEPENDENCY_ALIAS,
                Some(target_reference.package_build_id.clone()),
            );
            requirement.exact_version = "9.9.9".to_string();
            vec![requirement]
        }
        RequirementMode::WrongAbi => {
            let mut requirement = requirement_for(
                &target_reference,
                DEPENDENCY_ALIAS,
                Some(target_reference.package_build_id.clone()),
            );
            requirement.expected_local_abi =
                PackageLocalAbiIdentity::new("abi:wrong-type-provider");
            vec![requirement]
        }
        RequirementMode::WrongBuild => vec![requirement_for(
            &target_reference,
            DEPENDENCY_ALIAS,
            Some(PackageBuildId::new("build:wrong-type-provider")),
        )],
        RequirementMode::DuplicateExact => vec![
            requirement_for(
                &target_reference,
                DEPENDENCY_ALIAS,
                Some(target_reference.package_build_id.clone()),
            ),
            requirement_for(
                &target_reference,
                "duplicate-dependency",
                Some(target_reference.package_build_id.clone()),
            ),
        ],
    };
    let caller_artifact = Arc::new(caller_artifact);
    let caller_reference = package_reference(&caller_artifact);
    let caller = HydratedBytecodePackage::checked(
        caller_reference.clone(),
        caller_artifact,
        caller_bytecode,
    )
    .unwrap();

    let own_contract = contract_reference("example.type-consumer-service");
    let mut deployment = deployment(caller_reference.clone(), own_contract.clone(), Vec::new())
        .as_ref()
        .clone();
    if !matches!(
        requirement_mode,
        RequirementMode::Missing | RequirementMode::ExactWithoutBinding
    ) {
        deployment.package_bindings.push(PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: caller_reference.package_build_id.clone(),
                package_requirement_alias: DEPENDENCY_ALIAS.to_string(),
            },
            package: target_reference.clone(),
        });
    }
    if matches!(requirement_mode, RequirementMode::DuplicateExact) {
        deployment.package_bindings.push(PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: caller_reference.package_build_id,
                package_requirement_alias: "duplicate-dependency".to_string(),
            },
            package: target_reference,
        });
    }
    let deployment = Arc::new(deployment);
    HydratedDeploymentBytecode::checked(
        deployment_reference(&deployment),
        deployment,
        BTreeMap::from([(own_contract.clone(), contract(&own_contract))]),
        Vec::new(),
        vec![caller, target],
    )
}

fn assert_package_reference_rejected(
    result: Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError>,
    detail_fragment: &str,
) {
    let error = result.expect_err("package type authority must fail closed");
    let (kind, detail) = match error {
        DeploymentBytecodeHydrationError::ManifestMismatch { kind, detail, .. } => (kind, detail),
        other => panic!("unexpected package type authority error: {other}"),
    };
    assert_eq!(kind, DeploymentBytecodeManifestKind::PackageReference);
    assert!(detail.contains(detail_fragment), "{detail}");
}

#[test]
fn dependency_public_type_remains_available_without_build_pin() {
    let public = package_type_symbol();
    validate_dependency_type(
        TypeReferenceKind::DependencyAlias,
        RequirementMode::Unpinned,
        Some(public.clone()),
        Some(public),
    )
    .unwrap();
}

#[test]
fn dependency_private_type_requires_exact_build_pin() {
    validate_dependency_type(
        TypeReferenceKind::DependencyAlias,
        RequirementMode::Exact,
        None,
        Some(package_type_symbol()),
    )
    .unwrap();

    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::DependencyAlias,
            RequirementMode::Unpinned,
            None,
            Some(package_type_symbol()),
        ),
        "requires exact build pin",
    );
    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::DependencyAlias,
            RequirementMode::WrongBuild,
            None,
            Some(package_type_symbol()),
        ),
        "binding violates its exact ABI/build requirement",
    );
}

#[test]
fn normalized_package_id_private_type_recovers_one_exact_requirement() {
    validate_dependency_type(
        TypeReferenceKind::ExactPackageId,
        RequirementMode::Exact,
        None,
        Some(package_type_symbol()),
    )
    .unwrap();

    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::ExactPackageId,
            RequirementMode::Missing,
            None,
            Some(package_type_symbol()),
        ),
        "has no unique direct exact version/ABI/build requirement and binding",
    );
    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::ExactPackageId,
            RequirementMode::DuplicateExact,
            None,
            Some(package_type_symbol()),
        ),
        "has no unique direct exact version/ABI/build requirement and binding",
    );
    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::ExactPackageId,
            RequirementMode::ExactWithoutBinding,
            None,
            Some(package_type_symbol()),
        ),
        "has no unique direct exact version/ABI/build requirement and binding",
    );
    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::ExactPackageId,
            RequirementMode::WrongVersion,
            None,
            Some(package_type_symbol()),
        ),
        "has no unique direct exact version/ABI/build requirement and binding",
    );
    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::ExactPackageId,
            RequirementMode::WrongAbi,
            None,
            Some(package_type_symbol()),
        ),
        "has no unique direct exact version/ABI/build requirement and binding",
    );
    assert_package_reference_rejected(
        validate_dependency_type(
            TypeReferenceKind::ExactPackageId,
            RequirementMode::WrongBuild,
            None,
            Some(package_type_symbol()),
        ),
        "has no unique direct exact version/ABI/build requirement and binding",
    );
}

#[test]
fn self_package_id_private_type_remains_available_without_requirement() {
    let package_id = "example.self-type";
    let bytecode = bytecode_with_type_root(
        "self-type",
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path: TYPE_PATH.to_string(),
                abi_expectation: Some(format!("abi:{package_id}")),
            },
        },
    );
    let mut artifact = package_artifact(
        package_id,
        "build:self-type",
        Some(bytecode.reference().clone()),
    )
    .as_ref()
    .clone();
    artifact
        .package_local_abi
        .implementation_symbols
        .insert(TYPE_PATH.to_string(), package_type_symbol());
    let artifact = Arc::new(artifact);
    let package =
        HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode).unwrap();
    let reference = package.reference().clone();

    hydrated_deployment(reference, vec![package]).unwrap();
}

#[test]
fn public_and_implementation_type_semantic_drift_fails_closed() {
    let public = package_type_symbol();
    let mut conflicts = Vec::new();

    let mut descriptor = public.clone();
    let PackageLocalAbiSymbol::Type {
        descriptor: value, ..
    } = &mut descriptor
    else {
        unreachable!();
    };
    *value = TypeDescriptorIr::Alias {
        target: TypeRefIr::builtin("string"),
    };
    conflicts.push(("descriptor", descriptor));

    let mut alias = public.clone();
    let PackageLocalAbiSymbol::Type { is_alias, .. } = &mut alias else {
        unreachable!();
    };
    *is_alias = true;
    conflicts.push(("isAlias", alias));

    let mut interface = public.clone();
    let PackageLocalAbiSymbol::Type { is_interface, .. } = &mut interface else {
        unreachable!();
    };
    *is_interface = true;
    conflicts.push(("isInterface", interface));

    let mut type_params = public.clone();
    let PackageLocalAbiSymbol::Type {
        type_params: value, ..
    } = &mut type_params
    else {
        unreachable!();
    };
    *value = vec!["T".to_string()];
    conflicts.push(("typeParams", type_params));

    let mut methods = public.clone();
    let PackageLocalAbiSymbol::Type {
        interface_methods, ..
    } = &mut methods
    else {
        unreachable!();
    };
    *interface_methods = vec![InterfaceMethodSignature {
        name: "read".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }];
    conflicts.push(("interfaceMethods", methods));

    let mut actor = public.clone();
    let PackageLocalAbiSymbol::Type { actor: value, .. } = &mut actor else {
        unreachable!();
    };
    *value = Some(PackageActorAbi {
        actor_abi_identity: ActorAbiIdentity::new("actor-abi:drift"),
        abi: ActorAbiInput {
            actor_name: "Value".to_string(),
            actor_id_type: TypeRefIr::builtin("string"),
            key_field: "id".to_string(),
            fields: Vec::new(),
            create: None,
            public_methods: Vec::new(),
            actor_runtime_abi_version: "skiff-actor-runtime-abi-v1".to_string(),
        },
    });
    conflicts.push(("actor", actor));

    for (field, implementation) in conflicts {
        let result = validate_dependency_type(
            TypeReferenceKind::DependencyAlias,
            RequirementMode::Exact,
            Some(public.clone()),
            Some(implementation),
        );
        assert!(result.is_err(), "{field} drift must fail closed");
        assert_package_reference_rejected(result, "different public and implementation semantics");
    }
}
