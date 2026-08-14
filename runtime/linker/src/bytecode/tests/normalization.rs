use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodePoolEntry, FunctionTypeParamIr, InterfaceInstantiationRef, NominalTypeRefBaseIr,
    PackageCallableId, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, LinkedShapeEntry, LinkedShapeField, LinkedValueDropPlan,
    LinkedValueTransferPlan, SpecializationKey,
};
use skiff_runtime_loader::{
    DeploymentBytecodeHydrationError, HydratedBytecodePackage, HydratedDeploymentBytecode,
};

use crate::bytecode::{
    types::{normalize_type, TypeLinker},
    BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

use super::fixtures::{
    self, DependencyTypeSurfaceConflict, Fixture, DEPENDENCY_ALIAS, DEPENDENCY_PACKAGE_ID,
    OWNER_IMPLEMENTATION_PATH, OWNER_PUBLIC_PATH, PRIVATE_IMPLEMENTATION_PATH, ROOT_CALLABLE,
};
use super::generous_limits;

#[test]
fn type_linker_interns_only_the_owner_complete_type() {
    let hydrated = Fixture::normalization().hydrate();
    let package = implementation_package(&hydrated);
    let source = TypeRefIr::PublicationType {
        module_path: "fixture".to_string(),
        type_index: 0,
    };
    let artifact_index = package
        .bytecode()
        .unwrap()
        .view()
        .pools()
        .types
        .iter()
        .position(|entry| matches!(entry, BytecodePoolEntry::TypeRef { ty } if *ty == source))
        .and_then(|index| u32::try_from(index).ok())
        .unwrap();
    let specialization = SpecializationKey::new(
        package.reference().package_build_id.clone(),
        ArtifactFunctionKey::parse("fixture::normalization").unwrap(),
        PackageCallableId::new(ROOT_CALLABLE),
        Box::new([]),
        None,
    );
    let location = package_location(package);
    let limits = generous_limits();
    let mut linker = TypeLinker::new(&hydrated, &limits);
    let linked_index = linker
        .intern_pool_type(
            package,
            &specialization,
            artifact_index,
            &BTreeMap::new(),
            location.clone(),
        )
        .unwrap();
    let entries = linker.finish(location).unwrap().types;

    let entry = entries
        .iter()
        .find(|entry| entry.index() == linked_index)
        .unwrap();
    assert_eq!(
        entry.type_ref(),
        &exact_type(package, OWNER_IMPLEMENTATION_PATH)
    );
}

#[test]
fn dense_result_and_parameter_materialization_reject_wrong_nominal_abi_plan_and_fields() {
    let hydrated = Fixture::record_shape().hydrate();
    let package = implementation_package(&hydrated);
    let specialization = SpecializationKey::new(
        package.reference().package_build_id.clone(),
        ArtifactFunctionKey::parse("fixture::root").unwrap(),
        PackageCallableId::new(ROOT_CALLABLE),
        Box::new([]),
        None,
    );
    let location = package_location(package);
    let limits = generous_limits();
    let mut linker = TypeLinker::new(&hydrated, &limits);
    let shape_index = linker
        .intern_pool_shape(
            package,
            &specialization,
            0,
            &BTreeMap::new(),
            location.clone(),
        )
        .unwrap();
    let emitted_shape = linker.shape(shape_index).unwrap().clone();
    let field_type = linker
        .linked_type_ref(emitted_shape.fields()[0].ty())
        .unwrap()
        .clone();
    let exact_field_plan = linker
        .plan_for_concrete_type(&field_type, location.clone())
        .unwrap();
    let shape = LinkedShapeEntry::new(
        emitted_shape.index(),
        emitted_shape.origin().clone(),
        emitted_shape.nominal_type(),
        emitted_shape.plan().clone(),
        None,
        Box::new([
            LinkedShapeField::new("name", emitted_shape.fields()[0].ty(), exact_field_plan)
                .unwrap(),
        ]),
    )
    .unwrap();
    let snapshot_plan = LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::SnapshotRelease,
    };
    linker
        .validate_dense_result_materialization(
            shape.nominal_type(),
            &snapshot_plan,
            &shape,
            location.clone(),
        )
        .expect("the compiler-emitted structural record closes exactly");
    linker
        .validate_dense_parameter_materialization(
            shape.nominal_type(),
            &snapshot_plan,
            &shape,
            location.clone(),
        )
        .expect("the same exact record closes a compiler-owned frame parameter");

    let wrong_nominal = linker
        .validate_dense_parameter_materialization(
            shape.fields()[0].ty(),
            &snapshot_plan,
            &shape,
            location.clone(),
        )
        .expect_err("a field TypeIndex cannot stand in for the result nominal type");
    assert!(matches!(
        wrong_nominal,
        BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            ref detail,
            ..
        } if detail.contains("TypeRef/ABI")
    ));

    let wrong_plan = linker
        .validate_dense_parameter_materialization(
            shape.nominal_type(),
            &LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            },
            &shape,
            location.clone(),
        )
        .expect_err("dense materialization cannot weaken the exact snapshot drop");
    assert!(matches!(
        wrong_plan,
        BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
            ..
        }
    ));

    let wrong_fields = LinkedShapeEntry::new(
        shape.index(),
        shape.origin().clone(),
        shape.nominal_type(),
        shape.plan().clone(),
        None,
        Box::new([LinkedShapeField::new(
            "wrong",
            shape.fields()[0].ty(),
            shape.fields()[0].plan().clone(),
        )
        .unwrap()]),
    )
    .unwrap();
    let wrong_fields = linker
        .validate_dense_parameter_materialization(
            shape.nominal_type(),
            &snapshot_plan,
            &wrong_fields,
            location.clone(),
        )
        .expect_err("artifact field drift cannot replace the exact record descriptor");
    assert!(matches!(
        wrong_fields,
        BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            ref detail,
            ..
        } if detail.contains("field 0")
    ));

    let privileged = LinkedShapeEntry::new(
        shape.index(),
        shape.origin().clone(),
        shape.nominal_type(),
        shape.plan().clone(),
        Some(skiff_artifact_model::PrivilegedAffineCompositeIdentity::HttpClientStreamHandle),
        shape.fields().into(),
    )
    .unwrap();
    assert!(linker
        .validate_dense_parameter_materialization(
            shape.nominal_type(),
            &snapshot_plan,
            &privileged,
            location.clone(),
        )
        .is_err());

    assert!(matches!(
        normalize_type(
            &hydrated,
            package,
            &self_type(OWNER_PUBLIC_PATH, Some("abi:wrong")),
            &location,
        ),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            detail,
            ..
        }) if detail.contains("ABI expectation")
    ));
}

#[test]
fn canonicalizes_recursive_owner_forms_and_interface_identity() {
    let hydrated = Fixture::normalization().hydrate();
    let package = implementation_package(&hydrated);
    let dependency = dependency_package(&hydrated);
    let location = package_location(package);
    let interface_identity = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "fixture".to_string(),
            symbol: "Owner".to_string(),
        },
    };
    let interface_abi_id =
        String::from_utf8(skiff_canonical_json::canonical_json_bytes(&interface_identity).unwrap())
            .unwrap();
    let source = TypeRefIr::Record {
        fields: BTreeMap::from([
            (
                "nominal".to_string(),
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::PublicationType {
                        module_path: "fixture".to_string(),
                        type_index: 0,
                    },
                    arguments: vec![TypeRefIr::Union {
                        items: vec![
                            dependency_type(OWNER_PUBLIC_PATH, None),
                            TypeRefIr::Nullable {
                                inner: Box::new(interface_identity.clone()),
                            },
                        ],
                    }],
                },
            ),
            (
                "interface".to_string(),
                TypeRefIr::AnyInterface {
                    interface: InterfaceInstantiationRef {
                        interface_abi_id,
                        canonical_type_args: vec![self_type(OWNER_PUBLIC_PATH, None)],
                    },
                },
            ),
        ]),
    };

    let normalized = normalize_type(&hydrated, package, &source, &location).unwrap();
    let exact_implementation = exact_type(package, OWNER_IMPLEMENTATION_PATH);
    let exact_dependency_public = exact_type(dependency, OWNER_PUBLIC_PATH);
    let exact_self_public = exact_type(package, OWNER_PUBLIC_PATH);
    let TypeRefIr::Record { fields } = normalized else {
        panic!("normalization must preserve the record shape");
    };
    let TypeRefIr::AppliedNominal { base, arguments } = &fields["nominal"] else {
        panic!("normalization must preserve the applied nominal shape");
    };
    assert_eq!(
        base,
        &nominal_base(exact_implementation.clone()),
        "publication base must become its canonical implementation Type path"
    );
    let TypeRefIr::Union { items } = &arguments[0] else {
        panic!("normalization must preserve recursive union arguments");
    };
    assert_eq!(items[0], exact_dependency_public);
    assert_eq!(
        items[1],
        TypeRefIr::Nullable {
            inner: Box::new(exact_implementation.clone()),
        }
    );
    let TypeRefIr::AnyInterface { interface } = &fields["interface"] else {
        panic!("normalization must preserve AnyInterface");
    };
    let normalized_identity =
        serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).unwrap();
    assert_eq!(normalized_identity, exact_implementation);
    assert_eq!(interface.canonical_type_args, vec![exact_self_public]);
    assert_eq!(
        interface.interface_abi_id.as_bytes(),
        skiff_canonical_json::canonical_json_bytes(&normalized_identity)
            .unwrap()
            .as_slice(),
        "embedded interface identity must be canonically re-encoded"
    );
}

#[test]
fn allows_unpinned_public_dependency_types_and_matching_dual_surfaces() {
    let hydrated = Fixture::unpinned_normalization().hydrate();
    let package = implementation_package(&hydrated);
    let dependency = dependency_package(&hydrated);
    let location = package_location(package);

    assert_eq!(
        normalize_type(
            &hydrated,
            package,
            &dependency_type(OWNER_PUBLIC_PATH, None),
            &location,
        )
        .unwrap(),
        exact_type(dependency, OWNER_PUBLIC_PATH)
    );
    assert_eq!(
        normalize_type(
            &hydrated,
            package,
            &dependency_type(OWNER_IMPLEMENTATION_PATH, None),
            &location,
        )
        .unwrap(),
        exact_type(dependency, OWNER_IMPLEMENTATION_PATH),
        "matching public and implementation rows remain usable as a public type"
    );
}

#[test]
fn allows_exact_build_pinned_private_dependency_types() {
    let hydrated = Fixture::normalization().hydrate();
    let package = implementation_package(&hydrated);
    let dependency = dependency_package(&hydrated);
    let location = package_location(package);

    let expected = exact_type(dependency, PRIVATE_IMPLEMENTATION_PATH);
    assert_eq!(
        normalize_type(
            &hydrated,
            package,
            &dependency_type(PRIVATE_IMPLEMENTATION_PATH, None),
            &location,
        )
        .unwrap(),
        expected
    );
    assert_eq!(
        normalize_type(&hydrated, package, &expected, &location).unwrap(),
        expected,
        "an exact PackageId form may recover only its caller's direct pinned requirement"
    );
}

#[test]
fn rejects_unpinned_private_dependency_types_for_alias_and_package_id() {
    let hydrated = Fixture::unpinned_normalization().hydrate();
    let package = implementation_package(&hydrated);
    let dependency = dependency_package(&hydrated);
    let location = package_location(package);

    for private in [
        dependency_type(PRIVATE_IMPLEMENTATION_PATH, None),
        exact_type(dependency, PRIVATE_IMPLEMENTATION_PATH),
    ] {
        assert!(matches!(
            normalize_type(&hydrated, package, &private, &location),
            Err(BytecodeLinkError::UnsatisfiedObligation {
                obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                detail,
                ..
            }) if detail.contains("no unique direct exact-build authority")
        ));
    }
}

#[test]
fn rejects_wrong_dependency_build_pin_before_private_authority_exists() {
    assert!(matches!(
        Fixture::wrong_pinned_normalization().try_hydrate(),
        Err(DeploymentBytecodeHydrationError::PackageRequirementMismatch { .. })
    ));
}

#[test]
fn rejects_conflicting_dependency_type_surfaces() {
    for conflict in [
        DependencyTypeSurfaceConflict::Descriptor,
        DependencyTypeSurfaceConflict::TypeParameters,
        DependencyTypeSurfaceConflict::InterfaceFlag,
    ] {
        let hydrated = Fixture::conflicting_dependency_normalization(conflict).hydrate();
        let package = implementation_package(&hydrated);
        let location = package_location(package);
        assert!(matches!(
            normalize_type(
                &hydrated,
                package,
                &dependency_type(OWNER_IMPLEMENTATION_PATH, None),
                &location,
            ),
            Err(BytecodeLinkError::UnsatisfiedObligation {
                obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                detail,
                ..
            }) if detail.contains("ambiguous across different public and implementation semantics")
        ));
    }
}

#[test]
fn retains_self_implementation_authority_and_checks_exact_abi() {
    let hydrated = Fixture::normalization().hydrate();
    let package = implementation_package(&hydrated);
    let location = package_location(package);

    assert_eq!(
        normalize_type(
            &hydrated,
            package,
            &self_type(OWNER_IMPLEMENTATION_PATH, None),
            &location,
        )
        .unwrap(),
        exact_type(package, OWNER_IMPLEMENTATION_PATH),
        "self owner may select its exact implementation descriptor"
    );
    assert!(matches!(
        normalize_type(
            &hydrated,
            package,
            &self_type(OWNER_PUBLIC_PATH, Some("abi:wrong")),
            &location,
        ),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            detail,
            ..
        }) if detail.contains("ABI expectation")
    ));
}

#[test]
fn checks_schema_triples_and_rejects_incomplete_owners() {
    let hydrated = Fixture::normalization().hydrate();
    let package = implementation_package(&hydrated);
    let location = package_location(package);
    let schema = fixtures::schema_type();
    assert_eq!(
        normalize_type(&hydrated, package, &schema, &location).unwrap(),
        schema
    );
    let TypeRefIr::PackageSchema {
        package_id,
        package_schema_type_id,
        ..
    } = fixtures::schema_type()
    else {
        unreachable!()
    };
    let mismatched_schema = TypeRefIr::PackageSchema {
        package_id,
        stable_schema_key: "fixture.Wrong".to_string(),
        package_schema_type_id,
    };
    assert!(matches!(
        normalize_type(&hydrated, package, &mismatched_schema, &location),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            detail,
            ..
        }) if detail.contains("descriptor triple")
    ));

    for incomplete in [
        TypeRefIr::LocalType { type_index: 0 },
        TypeRefIr::DbObjectSymbol {
            symbol: ServiceSymbolRef {
                module_path: "fixture".to_string(),
                symbol: "Object".to_string(),
            },
        },
        TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
        TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "value".to_string(),
                ty: self_type(OWNER_IMPLEMENTATION_PATH, None),
            }],
            return_type: Box::new(TypeRefIr::builtin("void")),
        },
    ] {
        assert!(matches!(
            normalize_type(&hydrated, package, &incomplete, &location),
            Err(BytecodeLinkError::UnsatisfiedObligation {
                obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                ..
            })
        ));
    }
}

#[test]
fn rejects_conflicting_self_surfaces() {
    let hydrated = Fixture::conflicting_normalization().hydrate();
    let package = implementation_package(&hydrated);
    let location = package_location(package);
    assert!(matches!(
        normalize_type(
            &hydrated,
            package,
            &self_type(OWNER_IMPLEMENTATION_PATH, None),
            &location,
        ),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
            detail,
            ..
        }) if detail.contains("different public and implementation semantics")
    ));
}

fn implementation_package(hydrated: &HydratedDeploymentBytecode) -> &HydratedBytecodePackage {
    hydrated
        .packages()
        .get(&hydrated.deployment().implementation.package_build_id)
        .unwrap()
}

fn dependency_package(hydrated: &HydratedDeploymentBytecode) -> &HydratedBytecodePackage {
    let mut packages = hydrated
        .packages()
        .values()
        .filter(|package| package.reference().package_id == DEPENDENCY_PACKAGE_ID);
    let dependency = packages.next().unwrap();
    assert!(packages.next().is_none());
    dependency
}

fn package_location(package: &HydratedBytecodePackage) -> BytecodeLinkLocation {
    BytecodeLinkLocation::Package {
        package: Box::new(package.reference().clone()),
    }
}

fn dependency_type(symbol_path: &str, abi_expectation: Option<&str>) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: DEPENDENCY_ALIAS.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: abi_expectation.map(str::to_string),
        },
    }
}

fn self_type(symbol_path: &str, abi_expectation: Option<&str>) -> TypeRefIr {
    package_id_type("example.bytecode-link", symbol_path, abi_expectation)
}

fn package_id_type(
    package_id: &str,
    symbol_path: &str,
    abi_expectation: Option<&str>,
) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: abi_expectation.map(str::to_string),
        },
    }
}

fn exact_type(package: &HydratedBytecodePackage, symbol_path: &str) -> TypeRefIr {
    package_id_type(
        &package.reference().package_id,
        symbol_path,
        Some(package.reference().package_local_abi_identity.as_str()),
    )
}

fn nominal_base(ty: TypeRefIr) -> NominalTypeRefBaseIr {
    let TypeRefIr::PackageSymbol { symbol } = ty else {
        panic!("test fixture exact type must be a PackageSymbol");
    };
    NominalTypeRefBaseIr::PackageSymbol { symbol }
}
