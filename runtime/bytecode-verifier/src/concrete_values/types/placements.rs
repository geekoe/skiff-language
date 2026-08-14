use skiff_artifact_model::{
    native_value_lifecycle_registry, CallableRegistryTypeExpression, NativeResourceDropPlan,
    NativeValueDropPlan, NativeValueEmbedding, NativeValueLifecycleConcrete,
    PrivilegedAffineFieldAccess, ResourceDropPlan, TypeRefIr, ValueDropPlan, ValueTransferPlan,
};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, CandidateTable, LinkedBytecodeCandidate, LinkedContainerLayout,
    LinkedContainerLayoutKind, LinkedContainerPosition, LinkedResourceDropPlan, LinkedShapeEntry,
    LinkedTypeEntry, LinkedValueDropPlan, LinkedValueTransferPlan, LinkedWritablePathSegment,
    TypeIndex,
};

use crate::{VerificationError, VerificationLocation, VerificationObligation};

use super::super::{
    resolver::HydratedValueLifecycleResolver, ConcreteTypeFact, ConcreteValueFacts,
    PrivilegedAffineShapeFact,
};

pub(super) fn prove_type_placements(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
    resolver: &mut HydratedValueLifecycleResolver<'_>,
) -> Result<Box<[PrivilegedAffineShapeFact]>, VerificationError> {
    prove_container_layouts(candidate, facts)?;
    let privileged = prove_shape_layouts(candidate, facts, resolver)?;
    prove_writable_paths(candidate, facts, &privileged)?;
    Ok(privileged)
}

fn prove_container_layouts(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    for row in candidate.types() {
        let location = table_location(CandidateTable::Types, row.index().get());
        let fact = fact_for(facts, row.index(), location, "container owner type")?;
        let TypeRefIr::Builtin { name, args } = &fact.normalized_type else {
            require_no_container_layout(row, location)?;
            continue;
        };
        match (name.as_str(), args.as_slice()) {
            ("Array", [element]) => {
                let layout =
                    require_container_layout(row, LinkedContainerLayoutKind::Array, location)?;
                prove_position(layout.element(), element, facts, location, "Array element")?;
            }
            ("Map", [key, value]) => {
                let layout =
                    require_container_layout(row, LinkedContainerLayoutKind::Map, location)?;
                prove_position(layout.key(), key, facts, location, "Map key")?;
                prove_position(layout.value(), value, facts, location, "Map value")?;
            }
            ("Json", []) => {
                let layout =
                    require_container_layout(row, LinkedContainerLayoutKind::Json, location)?;
                let position = layout.recursive_value().ok_or_else(|| {
                    violation(location, "Json layout has no recursive value position")
                })?;
                if position.ty() != row.index() {
                    return Err(violation(
                        location,
                        "Json recursive value does not select its exact owner type row",
                    ));
                }
                prove_position(
                    Some(position),
                    &TypeRefIr::builtin("Json"),
                    facts,
                    location,
                    "Json recursive value",
                )?;
            }
            ("JsonObject", []) => {
                let layout =
                    require_container_layout(row, LinkedContainerLayoutKind::JsonObject, location)?;
                prove_position(
                    layout.key(),
                    &TypeRefIr::builtin("string"),
                    facts,
                    location,
                    "JsonObject key",
                )?;
                prove_position(
                    layout.value(),
                    &TypeRefIr::builtin("Json"),
                    facts,
                    location,
                    "JsonObject value",
                )?;
            }
            ("Array" | "Map" | "Json" | "JsonObject", _) => {
                return Err(violation(
                    location,
                    format!("container builtin {name:?} has a noncanonical arity"),
                ));
            }
            _ => require_no_container_layout(row, location)?,
        }
    }
    Ok(())
}

fn require_container_layout(
    row: &LinkedTypeEntry,
    expected: LinkedContainerLayoutKind,
    location: VerificationLocation,
) -> Result<&LinkedContainerLayout, VerificationError> {
    let layout = row
        .container_layout()
        .ok_or_else(|| violation(location, "container type has no concrete position layout"))?;
    if layout.kind() != expected {
        return Err(violation(
            location,
            "container type has the wrong concrete position layout kind",
        ));
    }
    Ok(layout)
}

fn require_no_container_layout(
    row: &LinkedTypeEntry,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if row.container_layout().is_some() {
        return Err(violation(
            location,
            "non-container type retains a concrete container layout",
        ));
    }
    Ok(())
}

fn prove_position(
    position: Option<&LinkedContainerPosition>,
    expected: &TypeRefIr,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
    role: &'static str,
) -> Result<(), VerificationError> {
    let position = position.ok_or_else(|| {
        violation(
            location,
            format!("{role} is absent from the concrete container layout"),
        )
    })?;
    let fact = fact_for(facts, position.ty(), location, role)?;
    if &fact.normalized_type != expected {
        return Err(violation(
            location,
            format!("{role} type differs from the exact normalized child type"),
        ));
    }
    require_ordinary_snapshot(fact, location, role)
}

fn prove_shape_layouts(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
    resolver: &mut HydratedValueLifecycleResolver<'_>,
) -> Result<Box<[PrivilegedAffineShapeFact]>, VerificationError> {
    let mut privileged = Vec::new();
    for shape in candidate.shapes() {
        let location = table_location(CandidateTable::Shapes, shape.index().get());
        let nominal = fact_for(facts, shape.nominal_type(), location, "shape nominal type")?;
        if let Some(identity) = shape.privileged_affine_composite() {
            privileged.push(prove_privileged_shape(
                candidate, facts, resolver, shape, nominal, identity, location,
            )?);
        } else {
            if matches!(
                nominal.lifecycle.embedding,
                NativeValueEmbedding::Privileged
            ) {
                return Err(violation(
                    location,
                    "privileged nominal type lacks explicit linked composite identity",
                ));
            }
            for field in shape.fields() {
                let field_fact = fact_for(facts, field.ty(), location, "shape field type")?;
                require_ordinary_snapshot(field_fact, location, "shape field")?;
            }
        }
        if let TypeRefIr::Record { fields } = &nominal.normalized_type {
            prove_structural_shape(shape, fields, facts, location)?;
        }
    }
    Ok(privileged.into_boxed_slice())
}

fn prove_privileged_shape(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
    resolver: &mut HydratedValueLifecycleResolver<'_>,
    shape: &LinkedShapeEntry,
    nominal: &ConcreteTypeFact,
    identity: skiff_artifact_model::PrivilegedAffineCompositeIdentity,
    location: VerificationLocation,
) -> Result<PrivilegedAffineShapeFact, VerificationError> {
    resolver
        .begin_row(shape.origin().package_build_id())
        .map_err(|error| {
            violation(
                location,
                format!(
                    "privileged shape origin is not exact at {}: {}",
                    error.authority, error.message
                ),
            )
        })?;
    let source = resolver
        .source_shape(*shape.origin().artifact_index())
        .map_err(|error| {
            violation(
                location,
                format!(
                    "privileged source shape is unavailable at {}: {}",
                    error.authority, error.message
                ),
            )
        })?;
    if source.privileged_affine_composite != Some(identity) {
        return Err(violation(
            location,
            "linked privileged identity differs from its exact admitted source shape",
        ));
    }
    require_exact_type_origin(
        candidate,
        shape,
        shape.nominal_type(),
        source.type_ref,
        location,
        "privileged nominal type",
    )?;

    let registry = native_value_lifecycle_registry();
    let schema = registry
        .privileged_affine_composite(identity)
        .ok_or_else(|| {
            violation(
                location,
                "linked privileged identity is absent from the pinned lifecycle registry",
            )
        })?;
    let TypeRefIr::PackageSymbol { symbol } = &nominal.normalized_type else {
        return Err(violation(
            location,
            "privileged shape nominal type is not a package symbol",
        ));
    };
    if registry
        .privileged_affine_composite_for_symbol(symbol)
        .map(|row| row.identity)
        != Some(identity)
    {
        return Err(violation(
            location,
            "privileged shape identity does not match the exact registry symbol",
        ));
    }
    if nominal.lifecycle.lifecycle != schema.lifecycle
        || nominal.lifecycle.embedding != schema.embedding
    {
        return Err(violation(
            location,
            "privileged shape root lifecycle differs from independent classification",
        ));
    }
    if source.fields.len() != schema.fields.len() || shape.fields().len() != schema.fields.len() {
        return Err(violation(
            location,
            "privileged shape field coverage differs from the exact registry schema",
        ));
    }

    let mut affine_field_ordinal = None;
    for (ordinal, ((source_field, linked_field), expected)) in source
        .fields
        .iter()
        .zip(shape.fields())
        .zip(&schema.fields)
        .enumerate()
    {
        let ordinal_u32 = u32::try_from(ordinal)
            .map_err(|_| violation(location, "privileged field ordinal exceeds u32"))?;
        require_exact_type_origin(
            candidate,
            shape,
            linked_field.ty(),
            source_field.type_ref,
            location,
            "privileged field type",
        )?;
        let field_fact = fact_for(facts, linked_field.ty(), location, "privileged field type")?;
        let expected_plan = bridge_field_lifecycle(&expected.lifecycle).ok_or_else(|| {
            violation(
                location,
                "privileged field registry lifecycle is recursively shaped",
            )
        })?;
        if source_field.name != expected.name
            || linked_field.name() != expected.name
            || !matches_registry_type(&expected.ty, &field_fact.normalized_type)
            || field_fact.lifecycle.lifecycle != expected.lifecycle
            || !artifact_plan_matches_lifecycle(&source_field.plan, &expected.lifecycle)
            || linked_field.plan() != &expected_plan
        {
            return Err(violation(
                location,
                format!(
                    "privileged field ordinal {ordinal} differs from its exact source/registry name, type, or lifecycle"
                ),
            ));
        }
        if expected.access == PrivilegedAffineFieldAccess::AffineTake
            && affine_field_ordinal.replace(ordinal_u32).is_some()
        {
            return Err(violation(
                location,
                "privileged shape has more than one affine-take field",
            ));
        }
    }
    let affine_field_ordinal = affine_field_ordinal
        .ok_or_else(|| violation(location, "privileged shape has no exact affine-take field"))?;
    Ok(PrivilegedAffineShapeFact {
        identity,
        shape: shape.index(),
        nominal_type: shape.nominal_type(),
        affine_field_ordinal,
    })
}

fn prove_structural_shape(
    shape: &LinkedShapeEntry,
    expected: &std::collections::BTreeMap<String, TypeRefIr>,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if shape.fields().len() != expected.len() {
        return Err(violation(
            location,
            "structural shape field coverage differs from its normalized nominal type",
        ));
    }
    for field in shape.fields() {
        let expected_type = expected.get(field.name()).ok_or_else(|| {
            violation(
                location,
                "structural shape field name is absent from its normalized nominal type",
            )
        })?;
        let actual = fact_for(facts, field.ty(), location, "structural shape field type")?;
        if &actual.normalized_type != expected_type {
            return Err(violation(
                location,
                "structural shape field differs from its exact normalized field type",
            ));
        }
    }
    Ok(())
}

fn require_exact_type_origin(
    candidate: &LinkedBytecodeCandidate,
    shape: &LinkedShapeEntry,
    ty: TypeIndex,
    artifact_type: u32,
    location: VerificationLocation,
    role: &'static str,
) -> Result<(), VerificationError> {
    let row = candidate
        .types()
        .get(ty.get() as usize)
        .filter(|row| row.index() == ty)
        .ok_or_else(|| violation(location, format!("{role} linked row is absent")))?;
    if row.origin().package_build_id() != shape.origin().package_build_id()
        || row.origin().artifact_index() != &ArtifactTypeIndex::new(artifact_type)
        || row.origin().specialization() != shape.origin().specialization()
    {
        return Err(violation(
            location,
            format!("{role} does not retain the exact source pool coordinate"),
        ));
    }
    Ok(())
}

fn matches_registry_type(expected: &CallableRegistryTypeExpression, actual: &TypeRefIr) -> bool {
    match (expected, actual) {
        (
            CallableRegistryTypeExpression::Builtin { name, arguments },
            TypeRefIr::Builtin {
                name: actual_name,
                args,
            },
        ) => {
            name == actual_name
                && arguments.len() == args.len()
                && arguments
                    .iter()
                    .zip(args)
                    .all(|(expected, actual)| matches_registry_type(expected, actual))
        }
        (
            CallableRegistryTypeExpression::PackageSymbol {
                package_id,
                symbol_path,
            },
            TypeRefIr::PackageSymbol { symbol },
        ) => {
            symbol.symbol_path == *symbol_path
                && matches!(
                    &symbol.package,
                    skiff_artifact_model::PackageRefIr::PackageId {
                        package_id: actual_package_id,
                    } if actual_package_id == package_id
                )
        }
        _ => false,
    }
}

fn bridge_field_lifecycle(
    lifecycle: &NativeValueLifecycleConcrete,
) -> Option<LinkedValueTransferPlan> {
    Some(match lifecycle {
        NativeValueLifecycleConcrete::SnapshotShare { drop } => {
            LinkedValueTransferPlan::SnapshotShare {
                drop: match drop {
                    NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
                    NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
                    NativeValueDropPlan::NativeAdapter { adapter } => {
                        LinkedValueDropPlan::NativeAdapter {
                            adapter: adapter.clone(),
                        }
                    }
                    NativeValueDropPlan::PrivilegedRecursiveShape => return None,
                },
            }
        }
        NativeValueLifecycleConcrete::MoveOnly { drop } => LinkedValueTransferPlan::MoveOnly {
            drop: match drop {
                NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
                NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
                NativeValueDropPlan::NativeAdapter { adapter } => {
                    LinkedValueDropPlan::NativeAdapter {
                        adapter: adapter.clone(),
                    }
                }
                NativeValueDropPlan::PrivilegedRecursiveShape => return None,
            },
        },
        NativeValueLifecycleConcrete::AffineResource { drop } => {
            LinkedValueTransferPlan::AffineResource {
                drop: match drop {
                    NativeResourceDropPlan::ResourceTableRelease => {
                        LinkedResourceDropPlan::ResourceTableRelease
                    }
                    NativeResourceDropPlan::NativeAdapter { adapter } => {
                        LinkedResourceDropPlan::NativeAdapter {
                            adapter: adapter.clone(),
                        }
                    }
                },
            }
        }
        NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => LinkedValueTransferPlan::ExplicitCloneLease {
            clone_adapter: clone_adapter.clone(),
            drop: match drop {
                NativeResourceDropPlan::ResourceTableRelease => {
                    LinkedResourceDropPlan::ResourceTableRelease
                }
                NativeResourceDropPlan::NativeAdapter { adapter } => {
                    LinkedResourceDropPlan::NativeAdapter {
                        adapter: adapter.clone(),
                    }
                }
            },
        },
    })
}

fn artifact_plan_matches_lifecycle(
    plan: &ValueTransferPlan,
    lifecycle: &NativeValueLifecycleConcrete,
) -> bool {
    match (plan, lifecycle) {
        (
            ValueTransferPlan::SnapshotShare { drop },
            NativeValueLifecycleConcrete::SnapshotShare {
                drop: expected_drop,
            },
        )
        | (
            ValueTransferPlan::MoveOnly { drop },
            NativeValueLifecycleConcrete::MoveOnly {
                drop: expected_drop,
            },
        ) => match (drop, expected_drop) {
            (ValueDropPlan::Trivial, NativeValueDropPlan::Trivial)
            | (ValueDropPlan::SnapshotRelease, NativeValueDropPlan::SnapshotRelease) => true,
            (
                ValueDropPlan::NativeAdapter { adapter },
                NativeValueDropPlan::NativeAdapter { adapter: expected },
            ) => adapter.binding_key == expected.binding_key,
            (
                ValueDropPlan::RecursiveShape { .. },
                NativeValueDropPlan::PrivilegedRecursiveShape,
            ) => true,
            _ => false,
        },
        (
            ValueTransferPlan::AffineResource { drop },
            NativeValueLifecycleConcrete::AffineResource {
                drop: expected_drop,
            },
        ) => match (drop, expected_drop) {
            (
                ResourceDropPlan::ResourceTableRelease,
                NativeResourceDropPlan::ResourceTableRelease,
            ) => true,
            (
                ResourceDropPlan::NativeAdapter { adapter },
                NativeResourceDropPlan::NativeAdapter { adapter: expected },
            ) => adapter.binding_key == expected.binding_key,
            _ => false,
        },
        _ => false,
    }
}

fn prove_writable_paths(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
    privileged: &[PrivilegedAffineShapeFact],
) -> Result<(), VerificationError> {
    for path in candidate.writable_paths() {
        let location = table_location(CandidateTable::WritablePaths, path.index().get());
        let mut current = fact_for(facts, path.root_type(), location, "writable path root")?;
        let mut next_selector = 0_u32;
        for segment in path.segments() {
            current = match segment {
                LinkedWritablePathSegment::DenseField {
                    shape,
                    field_ordinal,
                } => prove_dense_field(
                    candidate,
                    facts,
                    privileged,
                    current,
                    *shape,
                    *field_ordinal,
                    location,
                )?,
                LinkedWritablePathSegment::ArrayIndex {
                    selector_ordinal,
                    element_type,
                } => {
                    prove_selector(*selector_ordinal, &mut next_selector, location)?;
                    prove_array_index(facts, current, *element_type, location)?
                }
                LinkedWritablePathSegment::MapKey {
                    selector_ordinal,
                    key_type,
                    value_type,
                } => {
                    prove_selector(*selector_ordinal, &mut next_selector, location)?;
                    prove_map_key(facts, current, *key_type, *value_type, location)?
                }
            };
        }
        if next_selector != path.selector_count() {
            return Err(violation(
                location,
                "writable path selector count differs from its traversed segments",
            ));
        }
        let leaf = fact_for(facts, path.leaf_type(), location, "writable path leaf")?;
        if current.normalized_type != leaf.normalized_type {
            return Err(violation(
                location,
                "writable path traversal does not resolve to its declared leaf type",
            ));
        }
    }
    Ok(())
}

fn prove_dense_field<'a>(
    candidate: &'a LinkedBytecodeCandidate,
    facts: &'a ConcreteValueFacts,
    privileged: &[PrivilegedAffineShapeFact],
    current: &ConcreteTypeFact,
    shape_index: skiff_runtime_linked_bytecode::ShapeIndex,
    field_ordinal: u32,
    location: VerificationLocation,
) -> Result<&'a ConcreteTypeFact, VerificationError> {
    let shape = candidate
        .shapes()
        .get(shape_index.get() as usize)
        .ok_or_else(|| violation(location, "writable path shape index is out of bounds"))?;
    if privileged.iter().any(|fact| fact.shape == shape_index) {
        return Err(violation(
            location,
            "writable paths cannot project through a privileged affine shape",
        ));
    }
    let nominal = fact_for(
        facts,
        shape.nominal_type(),
        location,
        "path shape nominal type",
    )?;
    if current.normalized_type != nominal.normalized_type {
        return Err(violation(
            location,
            "writable dense-field segment does not match the current nominal type",
        ));
    }
    let field = shape
        .fields()
        .get(field_ordinal as usize)
        .ok_or_else(|| violation(location, "writable dense-field ordinal is out of bounds"))?;
    fact_for(facts, field.ty(), location, "writable dense-field result")
}

fn prove_array_index<'a>(
    facts: &'a ConcreteValueFacts,
    current: &ConcreteTypeFact,
    element_type: TypeIndex,
    location: VerificationLocation,
) -> Result<&'a ConcreteTypeFact, VerificationError> {
    let element = fact_for(facts, element_type, location, "writable Array element")?;
    let TypeRefIr::Builtin { name, args } = &current.normalized_type else {
        return Err(violation(
            location,
            "writable Array segment has a non-Array owner",
        ));
    };
    if name != "Array" || args.as_slice() != std::slice::from_ref(&element.normalized_type) {
        return Err(violation(
            location,
            "writable Array segment element differs from its exact owner argument",
        ));
    }
    Ok(element)
}

fn prove_map_key<'a>(
    facts: &'a ConcreteValueFacts,
    current: &ConcreteTypeFact,
    key_type: TypeIndex,
    value_type: TypeIndex,
    location: VerificationLocation,
) -> Result<&'a ConcreteTypeFact, VerificationError> {
    let key = fact_for(facts, key_type, location, "writable Map key")?;
    let value = fact_for(facts, value_type, location, "writable Map value")?;
    let TypeRefIr::Builtin { name, args } = &current.normalized_type else {
        return Err(violation(
            location,
            "writable Map segment has a non-Map owner",
        ));
    };
    if name != "Map"
        || args.len() != 2
        || args[0] != key.normalized_type
        || args[1] != value.normalized_type
    {
        return Err(violation(
            location,
            "writable Map segment key/value differ from its exact owner arguments",
        ));
    }
    Ok(value)
}

fn prove_selector(
    actual: u32,
    next: &mut u32,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if actual != *next {
        return Err(violation(
            location,
            "writable path selector ordinals are not dense",
        ));
    }
    *next = next
        .checked_add(1)
        .ok_or_else(|| violation(location, "writable path selector count overflowed u32"))?;
    Ok(())
}

fn require_ordinary_snapshot(
    fact: &ConcreteTypeFact,
    location: VerificationLocation,
    role: &'static str,
) -> Result<(), VerificationError> {
    if fact.lifecycle.embedding != NativeValueEmbedding::Ordinary
        || !matches!(
            fact.lifecycle.lifecycle,
            NativeValueLifecycleConcrete::SnapshotShare { .. }
        )
    {
        return Err(violation(
            location,
            format!("{role} is not an Ordinary SnapshotShare value"),
        ));
    }
    Ok(())
}

fn fact_for<'a>(
    facts: &'a ConcreteValueFacts,
    index: TypeIndex,
    location: VerificationLocation,
    role: &'static str,
) -> Result<&'a ConcreteTypeFact, VerificationError> {
    facts.types.get(index.get() as usize).ok_or_else(|| {
        violation(
            location,
            format!("{role} type index is outside the independently proven facts"),
        )
    })
}

fn table_location(table: CandidateTable, row: u32) -> VerificationLocation {
    VerificationLocation::Table { table, row }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ConcreteTypeAndShape,
        location,
        detail: detail.into(),
    }
}
