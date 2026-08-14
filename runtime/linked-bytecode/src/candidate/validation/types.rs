use std::collections::BTreeSet;

use skiff_artifact_model::PackageBuildId;

use crate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts, LinkedContainerLayout, LinkedContainerPositionKind,
    LinkedTypeEntry,
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
    validate_plan(row.plan(), location, parts)?;
    if let Some(layout) = row.container_layout() {
        for (kind, position) in layout.position_entries() {
            check_index(
                location,
                CandidateReferenceKind::Type,
                position.ty().get(),
                parts.types.len(),
            )?;
            validate_plan(position.plan(), location, parts)?;
            let target = &parts.types[position.ty().get() as usize];
            if position.plan() != target.plan() {
                return Err(
                    LinkedBytecodeCandidateError::ContainerPositionPlanMismatch {
                        type_index: row.index(),
                        position: kind,
                    },
                );
            }
        }
        validate_container_closure(row, layout)?;
    }
    Ok(())
}

fn validate_container_closure(
    row: &LinkedTypeEntry,
    layout: &LinkedContainerLayout,
) -> Result<(), LinkedBytecodeCandidateError> {
    for (kind, position) in layout.position_entries() {
        if kind == LinkedContainerPositionKind::JsonRecursiveValue && position.ty() != row.index() {
            return Err(
                LinkedBytecodeCandidateError::ContainerPositionTypeMismatch {
                    type_index: row.index(),
                    position: kind,
                    actual: position.ty(),
                },
            );
        }
    }
    Ok(())
}
