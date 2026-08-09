use std::collections::BTreeSet;

use skiff_artifact_model::{PackageBuildId, TypeRefIr};

use crate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts, LinkedContainerLayout, LinkedContainerLayoutKind,
    LinkedContainerPosition, LinkedContainerPositionKind, LinkedTypeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan,
};

use super::{check_index, plans::validate_plan, validate_origin};

pub(super) fn validate_type(
    row: &LinkedTypeEntry,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::Types,
        row: row.index().get(),
    };
    validate_origin(row.origin(), location, parts, package_ids)?;
    let expected = expected_container_layout(row)?;
    match (expected, row.container_layout()) {
        (Some(expected), None) => {
            return Err(LinkedBytecodeCandidateError::MissingContainerLayout {
                type_index: row.index(),
                expected,
            });
        }
        (None, Some(layout)) => {
            return Err(LinkedBytecodeCandidateError::UnexpectedContainerLayout {
                type_index: row.index(),
                actual: layout.kind(),
            });
        }
        (Some(expected), Some(layout)) if expected != layout.kind() => {
            return Err(LinkedBytecodeCandidateError::ContainerLayoutKindMismatch {
                type_index: row.index(),
                expected,
                actual: layout.kind(),
            });
        }
        (None, None) | (Some(_), Some(_)) => {}
    }
    if let Some(layout) = row.container_layout() {
        for position in layout.positions() {
            check_index(
                location,
                CandidateReferenceKind::Type,
                position.ty().get(),
                parts.types.len(),
            )?;
            validate_plan(position.plan(), location, parts)?;
        }
        validate_container_position_correlations(row, layout, parts)?;
    }
    Ok(())
}

fn validate_container_position_correlations(
    row: &LinkedTypeEntry,
    layout: &LinkedContainerLayout,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let TypeRefIr::Builtin { args, .. } = row.type_ref() else {
        return Ok(());
    };
    for (kind, position) in layout.position_entries() {
        let matches = match kind {
            LinkedContainerPositionKind::ArrayElement => {
                position_matches_type_ref(position, &args[0], parts)
            }
            LinkedContainerPositionKind::MapKey => {
                position_matches_type_ref(position, &args[0], parts)
            }
            LinkedContainerPositionKind::MapValue => {
                position_matches_type_ref(position, &args[1], parts)
            }
            LinkedContainerPositionKind::JsonRecursiveValue => position.ty() == row.index(),
            LinkedContainerPositionKind::JsonObjectKey => {
                position_matches_builtin(position, "string", parts)
            }
            LinkedContainerPositionKind::JsonObjectValue => {
                position_matches_builtin(position, "Json", parts)
            }
        };
        if !matches {
            return Err(
                LinkedBytecodeCandidateError::ContainerPositionTypeMismatch {
                    type_index: row.index(),
                    position: kind,
                    actual: position.ty(),
                },
            );
        }
        if matches!(
            kind,
            LinkedContainerPositionKind::JsonRecursiveValue
                | LinkedContainerPositionKind::JsonObjectKey
                | LinkedContainerPositionKind::JsonObjectValue
        ) && !is_snapshot_release(position.plan())
        {
            return Err(
                LinkedBytecodeCandidateError::ContainerPositionPlanMismatch {
                    type_index: row.index(),
                    position: kind,
                },
            );
        }
    }
    Ok(())
}

fn position_matches_type_ref(
    position: &LinkedContainerPosition,
    expected: &TypeRefIr,
    parts: &LinkedBytecodeCandidateParts,
) -> bool {
    parts
        .types
        .get(position.ty().get() as usize)
        .is_some_and(|row| row.type_ref() == expected)
}

fn position_matches_builtin(
    position: &LinkedContainerPosition,
    expected_name: &str,
    parts: &LinkedBytecodeCandidateParts,
) -> bool {
    matches!(
        parts
            .types
            .get(position.ty().get() as usize)
            .map(|row| row.type_ref()),
        Some(TypeRefIr::Builtin { name, args })
            if name == expected_name && args.is_empty()
    )
}

fn is_snapshot_release(plan: &LinkedValueTransferPlan) -> bool {
    matches!(
        plan,
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease
        }
    )
}

fn expected_container_layout(
    row: &LinkedTypeEntry,
) -> Result<Option<LinkedContainerLayoutKind>, LinkedBytecodeCandidateError> {
    let TypeRefIr::Builtin { name, args } = row.type_ref() else {
        return Ok(None);
    };
    let (kind, expected_arity) = match name.as_str() {
        "Array" => (LinkedContainerLayoutKind::Array, 1),
        "Map" => (LinkedContainerLayoutKind::Map, 2),
        "Json" => (LinkedContainerLayoutKind::Json, 0),
        "JsonObject" => (LinkedContainerLayoutKind::JsonObject, 0),
        _ => return Ok(None),
    };
    if args.len() != expected_arity {
        return Err(
            LinkedBytecodeCandidateError::ContainerBuiltinArityMismatch {
                type_index: row.index(),
                kind,
                expected: expected_arity,
                actual: args.len(),
            },
        );
    }
    Ok(Some(kind))
}
