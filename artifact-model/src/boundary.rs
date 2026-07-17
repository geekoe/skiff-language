mod operation;
mod projection;
mod value;

pub use operation::*;
pub use projection::*;
pub use value::*;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn boundary_lanes_require_explicit_tag_and_semantic_fields() {
        for invalid in [
            json!({
                "carrier": "detachedValueGraph",
                "encoding": "canonicalValue",
                "owner": "caller",
                "lifetime": "call"
            }),
            json!({
                "kind": "linkable",
                "carrier": "detachedValueGraph",
                "encoding": "canonicalValue",
                "owner": "caller"
            }),
            json!({
                "kind": "linkable",
                "carrier": "detachedValueGraph",
                "encoding": "canonicalValue",
                "owner": "caller",
                "lifetime": "call",
                "providerBuildId": "forbidden"
            }),
        ] {
            assert!(serde_json::from_value::<BoundaryValuePlan>(invalid).is_err());
        }

        assert_eq!(
            serde_json::to_value(BoundaryStreamContract::Unsupported {
                reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
            })
            .unwrap(),
            json!({ "kind": "unsupported", "reason": "languageUnsupported" })
        );
    }

    #[test]
    fn unavailable_projection_requires_stable_non_optional_reason_field() {
        assert!(serde_json::from_value::<BoundaryCallableProjection>(json!({
            "kind": "unavailable"
        }))
        .is_err());
        assert!(serde_json::from_value::<BoundaryCallableProjection>(json!({
            "kind": "available",
            "descriptor": {}
        }))
        .is_err());
        assert_eq!(
            serde_json::to_value(BoundaryCallableProjection::Unavailable {
                reasons: vec![BoundaryUnavailableReason::UnknownCallTarget],
            })
            .unwrap(),
            json!({
                "kind": "unavailable",
                "reasons": [{ "kind": "unknownCallTarget" }]
            })
        );
    }
}
