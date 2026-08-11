use std::mem::size_of;

use super::{
    PlatformErrorProjectionPayload, StdFileFileErrorPayload, ValidatedKnownPlatformErrorProjection,
};

#[test]
fn validated_evidence_boxes_the_generated_payload_union() {
    assert_eq!(
        size_of::<ValidatedKnownPlatformErrorProjection>(),
        size_of::<Box<PlatformErrorProjectionPayload>>()
    );
    assert!(
        size_of::<ValidatedKnownPlatformErrorProjection>()
            < size_of::<PlatformErrorProjectionPayload>()
    );

    let payload = PlatformErrorProjectionPayload::StdFileFileError(StdFileFileErrorPayload {
        message: "safe".to_owned(),
    });
    let validated = ValidatedKnownPlatformErrorProjection::new(payload.clone());
    assert_eq!(validated.payload(), &payload);
}
