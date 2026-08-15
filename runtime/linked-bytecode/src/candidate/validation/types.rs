use std::collections::BTreeSet;

use skiff_artifact_model::{PackageBuildId, PackageRefIr, TypeRefIr};

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
    if let Some(carrier) = row.representation_carrier() {
        validate_representation_carrier(row, carrier, location, parts)?;
    }
    if let Some(layout) = row.container_layout() {
        for position in layout.positions() {
            validate_type_plan(position.ty(), position.plan(), location, parts)?;
        }
        validate_container_closure(row, layout)?;
    }
    Ok(())
}

fn validate_representation_carrier(
    row: &LinkedTypeEntry,
    carrier: &crate::LinkedRepresentationCarrier,
    location: crate::CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    if !matches!(
        row.type_ref(),
        TypeRefIr::PackageSymbol { symbol }
            if matches!(symbol.package, PackageRefIr::PackageId { .. })
                && symbol.abi_expectation.is_some()
    ) {
        return Err(
            LinkedBytecodeCandidateError::RepresentationCarrierMismatch {
                type_index: row.index(),
                detail: "representation carrier owner is not an exact normalized package symbol",
            },
        );
    }
    let representation = carrier.representation_type();
    let physical = carrier.physical_carrier_type();
    if representation == row.index() || physical == row.index() || representation == physical {
        return Err(
            LinkedBytecodeCandidateError::RepresentationCarrierMismatch {
                type_index: row.index(),
                detail: "owner, representation, and physical carrier rows must be distinct",
            },
        );
    }
    validate_type_plan(representation, row.plan(), location, parts)?;
    validate_type_plan(physical, row.plan(), location, parts)?;

    let representation_row = &parts.types[representation.get() as usize];
    let physical_row = &parts.types[physical.get() as usize];
    if representation_row.representation_carrier().is_some()
        || physical_row.representation_carrier().is_some()
    {
        return Err(
            LinkedBytecodeCandidateError::RepresentationCarrierMismatch {
                type_index: row.index(),
                detail: "representation carrier closure must remain exactly one layer",
            },
        );
    }
    for referenced in [representation_row, physical_row] {
        if referenced.origin().package_build_id() != row.origin().package_build_id()
            || referenced.origin().specialization() != row.origin().specialization()
        {
            return Err(
                LinkedBytecodeCandidateError::RepresentationCarrierMismatch {
                    type_index: row.index(),
                    detail: "representation carrier row has a different exact artifact owner or specialization",
                },
            );
        }
    }
    if !matches!(
        representation_row.type_ref(),
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty()
    ) {
        return Err(
            LinkedBytecodeCandidateError::RepresentationCarrierMismatch {
                type_index: row.index(),
                detail: "source representation row is not the exact builtin integer payload",
            },
        );
    }
    if !matches!(
        physical_row.type_ref(),
        TypeRefIr::Builtin { name, args } if name == "number" && args.is_empty()
    ) {
        return Err(
            LinkedBytecodeCandidateError::RepresentationCarrierMismatch {
                type_index: row.index(),
                detail: "physical carrier row is not the exact builtin number carrier",
            },
        );
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
