mod authority_chains;
mod preflight;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    InterfaceInstantiationRef, LiteralIr, NominalTypeRefBaseIr, PackageRefIr, PackageSchemaTypeId,
    PackageSymbolRef, ServiceSymbolRef, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, LinkedArtifactPoolOrigin, LinkedTypeEntry, TypeIndex,
};

use crate::{
    concrete_values::{normalize_owner_for_test, prove_types_and_plans},
    VerificationError, VerificationObligation,
};

use super::super::fixtures::{
    candidate_for, candidate_for_concrete_types, generous_limits, owner_authority_fixture,
    OwnerAuthorityFixture, OwnerRequirementMode, OwnerTypeSurface, OWNER_CALLER_PACKAGE_ID,
    OWNER_DEPENDENCY_ALIAS, OWNER_SCHEMA_KEY, OWNER_SELF_TYPE_PATH, OWNER_TARGET_PACKAGE_ID,
    OWNER_TYPE_PATH,
};

#[test]
fn raw_publication_owner_is_independently_normalized_before_candidate_comparison() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let caller = fixture
        .hydrated
        .packages()
        .get(&fixture.caller_build_id)
        .unwrap();
    let expected = exact_package_type(
        caller.reference().package_id.as_str(),
        caller.reference().package_local_abi_identity.as_str(),
        OWNER_SELF_TYPE_PATH,
    );
    let linked = LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(
            fixture.caller_build_id.clone(),
            ArtifactTypeIndex::new(0),
            None,
        )
        .unwrap(),
        expected.clone(),
        None,
    );
    let candidate = candidate_for_concrete_types(&fixture.hydrated, vec![linked], Vec::new())
        .expect("owner-complete candidate type is locally valid");

    let facts = prove_types_and_plans(&fixture.hydrated, &candidate, &generous_limits())
        .expect("P2 independently reconstructs the publication owner");
    assert_eq!(
        facts
            .type_fact(TypeIndex::new(0))
            .unwrap()
            .normalized_type(),
        &expected
    );
}

#[test]
fn pinned_private_dependency_passes_complete_owner_lifecycle_and_class_chain() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let expected = exact_target_type(&fixture, OWNER_TYPE_PATH);
    let linked = LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(
            fixture.caller_build_id.clone(),
            ArtifactTypeIndex::new(1),
            None,
        )
        .unwrap(),
        expected.clone(),
        None,
    );
    let candidate = candidate_for_concrete_types(&fixture.hydrated, vec![linked], Vec::new())
        .expect("exact private dependency candidate is locally valid");

    let facts = prove_types_and_plans(&fixture.hydrated, &candidate, &generous_limits())
        .expect("exact-pinned private type passes the complete P2 chain");
    assert_eq!(
        facts
            .type_fact(TypeIndex::new(0))
            .unwrap()
            .normalized_type(),
        &expected
    );
}

#[test]
fn pinned_private_origin_rejects_wrong_candidate_owner_in_p2() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let linked = LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(
            fixture.caller_build_id.clone(),
            ArtifactTypeIndex::new(1),
            None,
        )
        .unwrap(),
        TypeRefIr::builtin("string"),
        None,
    );
    let candidate = candidate_for_concrete_types(&fixture.hydrated, vec![linked], Vec::new())
        .expect("wrong candidate owner remains locally constructible");
    let error = prove_types_and_plans(&fixture.hydrated, &candidate, &generous_limits())
        .expect_err("P2 must compare against its independent owner normalization");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation { detail, .. }
            if detail.contains("normalized admitted raw type")
    ));
}

#[test]
fn self_service_symbol_normalizes_to_exact_package_owner() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let normalized = normalize_owner(
        &fixture,
        &TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "model".to_string(),
                symbol: "SelfValue".to_string(),
            },
        },
    )
    .unwrap();
    let caller = fixture
        .hydrated
        .packages()
        .get(&fixture.caller_build_id)
        .unwrap();
    assert_eq!(
        normalized,
        exact_package_type(
            OWNER_CALLER_PACKAGE_ID,
            caller.reference().package_local_abi_identity.as_str(),
            OWNER_SELF_TYPE_PATH,
        )
    );
}

#[test]
fn external_public_type_allows_unpinned_alias_and_package_id() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Unpinned, OwnerTypeSurface::Public);
    let expected = exact_target_type(&fixture, OWNER_TYPE_PATH);
    for raw in [
        dependency_type(OWNER_DEPENDENCY_ALIAS, OWNER_TYPE_PATH, None),
        package_id_type(OWNER_TARGET_PACKAGE_ID, OWNER_TYPE_PATH, None),
    ] {
        assert_eq!(normalize_owner(&fixture, &raw).unwrap(), expected);
    }
}

#[test]
fn exact_pinned_private_type_allows_alias_and_normalized_package_id() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let expected = exact_target_type(&fixture, OWNER_TYPE_PATH);
    for raw in [
        dependency_type(OWNER_DEPENDENCY_ALIAS, OWNER_TYPE_PATH, None),
        package_id_type(OWNER_TARGET_PACKAGE_ID, OWNER_TYPE_PATH, None),
    ] {
        assert_eq!(normalize_owner(&fixture, &raw).unwrap(), expected);
    }
}

#[test]
fn private_type_rejects_missing_or_ambiguous_exact_pin() {
    let unpinned =
        owner_authority_fixture(OwnerRequirementMode::Unpinned, OwnerTypeSurface::Private);
    assert_owner_violation(
        normalize_owner(
            &unpinned,
            &dependency_type(OWNER_DEPENDENCY_ALIAS, OWNER_TYPE_PATH, None),
        ),
        "exact-build authority",
    );

    let ambiguous = owner_authority_fixture(
        OwnerRequirementMode::DuplicateExact,
        OwnerTypeSurface::Private,
    );
    assert_owner_violation(
        normalize_owner(
            &ambiguous,
            &package_id_type(OWNER_TARGET_PACKAGE_ID, OWNER_TYPE_PATH, None),
        ),
        "exact-build authority",
    );
}

#[test]
fn package_type_rejects_wrong_abi_and_dual_surface_semantic_drift() {
    let exact = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    assert_owner_violation(
        normalize_owner(
            &exact,
            &package_id_type(OWNER_TARGET_PACKAGE_ID, OWNER_TYPE_PATH, Some("abi:wrong")),
        ),
        "ABI expectation",
    );

    let drift = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Conflicting);
    assert_owner_violation(
        normalize_owner(
            &drift,
            &dependency_type(OWNER_DEPENDENCY_ALIAS, OWNER_TYPE_PATH, None),
        ),
        "different public and implementation semantics",
    );
}

#[test]
fn package_schema_requires_its_exact_owner_triple() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Public);
    let exact = TypeRefIr::PackageSchema {
        package_id: OWNER_TARGET_PACKAGE_ID.to_string(),
        stable_schema_key: OWNER_SCHEMA_KEY.to_string(),
        package_schema_type_id: fixture.schema_type_id.clone(),
    };
    assert_eq!(normalize_owner(&fixture, &exact).unwrap(), exact);

    let wrong_key = TypeRefIr::PackageSchema {
        package_id: OWNER_TARGET_PACKAGE_ID.to_string(),
        stable_schema_key: "model.Wrong".to_string(),
        package_schema_type_id: fixture.schema_type_id.clone(),
    };
    assert_owner_violation(normalize_owner(&fixture, &wrong_key), "descriptor triple");
    let wrong_id = TypeRefIr::PackageSchema {
        package_id: OWNER_TARGET_PACKAGE_ID.to_string(),
        stable_schema_key: OWNER_SCHEMA_KEY.to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("schema:wrong"),
    };
    assert_owner_violation(normalize_owner(&fixture, &wrong_id), "no exact descriptor");
}

#[test]
fn recursive_owner_normalization_covers_applied_record_union_nullable_and_literal() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let raw = TypeRefIr::Record {
        fields: BTreeMap::from([
            (
                "nominal".to_string(),
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::PackageSymbol {
                        symbol: dependency_symbol(OWNER_DEPENDENCY_ALIAS, OWNER_TYPE_PATH, None),
                    },
                    arguments: vec![TypeRefIr::builtin("string")],
                },
            ),
            (
                "choice".to_string(),
                TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::Union {
                        items: vec![
                            TypeRefIr::builtin("bool"),
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "ready".to_string(),
                                },
                            },
                        ],
                    }),
                },
            ),
        ]),
    };
    let normalized = normalize_owner(&fixture, &raw).unwrap();
    let TypeRefIr::Record { fields } = normalized else {
        panic!("record shape must be preserved");
    };
    let TypeRefIr::AppliedNominal { base, .. } = &fields["nominal"] else {
        panic!("applied nominal shape must be preserved");
    };
    assert!(matches!(
        base,
        NominalTypeRefBaseIr::PackageSymbol { symbol }
            if matches!(&symbol.package, PackageRefIr::PackageId { .. })
                && symbol.abi_expectation.as_deref()
                    == Some(fixture.target.package_local_abi_identity.as_str())
    ));
}

#[test]
fn ownerless_and_currently_unauthorized_type_forms_fail_closed() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    for (raw, keyword) in [
        (TypeRefIr::LocalType { type_index: 7 }, "ownerless local"),
        (
            TypeRefIr::DbObjectSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "model".to_string(),
                    symbol: "Db".to_string(),
                },
            },
            "DB object",
        ),
        (
            TypeRefIr::Function {
                params: Vec::new(),
                return_type: Box::new(TypeRefIr::builtin("string")),
            },
            "function type",
        ),
        (
            TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
            "UnknownTypeParameter",
        ),
    ] {
        assert_owner_violation(normalize_owner(&fixture, &raw), keyword);
    }
}

#[test]
fn any_interface_is_explicitly_proof_unavailable_without_identity_guessing() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Private);
    let result = normalize_owner(
        &fixture,
        &TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: "{}".to_string(),
                canonical_type_args: Vec::new(),
            },
        },
    );
    assert!(matches!(
        result,
        Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::InterfaceSignature,
            ..
        })
    ));
}

fn normalize_owner(
    fixture: &OwnerAuthorityFixture,
    raw: &TypeRefIr,
) -> Result<TypeRefIr, VerificationError> {
    let candidate = candidate_for(&fixture.hydrated, None);
    normalize_owner_for_test(
        &fixture.hydrated,
        &candidate,
        &fixture.caller_build_id,
        raw,
        &generous_limits(),
    )
}

fn exact_target_type(fixture: &OwnerAuthorityFixture, symbol_path: &str) -> TypeRefIr {
    exact_package_type(
        &fixture.target.package_id,
        fixture.target.package_local_abi_identity.as_str(),
        symbol_path,
    )
}

fn exact_package_type(package_id: &str, abi: &str, symbol_path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: Some(abi.to_string()),
        },
    }
}

fn dependency_type(alias: &str, symbol_path: &str, abi: Option<&str>) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: dependency_symbol(alias, symbol_path, abi),
    }
}

fn dependency_symbol(alias: &str, symbol_path: &str, abi: Option<&str>) -> PackageSymbolRef {
    PackageSymbolRef {
        package: PackageRefIr::Dependency {
            dependency_ref: alias.to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: abi.map(str::to_string),
    }
}

fn package_id_type(package_id: &str, symbol_path: &str, abi: Option<&str>) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package_id.to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: abi.map(str::to_string),
        },
    }
}

fn assert_owner_violation(result: Result<TypeRefIr, VerificationError>, keyword: &str) {
    assert!(matches!(
        result,
        Err(VerificationError::SemanticViolation {
            obligation: VerificationObligation::ConcreteTypeAndShape,
            detail,
            ..
        }) if detail.contains(keyword)
    ));
}
