use super::*;

#[test]
fn package_visible_type_text_maps_names_through_all_composite_shapes() {
    let mappings = BTreeMap::from([
        ("Status".to_string(), "agent.model.Status".to_string()),
        (
            "CleanupReason".to_string(),
            "agent.model.CleanupReason".to_string(),
        ),
    ]);

    assert_eq!(
            package_service_visible_type_text(
                "Map<string, Array<CleanupReason?>?>? | fn(value: Status) -> CleanupReason?",
                &mappings,
            ),
            "Map<string, Array<agent.model.CleanupReason?>?>? | fn(value: agent.model.Status) -> agent.model.CleanupReason?"
        );
}

#[test]
fn package_visible_type_text_does_not_follow_cyclic_mapping_outputs() {
    let mappings = BTreeMap::from([
        ("Left".to_string(), "Right".to_string()),
        ("Right".to_string(), "Left".to_string()),
    ]);

    assert_eq!(
        package_service_visible_type_text("Array<Left?>", &mappings),
        "Array<Right?>"
    );
}
mod timeout_source_semantics;
