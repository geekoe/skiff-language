use std::collections::BTreeSet;

use skiff_artifact_model::PackageBuildId;

use crate::{
    CandidateLocation, CandidateTable, LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts,
    LinkedContainerLayout, LinkedContainerPositionKind, LinkedTypeEntry,
};

use super::{
    plans::{validate_plan, validate_type_plan},
    validate_origin,
};

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
        for position in layout.positions() {
            validate_type_plan(position.ty(), position.plan(), location, parts)?;
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
