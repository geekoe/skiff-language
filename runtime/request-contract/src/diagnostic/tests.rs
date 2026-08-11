use std::{borrow::Cow, fmt};

use super::{
    DiagnosticAttributeRecordOutcome, DiagnosticAttributes, DiagnosticCode, DiagnosticFieldKey,
    DiagnosticFieldValue, RuntimeDiagnostic, StaticDiagnosticToken, MAX_DIAGNOSTIC_ATTRIBUTES,
};

const CODE_128: &str = concat!(
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);
const CODE_129: &str = concat!(
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "a"
);
const TOKEN_64: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOKEN_65: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VALID_CODE: DiagnosticCode = match DiagnosticCode::new("runtime.boundary-error") {
    Some(code) => code,
    None => panic!("test diagnostic code must be valid"),
};
const VALID_KEY: DiagnosticFieldKey = match DiagnosticFieldKey::new("boundary_kind") {
    Some(key) => key,
    None => panic!("test diagnostic key must be valid"),
};
const VALID_TOKEN: StaticDiagnosticToken = match StaticDiagnosticToken::new("value-rejection") {
    Some(token) => token,
    None => panic!("test diagnostic token must be valid"),
};

#[derive(Debug)]
struct TestDiagnostic {
    owned_message: bool,
}

impl fmt::Display for TestDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("test diagnostic")
    }
}

impl std::error::Error for TestDiagnostic {}

impl RuntimeDiagnostic for TestDiagnostic {
    fn diagnostic_code(&self) -> DiagnosticCode {
        VALID_CODE
    }

    fn diagnostic_message(&self) -> Cow<'_, str> {
        if self.owned_message {
            Cow::Owned("owned diagnostic message".to_string())
        } else {
            Cow::Borrowed("borrowed diagnostic message")
        }
    }

    fn record_diagnostic_attributes(&self, attributes: &mut DiagnosticAttributes) {
        let _ = attributes.record(VALID_KEY, VALID_TOKEN.into());
    }
}

#[test]
fn runtime_diagnostic_is_object_safe_and_supports_borrowed_and_owned_messages() {
    fn inspect(diagnostic: &dyn RuntimeDiagnostic) -> Cow<'_, str> {
        assert_eq!(diagnostic.diagnostic_code(), VALID_CODE);
        diagnostic.diagnostic_message()
    }

    let borrowed = TestDiagnostic {
        owned_message: false,
    };
    assert!(matches!(inspect(&borrowed), Cow::Borrowed(_)));

    let owned = TestDiagnostic {
        owned_message: true,
    };
    assert!(matches!(inspect(&owned), Cow::Owned(_)));

    let mut attributes = DiagnosticAttributes::new();
    (&owned as &dyn RuntimeDiagnostic).record_diagnostic_attributes(&mut attributes);
    assert_eq!(
        attributes.iter().next(),
        Some((&VALID_KEY, &DiagnosticFieldValue::StaticToken(VALID_TOKEN)))
    );
}

#[test]
fn token_types_enforce_their_distinct_length_and_grammar_bounds() {
    assert_eq!(VALID_CODE.as_str(), "runtime.boundary-error");
    assert_eq!(VALID_KEY.as_str(), "boundary_kind");
    assert_eq!(VALID_TOKEN.as_str(), "value-rejection");

    assert!(DiagnosticCode::new(CODE_128).is_some());
    assert!(DiagnosticCode::new(CODE_129).is_none());
    assert!(DiagnosticFieldKey::new(TOKEN_64).is_some());
    assert!(DiagnosticFieldKey::new(TOKEN_65).is_none());
    assert!(StaticDiagnosticToken::new(TOKEN_64).is_some());
    assert!(StaticDiagnosticToken::new(TOKEN_65).is_none());

    for invalid in ["", "has space", "has/slash", "has:colon", "caf\u{e9}"] {
        assert!(DiagnosticCode::new(invalid).is_none());
        assert!(DiagnosticFieldKey::new(invalid).is_none());
        assert!(StaticDiagnosticToken::new(invalid).is_none());
    }
}

#[test]
fn diagnostic_field_values_are_closed_and_have_explicit_conversions() {
    let values = [
        DiagnosticFieldValue::from(VALID_TOKEN),
        DiagnosticFieldValue::from(true),
        DiagnosticFieldValue::from(-7_i64),
        DiagnosticFieldValue::from(9_u64),
    ];

    assert_eq!(
        values,
        [
            DiagnosticFieldValue::StaticToken(VALID_TOKEN),
            DiagnosticFieldValue::Bool(true),
            DiagnosticFieldValue::I64(-7),
            DiagnosticFieldValue::U64(9),
        ]
    );
}

#[test]
fn attributes_keep_first_values_and_bound_unique_fields() {
    const KEYS: [&str; MAX_DIAGNOSTIC_ATTRIBUTES + 1] = [
        "key0", "key1", "key2", "key3", "key4", "key5", "key6", "key7", "key8",
    ];

    let mut attributes = DiagnosticAttributes::default();
    assert!(attributes.is_empty());
    assert!(!attributes.was_truncated());

    for (index, key) in KEYS[..MAX_DIAGNOSTIC_ATTRIBUTES].iter().enumerate() {
        assert_eq!(
            attributes.record(
                DiagnosticFieldKey::new(key).expect("test key must be valid"),
                DiagnosticFieldValue::U64(index as u64),
            ),
            DiagnosticAttributeRecordOutcome::Recorded
        );
    }
    assert_eq!(attributes.len(), MAX_DIAGNOSTIC_ATTRIBUTES);

    assert_eq!(
        attributes.record(
            DiagnosticFieldKey::new(KEYS[0]).expect("test key must be valid"),
            DiagnosticFieldValue::U64(99),
        ),
        DiagnosticAttributeRecordOutcome::Duplicate
    );
    assert!(!attributes.was_truncated());
    assert_eq!(
        attributes.iter().next(),
        Some((
            &DiagnosticFieldKey::new(KEYS[0]).expect("test key must be valid"),
            &DiagnosticFieldValue::U64(0),
        ))
    );

    assert_eq!(
        attributes.record(
            DiagnosticFieldKey::new(KEYS[MAX_DIAGNOSTIC_ATTRIBUTES])
                .expect("test key must be valid"),
            DiagnosticFieldValue::Bool(true),
        ),
        DiagnosticAttributeRecordOutcome::Truncated
    );
    assert_eq!(attributes.len(), MAX_DIAGNOSTIC_ATTRIBUTES);
    assert!(attributes.was_truncated());
    assert!(attributes
        .iter()
        .all(|(key, _)| key.as_str() != KEYS[MAX_DIAGNOSTIC_ATTRIBUTES]));
}
