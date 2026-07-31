use super::PublicationSourceRole;

#[test]
fn serializes_publication_source_roles_with_stable_wire_strings() {
    assert_eq!(
        serde_json::to_string(&PublicationSourceRole::Contract).unwrap(),
        "\"contract\""
    );
    assert_eq!(
        serde_json::to_string(&PublicationSourceRole::Implementation).unwrap(),
        "\"implementation\""
    );
    assert_eq!(
        serde_json::to_string(&PublicationSourceRole::Package).unwrap(),
        "\"package\""
    );
}
