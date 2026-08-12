use super::*;

const VALID_CATALOG: &str = r#"schemaVersion: skiff-platform-error-projection-catalog-v1
entries:
  - projectionKey: alpha.Error
    producerFamily: alpha
    semanticAdapterOwner: runtime.alpha
    publicMessagePolicy: semanticAdapterSanitized
    envelopeKind: platformError
    fallbackPolicy: fixedInternalErrorBeforeEnvelope
  - projectionKey: beta.Error
    producerFamily: beta
    semanticAdapterOwner: runtime.beta
    publicMessagePolicy: semanticAdapterSanitized
    envelopeKind: platformError
    fallbackPolicy: fixedInternalErrorBeforeEnvelope
"#;

#[test]
fn parses_and_serializes_only_projection_policy_facts() {
    let catalog = PlatformErrorProjectionCatalog::parse(VALID_CATALOG).unwrap();
    assert_eq!(
        catalog.schema_version(),
        PLATFORM_ERROR_PROJECTION_CATALOG_SCHEMA_VERSION
    );
    assert_eq!(catalog.entries().len(), 2);

    let serialized = serde_yaml::to_string(&catalog).unwrap();
    for forbidden in [
        "fields:",
        "fieldTypes:",
        "fieldOrder:",
        "publicFields:",
        "publicType:",
        "nominalIdentity:",
        "codecVersion:",
    ] {
        assert!(!serialized.contains(forbidden), "{serialized}");
    }
    assert_eq!(
        PlatformErrorProjectionCatalog::parse(&serialized).unwrap(),
        catalog
    );

    let with_field_schema = VALID_CATALOG.replace(
        "    producerFamily: alpha\n",
        "    producerFamily: alpha\n    fields: []\n",
    );
    assert!(PlatformErrorProjectionCatalog::parse(&with_field_schema)
        .unwrap_err()
        .contains("unknown field `fields`"));
}

#[test]
fn rejects_unknown_fields_and_schema_mismatch() {
    let unknown_top_level = VALID_CATALOG.replace("entries:\n", "unknown: true\nentries:\n");
    assert!(PlatformErrorProjectionCatalog::parse(&unknown_top_level)
        .unwrap_err()
        .contains("unknown field `unknown`"));

    let unknown_entry = VALID_CATALOG.replace(
        "    producerFamily: alpha\n",
        "    producerFamily: alpha\n    unknown: true\n",
    );
    assert!(PlatformErrorProjectionCatalog::parse(&unknown_entry)
        .unwrap_err()
        .contains("unknown field `unknown`"));

    let wrong_schema = VALID_CATALOG.replace(
        PLATFORM_ERROR_PROJECTION_CATALOG_SCHEMA_VERSION,
        "skiff-platform-error-projection-catalog-v2",
    );
    assert!(PlatformErrorProjectionCatalog::parse(&wrong_schema)
        .unwrap_err()
        .contains("schemaVersion must be skiff-platform-error-projection-catalog-v1"));
}

#[test]
fn rejects_unsorted_and_duplicate_projection_keys() {
    let unsorted = VALID_CATALOG
        .replace("projectionKey: alpha.Error", "projectionKey: zeta.Error")
        .replace("projectionKey: beta.Error", "projectionKey: alpha.Error");
    assert!(PlatformErrorProjectionCatalog::parse(&unsorted)
        .unwrap_err()
        .contains("strictly ascending ASCII projectionKey order"));

    let duplicate =
        VALID_CATALOG.replace("projectionKey: beta.Error", "projectionKey: alpha.Error");
    assert!(PlatformErrorProjectionCatalog::parse(&duplicate)
        .unwrap_err()
        .contains("duplicate projectionKey alpha.Error"));
}

#[test]
fn rejects_invalid_and_versioned_projection_keys() {
    for invalid in [
        "NoDot",
        "alpha/Error",
        "alpha.Érror",
        " alpha.Error",
        "alpha.Error ",
        "alpha.Error.v0",
        "alpha.Error.v000",
        "alpha.Error.v1",
        "alpha.Error.v01",
    ] {
        let text = VALID_CATALOG.replacen("alpha.Error", &format!("\"{invalid}\""), 1);
        assert!(
            PlatformErrorProjectionCatalog::parse(&text).is_err(),
            "accepted invalid projection key {invalid:?}"
        );
    }

    let too_long = format!("alpha.{}", "a".repeat(123));
    let text = VALID_CATALOG.replacen("alpha.Error", &too_long, 1);
    assert!(PlatformErrorProjectionCatalog::parse(&text)
        .unwrap_err()
        .contains("between 1 and 128 bytes"));
}

#[test]
fn rejects_invalid_policy_tokens() {
    for (valid, invalid) in [
        ("producerFamily: alpha", "producerFamily: alpha/beta"),
        (
            "semanticAdapterOwner: runtime.alpha",
            "semanticAdapterOwner: \" runtime.alpha\"",
        ),
        (
            "publicMessagePolicy: semanticAdapterSanitized",
            "publicMessagePolicy: \"\"",
        ),
        (
            "envelopeKind: platformError",
            "envelopeKind: platform error",
        ),
        (
            "fallbackPolicy: fixedInternalErrorBeforeEnvelope",
            "fallbackPolicy: fixed@internal",
        ),
    ] {
        let text = VALID_CATALOG.replacen(valid, invalid, 1);
        assert!(
            PlatformErrorProjectionCatalog::parse(&text).is_err(),
            "accepted invalid policy replacement {invalid:?}"
        );
    }

    let too_long = format!("producerFamily: {}", "a".repeat(129));
    let text = VALID_CATALOG.replacen("producerFamily: alpha", &too_long, 1);
    assert!(PlatformErrorProjectionCatalog::parse(&text)
        .unwrap_err()
        .contains("between 1 and 128 bytes"));
}
