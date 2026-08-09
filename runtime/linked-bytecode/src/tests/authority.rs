use skiff_artifact_model::{
    host_effect_registry_identity, intrinsic_registry_identity,
    native_value_lifecycle_registry_identity, value_lifecycle_policy_identity,
};

use crate::{
    LinkedBytecodeAuthority, LinkedBytecodeAuthorityField, LinkedBytecodeAuthorityPins,
    LinkedBytecodeAuthorityPinsError,
};

#[derive(Clone, Copy)]
enum AuthorityFixtureField {
    NativeRegistryId,
    NativeVersion,
    NativeFingerprint,
    PolicyVersion,
    PolicyFingerprint,
    HostRegistryId,
    HostVersion,
    HostFingerprint,
    IntrinsicRegistryId,
    IntrinsicVersion,
    IntrinsicFingerprint,
}

const AUTHORITY_FIELDS: [(
    AuthorityFixtureField,
    LinkedBytecodeAuthority,
    LinkedBytecodeAuthorityField,
); 11] = [
    (
        AuthorityFixtureField::NativeRegistryId,
        LinkedBytecodeAuthority::NativeValueLifecycleRegistry,
        LinkedBytecodeAuthorityField::RegistryId,
    ),
    (
        AuthorityFixtureField::NativeVersion,
        LinkedBytecodeAuthority::NativeValueLifecycleRegistry,
        LinkedBytecodeAuthorityField::Version,
    ),
    (
        AuthorityFixtureField::NativeFingerprint,
        LinkedBytecodeAuthority::NativeValueLifecycleRegistry,
        LinkedBytecodeAuthorityField::Fingerprint,
    ),
    (
        AuthorityFixtureField::PolicyVersion,
        LinkedBytecodeAuthority::ValueLifecyclePolicy,
        LinkedBytecodeAuthorityField::Version,
    ),
    (
        AuthorityFixtureField::PolicyFingerprint,
        LinkedBytecodeAuthority::ValueLifecyclePolicy,
        LinkedBytecodeAuthorityField::Fingerprint,
    ),
    (
        AuthorityFixtureField::HostRegistryId,
        LinkedBytecodeAuthority::HostEffectRegistry,
        LinkedBytecodeAuthorityField::RegistryId,
    ),
    (
        AuthorityFixtureField::HostVersion,
        LinkedBytecodeAuthority::HostEffectRegistry,
        LinkedBytecodeAuthorityField::Version,
    ),
    (
        AuthorityFixtureField::HostFingerprint,
        LinkedBytecodeAuthority::HostEffectRegistry,
        LinkedBytecodeAuthorityField::Fingerprint,
    ),
    (
        AuthorityFixtureField::IntrinsicRegistryId,
        LinkedBytecodeAuthority::IntrinsicRegistry,
        LinkedBytecodeAuthorityField::RegistryId,
    ),
    (
        AuthorityFixtureField::IntrinsicVersion,
        LinkedBytecodeAuthority::IntrinsicRegistry,
        LinkedBytecodeAuthorityField::Version,
    ),
    (
        AuthorityFixtureField::IntrinsicFingerprint,
        LinkedBytecodeAuthority::IntrinsicRegistry,
        LinkedBytecodeAuthorityField::Fingerprint,
    ),
];

impl AuthorityFixtureField {
    fn construct_with(
        self,
        value: &str,
    ) -> Result<LinkedBytecodeAuthorityPins, LinkedBytecodeAuthorityPinsError> {
        let mut native = native_value_lifecycle_registry_identity().clone();
        let mut policy = value_lifecycle_policy_identity().clone();
        let mut host = host_effect_registry_identity().clone();
        let mut intrinsic = intrinsic_registry_identity().clone();
        match self {
            Self::NativeRegistryId => native.registry_id = value.to_string(),
            Self::NativeVersion => native.version = value.to_string(),
            Self::NativeFingerprint => native.fingerprint = value.to_string(),
            Self::PolicyVersion => policy.version = value.to_string(),
            Self::PolicyFingerprint => policy.fingerprint = value.to_string(),
            Self::HostRegistryId => host.registry_id = value.to_string(),
            Self::HostVersion => host.version = value.to_string(),
            Self::HostFingerprint => host.fingerprint = value.to_string(),
            Self::IntrinsicRegistryId => intrinsic.registry_id = value.to_string(),
            Self::IntrinsicVersion => intrinsic.version = value.to_string(),
            Self::IntrinsicFingerprint => intrinsic.fingerprint = value.to_string(),
        }
        LinkedBytecodeAuthorityPins::new(native, policy, host, intrinsic)
    }
}

#[test]
fn authority_pins_retain_all_four_exact_identities() {
    let native = native_value_lifecycle_registry_identity().clone();
    let policy = value_lifecycle_policy_identity().clone();
    let host = host_effect_registry_identity().clone();
    let intrinsic = intrinsic_registry_identity().clone();

    let pins = LinkedBytecodeAuthorityPins::new(
        native.clone(),
        policy.clone(),
        host.clone(),
        intrinsic.clone(),
    )
    .expect("canonical authority identities are valid pins");

    assert_eq!(pins.native_value_lifecycle_registry(), &native);
    assert_eq!(pins.value_lifecycle_policy(), &policy);
    assert_eq!(pins.host_effect_registry(), &host);
    assert_eq!(pins.intrinsic_registry(), &intrinsic);
}

#[test]
fn authority_pins_reject_empty_fields() {
    for (fixture_field, authority, field) in AUTHORITY_FIELDS {
        assert_eq!(
            fixture_field
                .construct_with("")
                .expect_err("every authority identity field must be non-empty"),
            LinkedBytecodeAuthorityPinsError::EmptyField { authority, field }
        );
    }
}

#[test]
fn authority_pins_reject_whitespace_and_control_characters() {
    for invalid in [" ", "valid value", "\u{0007}", "valid\u{0007}value"] {
        let character_index = invalid
            .chars()
            .position(|character| character.is_whitespace() || character.is_control())
            .expect("test value contains whitespace or a control character");
        for (fixture_field, authority, field) in AUTHORITY_FIELDS {
            assert_eq!(
                fixture_field
                    .construct_with(invalid)
                    .expect_err("authority identity fields must be canonical text"),
                LinkedBytecodeAuthorityPinsError::InvalidField {
                    authority,
                    field,
                    value: invalid.to_string(),
                    character_index,
                }
            );
        }
    }
}

#[test]
fn authority_error_labels_name_the_exact_authority_and_field() {
    assert_eq!(
        LinkedBytecodeAuthority::NativeValueLifecycleRegistry.name(),
        "native value lifecycle registry"
    );
    assert_eq!(
        LinkedBytecodeAuthority::ValueLifecyclePolicy.name(),
        "value lifecycle policy"
    );
    assert_eq!(
        LinkedBytecodeAuthority::HostEffectRegistry.name(),
        "host effect registry"
    );
    assert_eq!(
        LinkedBytecodeAuthority::IntrinsicRegistry.name(),
        "intrinsic registry"
    );
    assert_eq!(
        LinkedBytecodeAuthorityField::RegistryId.name(),
        "registry id"
    );
    assert_eq!(LinkedBytecodeAuthorityField::Version.name(), "version");
    assert_eq!(
        LinkedBytecodeAuthorityField::Fingerprint.name(),
        "fingerprint"
    );
}
