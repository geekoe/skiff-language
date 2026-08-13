use std::collections::BTreeSet;

use skiff_artifact_model::{Opcode, PackageBuildId};

use crate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts, LinkedConstantReference, LinkedFrozenConstantValue,
    LinkedWritablePathSegment,
};

use super::{
    check_index, check_package, plans::validate_plan, table_location, types, validate_origin,
};

pub(super) fn validate_data_references(
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    for row in &parts.types {
        types::validate_type(row, parts, package_ids)?;
    }
    for row in &parts.shapes {
        validate_shape(row, parts, package_ids)?;
    }
    for row in &parts.constants {
        validate_constant(row, parts, package_ids)?;
    }
    for (position, root) in parts.constant_roots.iter().enumerate() {
        let location = table_location(CandidateTable::ConstantRoots, position, parts)?;
        check_package(location, root.owner_package_build_id(), package_ids)?;
        check_index(
            location,
            CandidateReferenceKind::Constant,
            root.constant().get(),
            parts.constants.len(),
        )?;
    }
    validate_frozen_graph(parts, package_ids)?;
    for resume in &parts.resume_sites {
        validate_resume_site(resume, parts)?;
    }
    for path in &parts.writable_paths {
        validate_writable_path(path, parts, package_ids)?;
    }
    Ok(())
}

fn validate_shape(
    shape: &crate::LinkedShapeEntry,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::Shapes,
        row: shape.index().get(),
    };
    validate_origin(shape.origin(), location, parts, package_ids)?;
    check_index(
        location,
        CandidateReferenceKind::Type,
        shape.nominal_type().get(),
        parts.types.len(),
    )?;
    for field in shape.fields() {
        check_index(
            location,
            CandidateReferenceKind::Type,
            field.ty().get(),
            parts.types.len(),
        )?;
        validate_plan(field.plan(), location, parts)?;
    }
    Ok(())
}

fn validate_constant(
    constant: &crate::LinkedConstantEntry,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::Constants,
        row: constant.index().get(),
    };
    validate_origin(constant.origin(), location, parts, package_ids)?;
    check_index(
        location,
        CandidateReferenceKind::Type,
        constant.ty().get(),
        parts.types.len(),
    )?;
    validate_plan(constant.plan(), location, parts)?;
    let node_index = constant.reference().node();
    check_index(
        location,
        CandidateReferenceKind::FrozenConstantNode,
        node_index.get(),
        parts.frozen_constant_nodes.len(),
    )?;
    let Some(node) = parts.frozen_constant_nodes.get(node_index.get() as usize) else {
        return Ok(());
    };
    match constant.reference() {
        LinkedConstantReference::LocalNode { .. } => {
            if constant.origin().package_build_id() != node.origin().package_build_id() {
                return Err(LinkedBytecodeCandidateError::ConstantResolutionMismatch {
                    constant: constant.index().get(),
                    node: node_index.get(),
                });
            }
        }
        LinkedConstantReference::PackageSymbol {
            resolved_origin, ..
        } => {
            validate_origin(resolved_origin, location, parts, package_ids)?;
            if resolved_origin != node.origin() {
                return Err(LinkedBytecodeCandidateError::ConstantResolutionMismatch {
                    constant: constant.index().get(),
                    node: node_index.get(),
                });
            }
        }
    }
    Ok(())
}

fn validate_frozen_graph(
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    for node in &parts.frozen_constant_nodes {
        let location = CandidateLocation::TableRow {
            table: CandidateTable::FrozenConstantNodes,
            row: node.index().get(),
        };
        validate_origin(node.origin(), location, parts, package_ids)?;
        for child in node.value().children() {
            if child.get() >= node.index().get() {
                return Err(
                    LinkedBytecodeCandidateError::NonTopologicalFrozenConstantEdge {
                        node: node.index().get(),
                        child: child.get(),
                    },
                );
            }
        }
        validate_frozen_node_value(node, location, parts)?;
    }
    Ok(())
}

fn validate_frozen_node_value(
    node: &crate::LinkedFrozenConstantNode,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    match node.value() {
        LinkedFrozenConstantValue::Literal(_) | LinkedFrozenConstantValue::Array { .. } => Ok(()),
        LinkedFrozenConstantValue::Record { shape, children } => {
            check_index(
                location,
                CandidateReferenceKind::Shape,
                shape.get(),
                parts.shapes.len(),
            )?;
            if let Some(shape_row) = parts.shapes.get(shape.get() as usize) {
                if children.len() != shape_row.fields().len() {
                    return Err(
                        LinkedBytecodeCandidateError::FrozenRecordFieldCountMismatch {
                            node: node.index().get(),
                            shape: shape.get(),
                            child_count: children.len(),
                            field_count: shape_row.fields().len(),
                        },
                    );
                }
            }
            Ok(())
        }
        LinkedFrozenConstantValue::Representation { ty, .. } => check_index(
            location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        ),
        LinkedFrozenConstantValue::Implementation { record, behaviors } => {
            validate_frozen_implementation(node, *record, behaviors, location, parts)
        }
    }
}

fn validate_frozen_implementation(
    node: &crate::LinkedFrozenConstantNode,
    record: crate::FrozenConstantNodeIndex,
    behaviors: &[crate::LinkedFrozenBehaviorBinding],
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    if !matches!(
        parts
            .frozen_constant_nodes
            .get(record.get() as usize)
            .map(|row| row.value()),
        Some(LinkedFrozenConstantValue::Record { .. })
    ) {
        return Err(
            LinkedBytecodeCandidateError::FrozenImplementationRecordMismatch {
                node: node.index().get(),
                record: record.get(),
            },
        );
    }
    if behaviors.is_empty() {
        return Err(LinkedBytecodeCandidateError::EmptyFrozenImplementation {
            node: node.index().get(),
        });
    }
    let mut previous = None;
    for behavior in behaviors {
        if let Some(previous) = previous {
            if behavior.artifact_function_key() <= previous {
                return Err(
                    LinkedBytecodeCandidateError::NonCanonicalFrozenBehaviorOrder {
                        node: node.index().get(),
                        previous: previous.as_str().to_string(),
                        current: behavior.artifact_function_key().as_str().to_string(),
                    },
                );
            }
        }
        previous = Some(behavior.artifact_function_key());
        check_frozen_behavior_target(node, behavior, location, parts)?;
    }
    Ok(())
}

fn check_frozen_behavior_target(
    node: &crate::LinkedFrozenConstantNode,
    behavior: &crate::LinkedFrozenBehaviorBinding,
    location: CandidateLocation,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    check_index(
        location,
        CandidateReferenceKind::Function,
        behavior.function().get(),
        parts.functions.len(),
    )?;
    if let Some(function) = parts.functions.get(behavior.function().get() as usize) {
        if function.key().package_build_id() != node.origin().package_build_id()
            || function.key().artifact_function_key() != behavior.artifact_function_key()
        {
            return Err(LinkedBytecodeCandidateError::FrozenBehaviorTargetMismatch {
                node: node.index().get(),
                function: behavior.function().get(),
            });
        }
    }
    Ok(())
}

fn validate_resume_site(
    resume: &crate::LinkedResumeSite,
    parts: &LinkedBytecodeCandidateParts,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::ResumeSites,
        row: resume.index().get(),
    };
    check_index(
        location,
        CandidateReferenceKind::Function,
        resume.function().get(),
        parts.functions.len(),
    )?;
    let instruction_len = parts
        .functions
        .get(resume.function().get() as usize)
        .map_or(0, |function| function.instructions().len());
    check_index(
        location,
        CandidateReferenceKind::Instruction,
        resume.site().get(),
        instruction_len,
    )?;
    check_index(
        location,
        CandidateReferenceKind::Instruction,
        resume.resume().get(),
        instruction_len,
    )?;
    if let Some(end_resume) = resume.end_resume() {
        check_index(
            location,
            CandidateReferenceKind::Instruction,
            end_resume.get(),
            instruction_len,
        )?;
    }
    let function = &parts.functions[resume.function().get() as usize];
    let site_instruction = &function.instructions()[resume.site().get() as usize];
    match (site_instruction.opcode(), resume.end_resume()) {
        (Opcode::StreamNext, None) => {
            return Err(LinkedBytecodeCandidateError::StreamNextMissingEndResume {
                resume_site: resume.index().get(),
                function: resume.function(),
                site: resume.site(),
            });
        }
        (Opcode::StreamNext, Some(end_resume)) if end_resume == resume.resume() => {
            return Err(
                LinkedBytecodeCandidateError::StreamNextResumeEndTargetsEqual {
                    resume_site: resume.index().get(),
                    function: resume.function(),
                    site: resume.site(),
                    resume: resume.resume(),
                    end_resume,
                },
            );
        }
        (Opcode::StreamNext, Some(_)) => {}
        (_, Some(_)) => {
            return Err(
                LinkedBytecodeCandidateError::EndResumeOnlyValidForStreamNext {
                    resume_site: resume.index().get(),
                    function: resume.function(),
                    site: resume.site(),
                },
            );
        }
        (_, None) => {}
    }
    for ty in resume.result_types() {
        check_index(
            location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        )?;
    }
    for plan in resume.result_plans() {
        validate_plan(plan, location, parts)?;
    }
    Ok(())
}

fn validate_writable_path(
    path: &crate::LinkedWritablePathEntry,
    parts: &LinkedBytecodeCandidateParts,
    package_ids: &BTreeSet<PackageBuildId>,
) -> Result<(), LinkedBytecodeCandidateError> {
    let location = CandidateLocation::TableRow {
        table: CandidateTable::WritablePaths,
        row: path.index().get(),
    };
    validate_origin(path.origin(), location, parts, package_ids)?;
    for ty in [path.root_type(), path.leaf_type()] {
        check_index(
            location,
            CandidateReferenceKind::Type,
            ty.get(),
            parts.types.len(),
        )?;
    }
    for segment in path.segments() {
        match segment {
            LinkedWritablePathSegment::DenseField { shape, .. } => check_index(
                location,
                CandidateReferenceKind::Shape,
                shape.get(),
                parts.shapes.len(),
            )?,
            LinkedWritablePathSegment::ArrayIndex { element_type, .. } => check_index(
                location,
                CandidateReferenceKind::Type,
                element_type.get(),
                parts.types.len(),
            )?,
            LinkedWritablePathSegment::MapKey {
                key_type,
                value_type,
                ..
            } => {
                check_index(
                    location,
                    CandidateReferenceKind::Type,
                    key_type.get(),
                    parts.types.len(),
                )?;
                check_index(
                    location,
                    CandidateReferenceKind::Type,
                    value_type.get(),
                    parts.types.len(),
                )?;
            }
        }
    }
    Ok(())
}
