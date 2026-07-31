use super::canonical_case_identity;

#[test]
fn canonical_identity_does_not_duplicate_the_display_name() {
    assert_eq!(
        canonical_case_identity("internal.http.__test", 2),
        "internal.http.__test::test[2]"
    );
}
