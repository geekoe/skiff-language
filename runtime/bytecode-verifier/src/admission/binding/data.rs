use skiff_artifact_model::{BytecodeConstantRef, BytecodePoolEntry, WritablePathSegment};
use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedArtifactPoolOrigin, LinkedBytecodeCandidate, LinkedConstantReference,
    LinkedWritablePathSegment, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

use crate::{VerificationError, VerificationLocation};

use super::{semantic_violation, table_location};

pub(super) fn prove_artifact_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    prove_type_origins(hydrated, candidate)?;
    prove_shape_origins(hydrated, candidate)?;
    prove_constant_origins(hydrated, candidate)?;
    prove_node_origins(hydrated, candidate)?;
    prove_capture_origins(hydrated, candidate)?;
    prove_path_origins(hydrated, candidate)
}

fn prove_type_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for row in candidate.types() {
        let location = table_location(CandidateTable::Types, row.index().get());
        let package = package_for_origin(hydrated, candidate, row.origin(), location)?;
        let source = package
            .bytecode()
            .view()
            .pools()
            .types
            .get(row.origin().artifact_index().get() as usize);
        let Some(BytecodePoolEntry::TypeRef { ty }) = source else {
            return Err(origin_row_mismatch(location, "type"));
        };
        if row.origin().specialization().is_none() && row.type_ref() != ty {
            return Err(semantic_violation(
                location,
                "package-global linked type differs from its exact artifact type row",
            ));
        }
    }
    Ok(())
}

fn prove_shape_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for row in candidate.shapes() {
        let location = table_location(CandidateTable::Shapes, row.index().get());
        let package = package_for_origin(hydrated, candidate, row.origin(), location)?;
        let source = package
            .bytecode()
            .view()
            .pools()
            .shapes
            .get(row.origin().artifact_index().get() as usize);
        let Some(BytecodePoolEntry::ShapeRef { shape }) = source else {
            return Err(origin_row_mismatch(location, "shape"));
        };
        if row.fields().len() != shape.fields.len() {
            return Err(semantic_violation(
                location,
                "linked shape field coverage differs from its exact artifact row",
            ));
        }
        prove_related_type_origin(
            candidate,
            row.nominal_type(),
            package,
            shape.type_ref,
            row.origin().specialization(),
            location,
        )?;
        for (linked, source) in row.fields().iter().zip(&shape.fields) {
            if linked.name() != source.name {
                return Err(semantic_violation(
                    location,
                    "linked shape field name differs from its exact artifact row",
                ));
            }
            prove_related_type_origin(
                candidate,
                linked.ty(),
                package,
                source.type_ref,
                row.origin().specialization(),
                location,
            )?;
        }
    }
    Ok(())
}

fn prove_constant_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for row in candidate.constants() {
        let location = table_location(CandidateTable::Constants, row.index().get());
        let package = package_for_origin(hydrated, candidate, row.origin(), location)?;
        let source = package
            .bytecode()
            .view()
            .pools()
            .constants
            .get(row.origin().artifact_index().get() as usize);
        let Some(BytecodePoolEntry::ConstantRef {
            reference,
            type_ref,
            ..
        }) = source
        else {
            return Err(origin_row_mismatch(location, "constant"));
        };
        prove_related_type_origin(
            candidate,
            row.ty(),
            package,
            *type_ref,
            row.origin().specialization(),
            location,
        )?;
        prove_constant_reference(
            hydrated,
            package,
            candidate,
            row.origin(),
            row.reference(),
            reference,
            location,
        )?;
    }
    Ok(())
}

fn prove_constant_reference(
    hydrated: &HydratedDeploymentBytecode,
    owner: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    constant_origin: &LinkedArtifactPoolOrigin<
        skiff_runtime_linked_bytecode::ArtifactConstantIndex,
    >,
    linked: &LinkedConstantReference,
    source: &BytecodeConstantRef,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    match (linked, source) {
        (
            LinkedConstantReference::LocalNode { node },
            BytecodeConstantRef::LocalNode { node_index },
        ) => {
            let linked_node = candidate
                .frozen_constant_nodes()
                .get(node.get() as usize)
                .ok_or_else(|| semantic_violation(location, "constant node is out of bounds"))?;
            if linked_node.origin().package_build_id() != &owner.reference().package_build_id
                || linked_node.origin().artifact_index().get() != *node_index
                || linked_node.origin().specialization() != constant_origin.specialization()
            {
                return Err(semantic_violation(
                    location,
                    "local constant node does not match its exact artifact node index",
                ));
            }
        }
        (
            LinkedConstantReference::PackageSymbol {
                source,
                resolved_origin,
                node,
            },
            BytecodeConstantRef::PackageSymbol { symbol },
        ) if source == symbol => {
            let linked_node = candidate
                .frozen_constant_nodes()
                .get(node.get() as usize)
                .ok_or_else(|| semantic_violation(location, "constant node is out of bounds"))?;
            if linked_node.origin() != resolved_origin {
                return Err(semantic_violation(
                    location,
                    "package constant resolution disagrees with the selected frozen node origin",
                ));
            }
            let target = package_for_origin(hydrated, candidate, resolved_origin, location)?;
            let exact_target = resolve_package_ref(hydrated, owner, &symbol.package)
                .is_some_and(|expected| expected.reference() == target.reference());
            let exact_root = target
                .bytecode()
                .view()
                .constant_roots()
                .get(&symbol.symbol_path)
                .and_then(|constant_index| {
                    target
                        .bytecode()
                        .view()
                        .pools()
                        .constants
                        .get(*constant_index as usize)
                })
                .is_some_and(|entry| {
                    matches!(
                        entry,
                        BytecodePoolEntry::ConstantRef {
                            reference: BytecodeConstantRef::LocalNode { node_index },
                            ..
                        } if *node_index == resolved_origin.artifact_index().get()
                    )
                });
            if !exact_target || !exact_root || resolved_origin.specialization().is_some() {
                return Err(semantic_violation(
                    location,
                    "package constant does not resolve to the exact hydrated root node",
                ));
            }
        }
        _ => {
            return Err(semantic_violation(
                location,
                "linked constant reference kind or symbol differs from its artifact row",
            ));
        }
    }
    Ok(())
}

fn resolve_package_ref<'a>(
    hydrated: &'a HydratedDeploymentBytecode,
    caller: &HydratedBytecodePackage,
    reference: &skiff_artifact_model::PackageRefIr,
) -> Option<&'a HydratedBytecodePackage> {
    match reference {
        skiff_artifact_model::PackageRefIr::PackageId { package_id } => {
            let mut matches = hydrated
                .packages()
                .values()
                .filter(|package| package.reference().package_id == *package_id);
            let selected = matches.next()?;
            matches.next().is_none().then_some(selected)
        }
        skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: caller.reference().package_build_id.clone(),
                package_requirement_alias: dependency_ref.clone(),
            };
            let binding = hydrated
                .deployment()
                .package_bindings
                .iter()
                .find(|binding| binding.key == key)?;
            hydrated
                .packages()
                .get(&binding.package.package_build_id)
                .filter(|package| package.reference() == &binding.package)
        }
    }
}

fn prove_node_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for row in candidate.frozen_constant_nodes() {
        let location = table_location(CandidateTable::FrozenConstantNodes, row.index().get());
        let package = package_for_origin(hydrated, candidate, row.origin(), location)?;
        if package
            .bytecode()
            .view()
            .frozen_constant_graph()
            .nodes
            .get(row.origin().artifact_index().get() as usize)
            .is_none()
        {
            return Err(origin_row_mismatch(location, "frozen constant node"));
        }
    }
    Ok(())
}

fn prove_capture_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for row in candidate.callback_capture_layouts() {
        let location = table_location(CandidateTable::CallbackCaptureLayouts, row.index().get());
        let package = package_for_origin(hydrated, candidate, row.origin(), location)?;
        let source = package
            .bytecode()
            .view()
            .pools()
            .callback_capture
            .get(row.origin().artifact_index().get() as usize);
        let Some(BytecodePoolEntry::CallbackCaptureLayout(source)) = source else {
            return Err(origin_row_mismatch(location, "callback capture"));
        };
        let function = candidate
            .functions()
            .get(row.function().get() as usize)
            .ok_or_else(|| semantic_violation(location, "capture function is out of bounds"))?;
        let exact = row.artifact_function_key().as_str() == source.function_key
            && function.key().package_build_id() == &package.reference().package_build_id
            && function.key().artifact_function_key() == row.artifact_function_key()
            && row.origin().specialization() == Some(function.key())
            && row.captures().len() == source.captures.len();
        if !exact {
            return Err(semantic_violation(
                location,
                "linked callback capture owner or coverage differs from its artifact row",
            ));
        }
        for (linked, source) in row.captures().iter().zip(&source.captures) {
            if linked.slot().get() != source.target_slot {
                return Err(semantic_violation(
                    location,
                    "linked callback capture slot differs from its artifact row",
                ));
            }
            prove_related_type_origin(
                candidate,
                linked.ty(),
                package,
                source.type_ref,
                row.origin().specialization(),
                location,
            )?;
        }
    }
    Ok(())
}

fn prove_path_origins(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    for row in candidate.writable_paths() {
        let location = table_location(CandidateTable::WritablePaths, row.index().get());
        let package = package_for_origin(hydrated, candidate, row.origin(), location)?;
        let source = package
            .bytecode()
            .view()
            .pools()
            .writable_paths
            .get(row.origin().artifact_index().get() as usize);
        let Some(BytecodePoolEntry::WritablePath(source)) = source else {
            return Err(origin_row_mismatch(location, "writable path"));
        };
        if row.segments().len() != source.segments.len()
            || row.selector_count() != source.selector_count()
        {
            return Err(semantic_violation(
                location,
                "linked writable-path shape differs from its exact artifact row",
            ));
        }
        prove_related_type_origin(
            candidate,
            row.root_type(),
            package,
            source.root_type_ref,
            row.origin().specialization(),
            location,
        )?;
        prove_related_type_origin(
            candidate,
            row.leaf_type(),
            package,
            source.leaf_type_ref,
            row.origin().specialization(),
            location,
        )?;
        for (linked, source) in row.segments().iter().zip(&source.segments) {
            prove_writable_segment(
                candidate,
                package,
                row.origin().specialization(),
                linked,
                source,
                location,
            )?;
        }
    }
    Ok(())
}

fn prove_writable_segment(
    candidate: &LinkedBytecodeCandidate,
    package: &HydratedBytecodePackage,
    specialization: Option<&SpecializationKey>,
    linked: &LinkedWritablePathSegment,
    source: &WritablePathSegment,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    match (linked, source) {
        (
            LinkedWritablePathSegment::DenseField {
                shape,
                field_ordinal,
            },
            WritablePathSegment::DenseField {
                shape_ref,
                field_ordinal: source_ordinal,
            },
        ) if field_ordinal == source_ordinal => {
            let linked_shape = candidate
                .shapes()
                .get(shape.get() as usize)
                .ok_or_else(|| semantic_violation(location, "path shape is out of bounds"))?;
            prove_related_origin(
                linked_shape.origin(),
                package,
                *shape_ref,
                specialization,
                location,
            )
        }
        (
            LinkedWritablePathSegment::ArrayIndex {
                selector_ordinal,
                element_type,
            },
            WritablePathSegment::ArrayIndex {
                selector_ordinal: source_ordinal,
                element_type_ref,
            },
        ) if selector_ordinal == source_ordinal => prove_related_type_origin(
            candidate,
            *element_type,
            package,
            *element_type_ref,
            specialization,
            location,
        ),
        (
            LinkedWritablePathSegment::MapKey {
                selector_ordinal,
                key_type,
                value_type,
            },
            WritablePathSegment::MapKey {
                selector_ordinal: source_ordinal,
                key_type_ref,
                value_type_ref,
            },
        ) if selector_ordinal == source_ordinal => {
            prove_related_type_origin(
                candidate,
                *key_type,
                package,
                *key_type_ref,
                specialization,
                location,
            )?;
            prove_related_type_origin(
                candidate,
                *value_type,
                package,
                *value_type_ref,
                specialization,
                location,
            )
        }
        _ => Err(semantic_violation(
            location,
            "linked writable-path segment differs from its exact artifact row",
        )),
    }
}

fn prove_related_type_origin(
    candidate: &LinkedBytecodeCandidate,
    ty: TypeIndex,
    package: &HydratedBytecodePackage,
    artifact_index: u32,
    specialization: Option<&SpecializationKey>,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let linked = candidate
        .types()
        .get(ty.get() as usize)
        .ok_or_else(|| semantic_violation(location, "related type is out of bounds"))?;
    prove_related_origin(
        linked.origin(),
        package,
        artifact_index,
        specialization,
        location,
    )
}

fn prove_related_origin<I>(
    origin: &LinkedArtifactPoolOrigin<I>,
    package: &HydratedBytecodePackage,
    artifact_index: u32,
    specialization: Option<&SpecializationKey>,
    location: VerificationLocation,
) -> Result<(), VerificationError>
where
    I: ArtifactIndex,
{
    let exact_specialization = match specialization {
        Some(specialization) => origin
            .specialization()
            .is_none_or(|actual| actual == specialization),
        None => origin.specialization().is_none(),
    };
    if origin.package_build_id() != &package.reference().package_build_id
        || origin.artifact_index().get_index() != artifact_index
        || !exact_specialization
    {
        return Err(semantic_violation(
            location,
            "related linked row does not carry the exact artifact origin",
        ));
    }
    Ok(())
}

trait ArtifactIndex {
    fn get_index(&self) -> u32;
}

impl ArtifactIndex for skiff_runtime_linked_bytecode::ArtifactTypeIndex {
    fn get_index(&self) -> u32 {
        self.get()
    }
}

impl ArtifactIndex for skiff_runtime_linked_bytecode::ArtifactShapeIndex {
    fn get_index(&self) -> u32 {
        self.get()
    }
}

pub(super) fn prove_constant_roots(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    let expected_count = hydrated
        .packages()
        .values()
        .try_fold(0_usize, |count, package| {
            count.checked_add(package.bytecode().view().constant_roots().len())
        })
        .ok_or_else(|| {
            semantic_violation(
                VerificationLocation::Image,
                "constant root count overflowed usize",
            )
        })?;
    if expected_count != candidate.constant_roots().len() {
        return Err(semantic_violation(
            VerificationLocation::Image,
            format!(
                "candidate has {} constant roots; exact hydration has {expected_count}",
                candidate.constant_roots().len()
            ),
        ));
    }

    for (build_id, package) in hydrated.packages() {
        for (symbol_path, artifact_index) in package.bytecode().view().constant_roots() {
            let root = candidate.constant_roots().iter().find(|root| {
                root.owner_package_build_id() == build_id
                    && root.symbol_path().as_str() == symbol_path
            });
            let Some(root) = root else {
                return Err(semantic_violation(
                    VerificationLocation::Image,
                    format!("candidate is missing constant root {build_id}/{symbol_path}"),
                ));
            };
            let constant = candidate
                .constants()
                .get(root.constant().get() as usize)
                .ok_or_else(|| {
                    semantic_violation(
                        VerificationLocation::Image,
                        format!("constant root {build_id}/{symbol_path} is out of bounds"),
                    )
                })?;
            if constant.origin().package_build_id() != build_id
                || constant.origin().artifact_index().get() != *artifact_index
                || constant.origin().specialization().is_some()
            {
                return Err(semantic_violation(
                    VerificationLocation::Image,
                    format!(
                        "constant root {build_id}/{symbol_path} does not select its exact package-global artifact row"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn package_for_origin<'a, I>(
    hydrated: &'a HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    origin: &LinkedArtifactPoolOrigin<I>,
    location: VerificationLocation,
) -> Result<&'a HydratedBytecodePackage, VerificationError> {
    let package = hydrated
        .packages()
        .get(origin.package_build_id())
        .ok_or_else(|| semantic_violation(location, "artifact origin package is not hydrated"))?;
    if let Some(specialization) = origin.specialization() {
        prove_origin_specialization(package, candidate, specialization, location)?;
    }
    Ok(package)
}

fn prove_origin_specialization(
    package: &HydratedBytecodePackage,
    candidate: &LinkedBytecodeCandidate,
    specialization: &SpecializationKey,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if specialization.package_build_id() != &package.reference().package_build_id
        || !candidate
            .functions()
            .iter()
            .any(|function| function.key() == specialization)
        || !package
            .bytecode()
            .view()
            .functions()
            .iter()
            .any(|function| {
                function.function_key == specialization.artifact_function_key().as_str()
            })
    {
        return Err(semantic_violation(
            location,
            "artifact origin specialization is not an exact candidate function in its package",
        ));
    }
    Ok(())
}

fn origin_row_mismatch(location: VerificationLocation, kind: &'static str) -> VerificationError {
    semantic_violation(
        location,
        format!("linked {kind} origin has no exact artifact row of that kind"),
    )
}
