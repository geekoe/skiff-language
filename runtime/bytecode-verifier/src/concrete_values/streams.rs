use skiff_artifact_model::{
    NativeResourceDropPlan, NativeValueEmbedding, NativeValueLifecycleConcrete, TypeRefIr,
};
use skiff_runtime_linked_bytecode::TypeIndex;

use super::ConcreteValueFacts;
use crate::{VerificationError, VerificationLocation, VerificationObligation};

pub(super) fn derive_item_type(
    facts: &ConcreteValueFacts,
    endpoint: TypeIndex,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let endpoint_fact = facts
        .type_fact(endpoint)
        .ok_or_else(|| violation(location, "stream endpoint has no concrete type fact"))?;
    let TypeRefIr::Builtin { name, args } = endpoint_fact.normalized_type() else {
        return Err(violation(
            location,
            "stream endpoint is not a normalized builtin",
        ));
    };
    let [item] = args.as_slice() else {
        return Err(violation(
            location,
            "stream endpoint does not have exactly one normalized item argument",
        ));
    };
    if name != "Stream"
        || endpoint_fact.lifecycle().embedding != NativeValueEmbedding::Forbidden
        || !matches!(
            &endpoint_fact.lifecycle().lifecycle,
            NativeValueLifecycleConcrete::AffineResource {
                drop: NativeResourceDropPlan::ResourceTableRelease
            }
        )
    {
        return Err(violation(
            location,
            "endpoint is not the authoritative affine Stream<T> resource",
        ));
    }

    let item_fact = facts
        .types
        .iter()
        .find(|fact| fact.normalized_type() == item)
        .ok_or_else(|| {
            violation(
                location,
                "normalized Stream<T> item has no linked concrete coordinate",
            )
        })?;
    let class = facts
        .class(item_fact.class)
        .ok_or_else(|| violation(location, "stream item semantic class is not dense"))?;
    Ok(class.representative)
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ConcreteTypeAndShape,
        location,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{NativeValueDropPlan, NativeValueLifecycleResolution, TypeRefIr};

    use super::*;

    #[test]
    fn derives_item_from_normalized_stream_authority() {
        let facts = facts(stream_lifecycle());
        assert_eq!(
            derive_item_type(&facts, TypeIndex::new(0), VerificationLocation::Image),
            Ok(TypeIndex::new(1)),
        );
    }

    #[test]
    fn rejects_stream_spelling_without_affine_resource_authority() {
        let facts = facts(NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                drop: NativeValueDropPlan::SnapshotRelease,
            },
            embedding: NativeValueEmbedding::Ordinary,
        });
        let error = derive_item_type(&facts, TypeIndex::new(0), VerificationLocation::Image)
            .expect_err("spelling alone must not authorize a stream endpoint");
        assert!(matches!(
            error,
            VerificationError::SemanticViolation {
                obligation: VerificationObligation::ConcreteTypeAndShape,
                location: VerificationLocation::Image,
                ..
            }
        ));
    }

    fn facts(stream: NativeValueLifecycleResolution) -> ConcreteValueFacts {
        ConcreteValueFacts::from_classified_types_for_test(vec![
            (
                TypeRefIr::Builtin {
                    name: "Stream".to_string(),
                    args: vec![TypeRefIr::builtin("string")],
                },
                stream,
            ),
            (
                TypeRefIr::builtin("string"),
                NativeValueLifecycleResolution {
                    lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                        drop: NativeValueDropPlan::SnapshotRelease,
                    },
                    embedding: NativeValueEmbedding::Ordinary,
                },
            ),
        ])
        .unwrap()
    }

    fn stream_lifecycle() -> NativeValueLifecycleResolution {
        NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::AffineResource {
                drop: NativeResourceDropPlan::ResourceTableRelease,
            },
            embedding: NativeValueEmbedding::Forbidden,
        }
    }
}
