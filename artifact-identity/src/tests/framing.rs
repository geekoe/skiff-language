use super::*;

#[test]
fn framed_identity_preserves_prefix_and_hash() {
    assert_eq!(
        framed_identity("skiff-example-v1:sha256", "abc123"),
        "skiff-example-v1:sha256:abc123"
    );
}
