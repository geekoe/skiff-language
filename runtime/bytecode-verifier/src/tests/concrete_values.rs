mod owner_normalization;

use skiff_artifact_model::{
    NativeValueDropPlan, NativeValueEmbedding, NativeValueLifecycleConcrete,
    NativeValueLifecycleResolution, PackageBuildId, ShapeDeclaration, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    ArtifactShapeIndex, ArtifactTypeIndex, CandidateTable, LinkedArtifactPoolOrigin,
    LinkedBytecodeCandidate, LinkedBytecodeCandidateError, LinkedContainerLayout,
    LinkedContainerPosition, LinkedShapeEntry, LinkedTypeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan, ShapeIndex, TypeIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{
    concrete_values::{prove_types_and_plans, ImplicitBuiltin},
    verify, VerificationError, VerificationLimit, VerificationLocation, VerificationObligation,
};

use super::fixtures::{
    candidate_for_concrete_types, exact_hydration_with_types,
    exact_hydration_with_types_and_shapes, generous_limits,
};

#[test]
fn package_global_string_and_array_with_exact_plans_reach_effect_gate() {
    let ArrayCandidate {
        hydrated,
        candidate,
    } = array_candidate(snapshot_release_plan(), false);
    let candidate = candidate.expect("exact concrete type candidate passes local validation");
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Image,
        }
    );
}

#[test]
fn array_element_with_same_kind_but_trivial_drop_is_rejected() {
    let wrong = LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::Trivial,
    };
    let ArrayCandidate {
        hydrated,
        candidate,
    } = array_candidate(wrong, false);
    let candidate = candidate.expect("same-kind drop corruption passes local validation");
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_types_violation(
        error,
        VerificationObligation::ValueTransferAndDrop,
        1,
        &["Trivial", "SnapshotRelease"],
    );
}

#[test]
fn residual_type_parameter_is_rejected_as_nonconcrete() {
    let residual = TypeRefIr::TypeParam {
        name: "T".to_string(),
    };
    let (hydrated, candidate) = scalar_candidate(residual);
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_types_violation(
        error,
        VerificationObligation::ConcreteTypeAndShape,
        0,
        &["UnknownTypeParameter", "T"],
    );
}

#[test]
fn exact_raw_origin_with_wrong_normalized_body_is_rejected_by_p2() {
    let raw = TypeRefIr::builtin("string");
    let hydrated = exact_hydration_with_types(vec![raw]);
    let linked = type_entry(&hydrated, 0, TypeRefIr::builtin("bytes"), None);
    let candidate = candidate_for_concrete_types(&hydrated, vec![linked], Vec::new())
        .expect("the candidate-local table accepts an independently proved type body");
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_types_violation(
        error,
        VerificationObligation::ConcreteTypeAndShape,
        0,
        &["normalized admitted raw type"],
    );
}

#[test]
fn concrete_type_node_budget_is_enforced_for_a_nonempty_table() {
    let (hydrated, candidate) = scalar_candidate(string_type());
    let mut limits = generous_limits();
    limits.max_value_lifecycle_nodes = 2;
    let error = verify(hydrated, candidate, &limits).unwrap_err();

    assert_types_limit(error, VerificationLimit::ValueLifecycleNodes, 0);
}

#[test]
fn concrete_type_canonical_byte_budget_is_enforced_for_a_nonempty_table() {
    let (hydrated, candidate) = scalar_candidate(string_type());
    let mut limits = generous_limits();
    limits.max_value_lifecycle_canonical_bytes = 1;
    let error = verify(hydrated, candidate, &limits).unwrap_err();

    assert_types_limit(error, VerificationLimit::ValueLifecycleCanonicalBytes, 0);
}

#[test]
fn concrete_type_depth_budget_is_enforced_for_a_nonempty_table() {
    let nested = TypeRefIr::Nullable {
        inner: Box::new(string_type()),
    };
    let (hydrated, candidate) = scalar_candidate(nested);
    let mut limits = generous_limits();
    limits.max_type_nesting_depth = 1;
    let error = verify(hydrated, candidate, &limits).unwrap_err();

    assert_types_limit(error, VerificationLimit::TypeNestingDepth, 0);
}

#[test]
fn locally_valid_recursive_shape_drop_is_rejected_by_p2() {
    let recursive = LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::RecursiveShape {
            shape: ShapeIndex::new(0),
        },
    };
    let ArrayCandidate {
        hydrated,
        candidate,
    } = array_candidate(recursive, true);
    let candidate = candidate.expect("bounded recursive-shape plan passes local validation");
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_types_violation(
        error,
        VerificationObligation::ValueTransferAndDrop,
        1,
        &["recursive-shape"],
    );
}

#[test]
fn duplicate_origins_share_one_class_and_merge_to_the_minimum_coordinate() {
    let facts = concrete_facts(vec![TypeRefIr::builtin("bool"), TypeRefIr::builtin("bool")]);

    assert_eq!(
        facts.type_fact(TypeIndex::new(1)).unwrap().coordinate(),
        TypeIndex::new(1)
    );
    assert_eq!(
        facts.type_class(TypeIndex::new(0)),
        facts.type_class(TypeIndex::new(1))
    );
    assert_eq!(
        facts.semantically_equal(TypeIndex::new(0), TypeIndex::new(1)),
        Some(true)
    );
    assert_eq!(
        facts
            .merge_coordinate(TypeIndex::new(1), TypeIndex::new(0))
            .unwrap(),
        TypeIndex::new(0)
    );
}

#[test]
fn equal_lifecycle_plans_do_not_merge_different_normalized_types() {
    let facts = concrete_facts(vec![
        TypeRefIr::builtin("string"),
        TypeRefIr::builtin("bytes"),
    ]);
    let string = facts.type_fact(TypeIndex::new(0)).unwrap();
    let json = facts.type_fact(TypeIndex::new(1)).unwrap();

    assert_eq!(string.lifecycle(), json.lifecycle());
    assert_ne!(
        facts.type_class(TypeIndex::new(0)),
        facts.type_class(TypeIndex::new(1))
    );
    assert_eq!(
        facts.semantically_equal(TypeIndex::new(0), TypeIndex::new(1)),
        Some(false)
    );
    assert!(facts
        .merge_coordinate(TypeIndex::new(0), TypeIndex::new(1))
        .is_err());
}

#[test]
fn duplicate_implicit_builtins_choose_their_class_minimum_deterministically() {
    let facts = concrete_facts(vec![
        TypeRefIr::builtin("number"),
        TypeRefIr::builtin("bool"),
        TypeRefIr::builtin("bool"),
        TypeRefIr::builtin("integer"),
        TypeRefIr::builtin("number"),
    ]);

    assert_eq!(
        facts.implicit_representative(ImplicitBuiltin::Bool),
        Some(TypeIndex::new(1))
    );
    assert_eq!(
        facts.implicit_representative(ImplicitBuiltin::Number),
        Some(TypeIndex::new(0))
    );
    assert_eq!(
        facts.implicit_representative(ImplicitBuiltin::Integer),
        Some(TypeIndex::new(3))
    );
}

#[test]
fn complete_lifecycle_resolution_including_embedding_partitions_classes() {
    let facts = crate::concrete_values::ConcreteValueFacts::from_classified_types_for_test(vec![
        (
            TypeRefIr::builtin("string"),
            snapshot_resolution(NativeValueEmbedding::Ordinary),
        ),
        (
            TypeRefIr::builtin("string"),
            snapshot_resolution(NativeValueEmbedding::Privileged),
        ),
    ])
    .unwrap();

    assert_eq!(
        facts.semantically_equal(TypeIndex::new(0), TypeIndex::new(1)),
        Some(false)
    );
}

#[test]
fn ambiguous_implicit_builtin_lifecycle_classes_fail_closed() {
    let error = crate::concrete_values::ConcreteValueFacts::from_classified_types_for_test(vec![
        (
            TypeRefIr::builtin("bool"),
            snapshot_resolution(NativeValueEmbedding::Ordinary),
        ),
        (
            TypeRefIr::builtin("bool"),
            snapshot_resolution(NativeValueEmbedding::Privileged),
        ),
    ])
    .unwrap_err();

    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ConcreteTypeAndShape,
            location: VerificationLocation::Table {
                table: CandidateTable::Types,
                row: 1,
            },
            detail,
        } if detail.contains("ambiguous lifecycle classes")
    ));
}

#[test]
fn class_key_canonical_bytes_share_the_lifecycle_budget_ceiling() {
    let error =
        crate::concrete_values::ConcreteValueFacts::from_classified_types_with_budget_for_test(
            vec![(
                TypeRefIr::builtin("string"),
                snapshot_resolution(NativeValueEmbedding::Ordinary),
            )],
            5,
            5,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        VerificationError::LimitExceeded {
            limit: VerificationLimit::ValueLifecycleCanonicalBytes,
            actual,
            max: 5,
            location: VerificationLocation::Table {
                table: CandidateTable::Types,
                row: 0,
            },
        } if actual > 5
    ));
}

fn snapshot_resolution(embedding: NativeValueEmbedding) -> NativeValueLifecycleResolution {
    NativeValueLifecycleResolution {
        lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
            drop: NativeValueDropPlan::SnapshotRelease,
        },
        embedding,
    }
}

fn concrete_facts(types: Vec<TypeRefIr>) -> crate::concrete_values::ConcreteValueFacts {
    let hydrated = exact_hydration_with_types(types.clone());
    let entries = types
        .into_iter()
        .enumerate()
        .map(|(index, ty)| type_entry(&hydrated, u32::try_from(index).unwrap(), ty, None))
        .collect();
    let candidate = candidate_for_concrete_types(&hydrated, entries, Vec::new())
        .expect("duplicate concrete types pass candidate-local validation");
    prove_types_and_plans(&hydrated, &candidate, &generous_limits())
        .expect("concrete type facts and classes are independently proved")
}

fn scalar_candidate(ty: TypeRefIr) -> (HydratedDeploymentBytecode, LinkedBytecodeCandidate) {
    let hydrated = exact_hydration_with_types(vec![ty.clone()]);
    let types = vec![type_entry(&hydrated, 0, ty, None)];
    let candidate = candidate_for_concrete_types(&hydrated, types, Vec::new())
        .expect("package-global scalar candidate passes local validation");
    (hydrated, candidate)
}

fn array_candidate(element_plan: LinkedValueTransferPlan, include_shape: bool) -> ArrayCandidate {
    let string = string_type();
    let array = array_type(string.clone());
    let source_shapes = if include_shape {
        vec![ShapeDeclaration {
            type_ref: 0,
            fields: Vec::new(),
        }]
    } else {
        Vec::new()
    };
    let hydrated =
        exact_hydration_with_types_and_shapes(vec![string.clone(), array.clone()], source_shapes);
    let types = vec![
        type_entry(&hydrated, 0, string, None),
        type_entry(
            &hydrated,
            1,
            array,
            Some(LinkedContainerLayout::array(LinkedContainerPosition::new(
                TypeIndex::new(0),
                element_plan,
            ))),
        ),
    ];
    let shapes = if include_shape {
        vec![LinkedShapeEntry::new(
            ShapeIndex::new(0),
            LinkedArtifactPoolOrigin::new(
                exact_package_build_id(&hydrated),
                ArtifactShapeIndex::new(0),
                None,
            )
            .unwrap(),
            TypeIndex::new(0),
            Vec::new().into_boxed_slice(),
        )
        .unwrap()]
    } else {
        Vec::new()
    };
    let candidate = candidate_for_concrete_types(&hydrated, types, shapes);
    ArrayCandidate {
        hydrated,
        candidate,
    }
}

struct ArrayCandidate {
    hydrated: HydratedDeploymentBytecode,
    candidate: Result<LinkedBytecodeCandidate, LinkedBytecodeCandidateError>,
}

fn type_entry(
    hydrated: &HydratedDeploymentBytecode,
    index: u32,
    ty: TypeRefIr,
    layout: Option<LinkedContainerLayout>,
) -> LinkedTypeEntry {
    LinkedTypeEntry::new(
        TypeIndex::new(index),
        LinkedArtifactPoolOrigin::new(
            exact_package_build_id(hydrated),
            ArtifactTypeIndex::new(index),
            None,
        )
        .unwrap(),
        ty,
        layout,
    )
}

fn exact_package_build_id(hydrated: &HydratedDeploymentBytecode) -> PackageBuildId {
    assert_eq!(hydrated.packages().len(), 1);
    hydrated.packages().keys().next().unwrap().clone()
}

fn string_type() -> TypeRefIr {
    TypeRefIr::builtin("string")
}

fn array_type(element: TypeRefIr) -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![element],
    }
}

fn snapshot_release_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::SnapshotRelease,
    }
}

fn assert_types_violation(
    error: VerificationError,
    expected_obligation: VerificationObligation,
    expected_row: u32,
    keywords: &[&str],
) {
    let VerificationError::SemanticViolation {
        obligation,
        location,
        detail,
    } = error
    else {
        panic!("expected a typed P2 semantic violation, got {error:?}");
    };
    assert_eq!(obligation, expected_obligation);
    assert_eq!(
        location,
        VerificationLocation::Table {
            table: CandidateTable::Types,
            row: expected_row,
        }
    );
    for keyword in keywords {
        assert!(
            detail.contains(keyword),
            "P2 diagnostic did not contain {keyword:?}: {detail}"
        );
    }
}

fn assert_types_limit(error: VerificationError, expected: VerificationLimit, row: u32) {
    assert!(matches!(
        error,
        VerificationError::LimitExceeded {
            limit,
            actual,
            max,
            location: VerificationLocation::Table {
                table: CandidateTable::Types,
                row: actual_row,
            },
        } if limit == expected && actual > max && actual_row == row
    ));
}
