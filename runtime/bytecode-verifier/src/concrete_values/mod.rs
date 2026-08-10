mod classes;
mod plans;
mod resolver;
mod types;

use skiff_artifact_model::{NativeValueLifecycleResolution, TypeRefIr};
use skiff_runtime_linked_bytecode::{LinkedBytecodeCandidate, TypeIndex};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{VerificationError, VerificationLimits};

/// Independently rederived concrete value facts in dense `TypeIndex` order.
///
/// Candidate-declared types and plans remain untrusted. Construction stays
/// inside the P2 proof modules; later proof families receive only this
/// read-only result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConcreteValueFacts {
    types: Box<[ConcreteTypeFact]>,
    classes: Box<[ConcreteTypeClass]>,
    implicit_builtins: ImplicitBuiltinClasses,
}

/// One normalized concrete type and its independently classified lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConcreteTypeFact {
    coordinate: TypeIndex,
    normalized_type: TypeRefIr,
    lifecycle: NativeValueLifecycleResolution,
    class: ConcreteTypeClassId,
}

/// Opaque verifier-private semantic identity for one concrete value class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConcreteTypeClassId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConcreteTypeClass {
    id: ConcreteTypeClassId,
    representative: TypeIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImplicitBuiltin {
    Bool,
    Number,
    Integer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ImplicitBuiltinClasses {
    classes: [Option<ConcreteTypeClassId>; 3],
}

impl ConcreteValueFacts {
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            types: Box::new([]),
            classes: Box::new([]),
            implicit_builtins: ImplicitBuiltinClasses::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_classified_types_for_test(
        types: Vec<(TypeRefIr, NativeValueLifecycleResolution)>,
    ) -> Result<Self, VerificationError> {
        Self::from_classified_types_with_budget_for_test(types, 0, u64::MAX)
    }

    #[cfg(test)]
    pub(crate) fn from_classified_types_with_budget_for_test(
        types: Vec<(TypeRefIr, NativeValueLifecycleResolution)>,
        lifecycle_canonical_bytes: u64,
        max_canonical_bytes: u64,
    ) -> Result<Self, VerificationError> {
        let classified = types
            .into_iter()
            .enumerate()
            .map(|(position, (normalized_type, lifecycle))| {
                let coordinate = u32::try_from(position)
                    .map(TypeIndex::new)
                    .map_err(|_| class_violation("test type position does not fit u32"))?;
                Ok(classes::ClassifiedType::new(
                    coordinate,
                    normalized_type,
                    lifecycle,
                ))
            })
            .collect::<Result<Vec<_>, VerificationError>>()?;
        classes::build_type_classes(classified, lifecycle_canonical_bytes, max_canonical_bytes)
    }

    /// Returns the independently derived fact for one dense linked type.
    #[allow(dead_code)]
    pub(crate) fn type_fact(&self, coordinate: TypeIndex) -> Option<&ConcreteTypeFact> {
        usize::try_from(coordinate.get())
            .ok()
            .and_then(|index| self.types.get(index))
            .filter(|fact| fact.coordinate == coordinate)
    }

    /// Returns the opaque semantic class for one exact linked coordinate.
    #[allow(dead_code)]
    pub(crate) fn type_class(&self, index: TypeIndex) -> Option<ConcreteTypeClassId> {
        self.type_fact(index).map(|fact| fact.class)
    }

    /// Compares only independently constructed semantic class identities.
    pub(crate) fn semantically_equal(&self, left: TypeIndex, right: TypeIndex) -> Option<bool> {
        Some(self.type_class(left)? == self.type_class(right)?)
    }

    /// Merges equivalent coordinates to the class's minimum dense member.
    #[allow(dead_code)]
    pub(crate) fn merge_coordinate(
        &self,
        left: TypeIndex,
        right: TypeIndex,
    ) -> Result<TypeIndex, VerificationError> {
        let left_class = self.type_class(left).ok_or_else(|| {
            class_violation(format!(
                "type coordinate {} has no concrete class",
                left.get()
            ))
        })?;
        let right_class = self.type_class(right).ok_or_else(|| {
            class_violation(format!(
                "type coordinate {} has no concrete class",
                right.get()
            ))
        })?;
        if left_class != right_class {
            return Err(class_violation(format!(
                "type coordinates {} and {} belong to different concrete classes",
                left.get(),
                right.get()
            )));
        }
        self.class(left_class)
            .map(|class| class.representative)
            .ok_or_else(|| class_violation("concrete class is not densely indexed"))
    }

    /// Returns the canonical coordinate for an exact zero-argument builtin.
    #[allow(dead_code)]
    pub(crate) fn implicit_representative(&self, builtin: ImplicitBuiltin) -> Option<TypeIndex> {
        let class = self.implicit_builtins.classes[builtin.ordinal()]?;
        self.class(class).map(|class| class.representative)
    }

    fn class(&self, id: ConcreteTypeClassId) -> Option<&ConcreteTypeClass> {
        usize::try_from(id.0)
            .ok()
            .and_then(|index| self.classes.get(index))
            .filter(|class| class.id == id)
    }
}

impl ConcreteTypeFact {
    #[allow(dead_code)]
    pub(crate) const fn coordinate(&self) -> TypeIndex {
        self.coordinate
    }

    #[allow(dead_code)]
    pub(crate) const fn normalized_type(&self) -> &TypeRefIr {
        &self.normalized_type
    }

    #[allow(dead_code)]
    pub(crate) const fn lifecycle(&self) -> &NativeValueLifecycleResolution {
        &self.lifecycle
    }
}

impl ImplicitBuiltin {
    const fn ordinal(self) -> usize {
        match self {
            Self::Bool => 0,
            Self::Number => 1,
            Self::Integer => 2,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Number => "number",
            Self::Integer => "integer",
        }
    }

    fn from_type(ty: &TypeRefIr) -> Option<Self> {
        let TypeRefIr::Builtin { name, args } = ty else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        match name.as_str() {
            "bool" => Some(Self::Bool),
            "number" => Some(Self::Number),
            "integer" => Some(Self::Integer),
            _ => None,
        }
    }
}

fn class_violation(detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: crate::VerificationObligation::ConcreteTypeAndShape,
        location: crate::VerificationLocation::Image,
        detail: detail.into(),
    }
}

/// P2 orchestration seam: resolve exact hydrated facts, independently
/// classify every linked type, then check every candidate-declared lifecycle
/// plan against those facts.
pub(crate) fn prove_types_and_plans(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<ConcreteValueFacts, VerificationError> {
    let mut resolver = resolver::HydratedValueLifecycleResolver::new(hydrated, candidate);
    let facts = types::prove_concrete_types(candidate, &mut resolver, limits)?;
    plans::prove_declared_plans(candidate, &facts, limits)?;
    Ok(facts)
}
