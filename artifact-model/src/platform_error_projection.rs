//! Compiler-generated platform error projection registry surface.
//!
//! The generated module owns the closed keys, descriptors, and registry
//! reference validation. Its concrete fields stay private so callers can only
//! consume generated authorities or descriptors admitted through strict
//! deserialization.

mod generated;

pub use generated::{
    assert_platform_error_projection_generated_surface,
    current_platform_error_projection_registry_ref, platform_error_projection_descriptor,
    platform_error_projection_descriptor_by_key, platform_error_projection_registry,
    validate_current_platform_error_projection_registry_ref,
    validate_platform_error_projection_registry_ref_shape, PlatformErrorProjectionDescriptor,
    PlatformErrorProjectionKey, PlatformErrorProjectionRegistryRef,
    PlatformErrorProjectionRegistryRefValidationError, UnknownPlatformErrorProjectionKey,
    PLATFORM_ERROR_PROJECTION_CODEC_VERSION, PLATFORM_ERROR_PROJECTION_REGISTRY_FINGERPRINT,
    PLATFORM_ERROR_PROJECTION_REGISTRY_ID, PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn first_registry_is_the_exact_ascii_sorted_unversioned_public_symbol_set() {
        let expected = [
            "config.DecodeError",
            "std.actor.ActivationTimeoutError",
            "std.actor.MethodInvocationTimeoutError",
            "std.bytes.DecodeError",
            "std.collection.ArrayIndexOutOfBoundsError",
            "std.collection.JsonObjectPropertyNotFoundError",
            "std.collection.MapKeyNotFoundError",
            "std.db.ConflictError",
            "std.db.ConstraintError",
            "std.db.DecodeError",
            "std.error.InstructionLimitExceededError",
            "std.error.TimeoutError",
            "std.file.FileError",
            "std.http.HttpError",
            "std.http.RequestTimeoutError",
            "std.json.DecodeError",
            "std.number.DecodeError",
            "std.service.ProtocolError",
            "std.service.ProviderUnavailableError",
            "std.time.DecodeError",
            "std.websocket.WebSocketRequestError",
        ];
        let actual = platform_error_projection_registry()
            .iter()
            .map(|descriptor| descriptor.key().as_str())
            .collect::<Vec<_>>();

        assert_eq!(actual.as_slice(), expected.as_slice());
        assert!(actual.windows(2).all(|pair| pair[0] < pair[1]));
        for descriptor in platform_error_projection_registry() {
            let key = descriptor.key().as_str();
            assert_eq!(key, descriptor.nominal_identity());
            let final_segment = key.rsplit('.').next().expect("non-empty generated key");
            assert!(!final_segment.bytes().all(|byte| byte.is_ascii_digit()));
            assert!(!final_segment.strip_prefix('v').is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            }));
        }
    }

    #[test]
    fn registry_ref_serde_admits_only_general_shape_and_exact_validation_is_separate() {
        let historical_fingerprint = format!("sha256:{}", "0".repeat(64));
        let historical: PlatformErrorProjectionRegistryRef = serde_json::from_value(json!({
            "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
            "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
            "fingerprint": historical_fingerprint,
        }))
        .expect("same registry/version historical fingerprint has valid general shape");

        validate_platform_error_projection_registry_ref_shape(&historical).unwrap();
        assert_eq!(
            validate_current_platform_error_projection_registry_ref(&historical),
            Err(PlatformErrorProjectionRegistryRefValidationError::CurrentFingerprintMismatch)
        );
        let current = current_platform_error_projection_registry_ref();
        validate_current_platform_error_projection_registry_ref(current).unwrap();
        assert!(std::ptr::eq(
            current,
            current_platform_error_projection_registry_ref()
        ));

        let valid_fingerprint = format!("sha256:{}", "0".repeat(64));
        for invalid in [
            json!({
                "registryId": "other",
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": valid_fingerprint,
            }),
            json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION + 1,
                "fingerprint": valid_fingerprint,
            }),
            json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": format!("sha256:{}", "A".repeat(64)),
            }),
            json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": valid_fingerprint,
                "extra": true,
            }),
            json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
            }),
        ] {
            assert!(serde_json::from_value::<PlatformErrorProjectionRegistryRef>(invalid).is_err());
        }
    }
}
