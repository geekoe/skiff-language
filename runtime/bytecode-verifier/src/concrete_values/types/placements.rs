use skiff_artifact_model::{NativeValueEmbedding, NativeValueLifecycleConcrete, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedBytecodeCandidate, LinkedContainerLayout, LinkedContainerLayoutKind,
    LinkedContainerPosition, LinkedShapeEntry, LinkedTypeEntry, LinkedWritablePathSegment,
    TypeIndex,
};

use crate::{VerificationError, VerificationLocation, VerificationObligation};

use super::super::{ConcreteTypeFact, ConcreteValueFacts};

pub(super) fn prove_type_placements(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    prove_container_layouts(candidate, facts)?;
    prove_shape_layouts(candidate, facts)?;
    prove_writable_paths(candidate, facts)
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

fn require_container_layout<'a>(
    row: &'a LinkedTypeEntry,
    expected: LinkedContainerLayoutKind,
    location: VerificationLocation,
) -> Result<&'a LinkedContainerLayout, VerificationError> {
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
) -> Result<(), VerificationError> {
    for shape in candidate.shapes() {
        let location = table_location(CandidateTable::Shapes, shape.index().get());
        let nominal = fact_for(facts, shape.nominal_type(), location, "shape nominal type")?;
        for field in shape.fields() {
            let field_fact = fact_for(facts, field.ty(), location, "shape field type")?;
            require_ordinary_snapshot(field_fact, location, "shape field")?;
        }
        if let TypeRefIr::Record { fields } = &nominal.normalized_type {
            prove_structural_shape(shape, fields, facts, location)?;
        }
    }
    Ok(())
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

fn prove_writable_paths(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
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
                } => {
                    prove_dense_field(candidate, facts, current, *shape, *field_ordinal, location)?
                }
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
    current: &ConcreteTypeFact,
    shape_index: skiff_runtime_linked_bytecode::ShapeIndex,
    field_ordinal: u32,
    location: VerificationLocation,
) -> Result<&'a ConcreteTypeFact, VerificationError> {
    let shape = candidate
        .shapes()
        .get(shape_index.get() as usize)
        .ok_or_else(|| violation(location, "writable path shape index is out of bounds"))?;
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
