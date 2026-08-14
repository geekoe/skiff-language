use skiff_artifact_model::{bytecode::limits, InterfaceInstantiationRef, TypeRefIr};

use crate::{
    CandidateLocation, CandidateTable, CandidateTypeStructureLimit, InterfaceTableIndex,
    LinkedBytecodeCandidate, LinkedBytecodeCandidateError, LinkedInterfaceInstantiation,
    LinkedInterfaceRequirementTable, LinkedInterfaceTable, LinkedInterfaceTableKind,
    LinkedTypeEntry, TypeIndex,
};

use super::fixtures::{minimal_parts, snapshot_plan, type_origin};

fn nullable_type(depth: u64) -> TypeRefIr {
    let mut ty = TypeRefIr::LocalType { type_index: 0 };
    for _ in 1..depth {
        ty = TypeRefIr::Nullable {
            inner: Box::new(ty),
        };
    }
    ty
}

fn parts_with_type(ty: TypeRefIr) -> crate::LinkedBytecodeCandidateParts {
    let mut parts = minimal_parts(Vec::new());
    parts.types = vec![LinkedTypeEntry::new(
        TypeIndex::new(0),
        type_origin(0, None),
        ty,
        snapshot_plan(),
        None,
    )];
    parts
}

fn requirement_interface_with_argument(argument: TypeRefIr) -> LinkedInterfaceTable {
    LinkedInterfaceTable::new(
        InterfaceTableIndex::new(0),
        LinkedInterfaceInstantiation::new(
            InterfaceInstantiationRef {
                interface_abi_id: "interface:example".to_string(),
                canonical_type_args: vec![argument],
            },
            Box::new([TypeIndex::new(0)]),
        )
        .expect("artifact and linked type-argument counts agree"),
        LinkedInterfaceTableKind::Requirement(
            LinkedInterfaceRequirementTable::new(Box::new([]))
                .expect("an empty method table is canonical"),
        ),
    )
}

#[test]
fn nullable_type_at_depth_limit_is_accepted() {
    LinkedBytecodeCandidate::try_from_parts(parts_with_type(nullable_type(
        limits::MAX_NESTING_DEPTH,
    )))
    .expect("a type at the artifact nesting authority boundary is valid");
}

#[test]
fn nullable_type_above_depth_limit_is_rejected_before_local_validation() {
    let actual = limits::MAX_NESTING_DEPTH + 1;
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(parts_with_type(nullable_type(actual))),
        Err(LinkedBytecodeCandidateError::TypeStructureLimitExceeded {
            location: CandidateLocation::TableRow {
                table: CandidateTable::Types,
                row: 0,
            },
            limit: CandidateTypeStructureLimit::TypeDepth,
            max: limits::MAX_NESTING_DEPTH,
            actual: error_actual,
        }) if error_actual == actual
    ));
}

#[test]
fn interface_canonical_type_argument_is_preflighted_before_derived_equality() {
    let actual = limits::MAX_NESTING_DEPTH + 1;
    let mut parts = minimal_parts(Vec::new());
    parts.interface_tables = vec![requirement_interface_with_argument(nullable_type(actual))];
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(parts),
        Err(LinkedBytecodeCandidateError::TypeStructureLimitExceeded {
            location: CandidateLocation::TableRow {
                table: CandidateTable::InterfaceTables,
                row: 0,
            },
            limit: CandidateTypeStructureLimit::TypeDepth,
            max: limits::MAX_NESTING_DEPTH,
            actual: error_actual,
        }) if error_actual == actual
    ));
}
