use skiff_artifact_model::{PackageBuildId, ShapeDeclaration, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    ArtifactShapeIndex, ArtifactTypeIndex, CandidateTable, LinkedArtifactPoolOrigin,
    LinkedBytecodeCandidate, LinkedBytecodeCandidateError, LinkedContainerLayout,
    LinkedContainerPosition, LinkedShapeEntry, LinkedTypeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan, ShapeIndex, TypeIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{
    verify, VerificationError, VerificationLimit, VerificationLocation, VerificationObligation,
};

use super::fixtures::{
    candidate_for_concrete_types, exact_hydration_with_types,
    exact_hydration_with_types_and_shapes, generous_limits,
};

#[test]
fn package_global_string_and_array_with_exact_plans_reach_control_flow() {
    let ArrayCandidate {
        hydrated,
        candidate,
    } = array_candidate(snapshot_release_plan(), false);
    let candidate = candidate.expect("exact concrete type candidate passes local validation");
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExactTargetAndCallPlan,
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
