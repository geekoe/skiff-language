use std::collections::BTreeMap;

use skiff_artifact_model::{NativeValueLifecycleResolution, TypeRefIr};
use skiff_runtime_linked_bytecode::{CandidateTable, TypeIndex};

use super::{
    ConcreteTypeClass, ConcreteTypeClassId, ConcreteTypeFact, ConcreteValueFacts, ImplicitBuiltin,
    ImplicitBuiltinClasses,
};
use crate::{VerificationError, VerificationLimit, VerificationLocation, VerificationObligation};

pub(super) struct ClassifiedType {
    coordinate: TypeIndex,
    normalized_type: TypeRefIr,
    lifecycle: NativeValueLifecycleResolution,
}

impl ClassifiedType {
    pub(super) const fn new(
        coordinate: TypeIndex,
        normalized_type: TypeRefIr,
        lifecycle: NativeValueLifecycleResolution,
    ) -> Self {
        Self {
            coordinate,
            normalized_type,
            lifecycle,
        }
    }
}

pub(super) fn build_type_classes(
    classified: Vec<ClassifiedType>,
    lifecycle_canonical_bytes: u64,
    max_canonical_bytes: u64,
) -> Result<ConcreteValueFacts, VerificationError> {
    let mut types = Vec::<ConcreteTypeFact>::with_capacity(classified.len());
    let mut classes = Vec::<ConcreteTypeClass>::new();
    let mut buckets = BTreeMap::<Vec<u8>, Vec<ConcreteTypeClassId>>::new();
    let mut used_canonical_bytes = lifecycle_canonical_bytes;

    for classified in classified {
        prove_dense_coordinate(classified.coordinate, types.len())?;
        let location = type_location(classified.coordinate);
        let key = skiff_canonical_json::canonical_json_bytes(&(
            &classified.normalized_type,
            &classified.lifecycle,
        ))
        .map_err(|error| {
            violation(
                location,
                format!("concrete class key cannot be canonically serialized: {error}"),
            )
        })?;
        charge_canonical_bytes(
            &mut used_canonical_bytes,
            key.len(),
            max_canonical_bytes,
            location,
        )?;

        let class = match find_exact_class(
            buckets.get(&key).map(Vec::as_slice),
            &classes,
            &types,
            &classified.normalized_type,
            &classified.lifecycle,
        ) {
            Some(class) => class,
            None => {
                let id = new_class_id(classes.len(), location)?;
                classes.push(ConcreteTypeClass {
                    id,
                    representative: classified.coordinate,
                });
                buckets.entry(key).or_default().push(id);
                id
            }
        };

        types.push(ConcreteTypeFact {
            coordinate: classified.coordinate,
            normalized_type: classified.normalized_type,
            lifecycle: classified.lifecycle,
            class,
        });
    }

    let implicit_builtins = index_implicit_builtins(&classes, &types)?;
    Ok(ConcreteValueFacts {
        types: types.into_boxed_slice(),
        classes: classes.into_boxed_slice(),
        implicit_builtins,
    })
}

fn find_exact_class(
    bucket: Option<&[ConcreteTypeClassId]>,
    classes: &[ConcreteTypeClass],
    types: &[ConcreteTypeFact],
    normalized_type: &TypeRefIr,
    lifecycle: &NativeValueLifecycleResolution,
) -> Option<ConcreteTypeClassId> {
    bucket?.iter().copied().find(|id| {
        class_fact(*id, classes, types).is_some_and(|representative| {
            representative.normalized_type == *normalized_type
                && representative.lifecycle == *lifecycle
        })
    })
}

fn new_class_id(
    position: usize,
    location: VerificationLocation,
) -> Result<ConcreteTypeClassId, VerificationError> {
    u32::try_from(position)
        .map(ConcreteTypeClassId)
        .map_err(|_| violation(location, "concrete class position does not fit u32"))
}

fn class_fact<'a>(
    id: ConcreteTypeClassId,
    classes: &'a [ConcreteTypeClass],
    types: &'a [ConcreteTypeFact],
) -> Option<&'a ConcreteTypeFact> {
    let class = classes
        .get(usize::try_from(id.0).ok()?)
        .filter(|class| class.id == id)?;
    types
        .get(usize::try_from(class.representative.get()).ok()?)
        .filter(|fact| fact.coordinate == class.representative)
}

fn index_implicit_builtins(
    classes: &[ConcreteTypeClass],
    types: &[ConcreteTypeFact],
) -> Result<ImplicitBuiltinClasses, VerificationError> {
    let mut implicit = ImplicitBuiltinClasses::default();
    for class in classes {
        let fact = class_fact(class.id, classes, types).ok_or_else(|| {
            violation(
                VerificationLocation::Image,
                "concrete class representative is not densely indexed",
            )
        })?;
        let Some(builtin) = ImplicitBuiltin::from_type(&fact.normalized_type) else {
            continue;
        };
        let slot = &mut implicit.classes[builtin.ordinal()];
        match *slot {
            None => *slot = Some(class.id),
            Some(existing) if existing == class.id => {}
            Some(_) => {
                return Err(violation(
                    type_location(fact.coordinate),
                    format!(
                        "implicit zero-argument builtin {} has ambiguous lifecycle classes",
                        builtin.name()
                    ),
                ));
            }
        }
    }
    Ok(implicit)
}

fn prove_dense_coordinate(coordinate: TypeIndex, position: usize) -> Result<(), VerificationError> {
    let expected = u32::try_from(position).map_err(|_| {
        violation(
            VerificationLocation::Image,
            "concrete type position does not fit its TypeIndex coordinate",
        )
    })?;
    if coordinate.get() != expected {
        return Err(violation(
            type_location(coordinate),
            "concrete type coordinates are not dense in TypeIndex order",
        ));
    }
    Ok(())
}

fn charge_canonical_bytes(
    used: &mut u64,
    bytes: usize,
    max: u64,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let bytes = u64::try_from(bytes).map_err(|_| VerificationError::LimitExceeded {
        limit: VerificationLimit::ValueLifecycleCanonicalBytes,
        actual: u64::MAX,
        max,
        location,
    })?;
    let next = used
        .checked_add(bytes)
        .ok_or(VerificationError::LimitExceeded {
            limit: VerificationLimit::ValueLifecycleCanonicalBytes,
            actual: u64::MAX,
            max,
            location,
        })?;
    if next > max {
        return Err(VerificationError::LimitExceeded {
            limit: VerificationLimit::ValueLifecycleCanonicalBytes,
            actual: next,
            max,
            location,
        });
    }
    *used = next;
    Ok(())
}

fn type_location(coordinate: TypeIndex) -> VerificationLocation {
    VerificationLocation::Table {
        table: CandidateTable::Types,
        row: coordinate.get(),
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ConcreteTypeAndShape,
        location,
        detail: detail.into(),
    }
}
