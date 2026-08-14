use std::fmt;

use skiff_artifact_model::{
    validate_platform_error_projection_registry_ref_shape, HostEffectRegistryIdentity,
    IntrinsicRegistryIdentity, NativeValueLifecycleRegistryIdentity,
    PlatformErrorProjectionRegistryRef, PlatformErrorProjectionRegistryRefValidationError,
    ValueLifecyclePolicyIdentity,
};

/// Exact semantic authorities used to interpret one package bytecode image.
///
/// The five identities remain grouped so linked provenance cannot retain the
/// native lifecycle table while accidentally dropping the classifier, host,
/// intrinsic, or platform-error authority pins retained by bytecode schema v10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedBytecodeAuthorityPins {
    native_value_lifecycle_registry: NativeValueLifecycleRegistryIdentity,
    value_lifecycle_policy: ValueLifecyclePolicyIdentity,
    host_effect_registry: HostEffectRegistryIdentity,
    intrinsic_registry: IntrinsicRegistryIdentity,
    platform_error_projection_registry: PlatformErrorProjectionRegistryRef,
}

impl LinkedBytecodeAuthorityPins {
    pub fn new(
        native_value_lifecycle_registry: NativeValueLifecycleRegistryIdentity,
        value_lifecycle_policy: ValueLifecyclePolicyIdentity,
        host_effect_registry: HostEffectRegistryIdentity,
        intrinsic_registry: IntrinsicRegistryIdentity,
        platform_error_projection_registry: PlatformErrorProjectionRegistryRef,
    ) -> Result<Self, LinkedBytecodeAuthorityPinsError> {
        validate_identity_fields(
            LinkedBytecodeAuthority::NativeValueLifecycleRegistry,
            [
                (
                    LinkedBytecodeAuthorityField::RegistryId,
                    native_value_lifecycle_registry.registry_id.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Version,
                    native_value_lifecycle_registry.version.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Fingerprint,
                    native_value_lifecycle_registry.fingerprint.as_str(),
                ),
            ],
        )?;
        validate_identity_fields(
            LinkedBytecodeAuthority::ValueLifecyclePolicy,
            [
                (
                    LinkedBytecodeAuthorityField::Version,
                    value_lifecycle_policy.version.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Fingerprint,
                    value_lifecycle_policy.fingerprint.as_str(),
                ),
            ],
        )?;
        validate_identity_fields(
            LinkedBytecodeAuthority::HostEffectRegistry,
            [
                (
                    LinkedBytecodeAuthorityField::RegistryId,
                    host_effect_registry.registry_id.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Version,
                    host_effect_registry.version.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Fingerprint,
                    host_effect_registry.fingerprint.as_str(),
                ),
            ],
        )?;
        validate_identity_fields(
            LinkedBytecodeAuthority::IntrinsicRegistry,
            [
                (
                    LinkedBytecodeAuthorityField::RegistryId,
                    intrinsic_registry.registry_id.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Version,
                    intrinsic_registry.version.as_str(),
                ),
                (
                    LinkedBytecodeAuthorityField::Fingerprint,
                    intrinsic_registry.fingerprint.as_str(),
                ),
            ],
        )?;
        validate_platform_error_projection_registry_ref_shape(&platform_error_projection_registry)
            .map_err(LinkedBytecodeAuthorityPinsError::from)?;

        Ok(Self {
            native_value_lifecycle_registry,
            value_lifecycle_policy,
            host_effect_registry,
            intrinsic_registry,
            platform_error_projection_registry,
        })
    }

    pub const fn native_value_lifecycle_registry(&self) -> &NativeValueLifecycleRegistryIdentity {
        &self.native_value_lifecycle_registry
    }

    pub const fn value_lifecycle_policy(&self) -> &ValueLifecyclePolicyIdentity {
        &self.value_lifecycle_policy
    }

    pub const fn host_effect_registry(&self) -> &HostEffectRegistryIdentity {
        &self.host_effect_registry
    }

    pub const fn intrinsic_registry(&self) -> &IntrinsicRegistryIdentity {
        &self.intrinsic_registry
    }

    pub const fn platform_error_projection_registry(&self) -> &PlatformErrorProjectionRegistryRef {
        &self.platform_error_projection_registry
    }
}

fn validate_identity_fields<const N: usize>(
    authority: LinkedBytecodeAuthority,
    fields: [(LinkedBytecodeAuthorityField, &str); N],
) -> Result<(), LinkedBytecodeAuthorityPinsError> {
    for (field, value) in fields {
        if value.is_empty() {
            return Err(LinkedBytecodeAuthorityPinsError::EmptyField { authority, field });
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_whitespace() || character.is_control())
        {
            return Err(LinkedBytecodeAuthorityPinsError::InvalidField {
                authority,
                field,
                value: value.to_string(),
                character_index,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedBytecodeAuthority {
    NativeValueLifecycleRegistry,
    ValueLifecyclePolicy,
    HostEffectRegistry,
    IntrinsicRegistry,
    PlatformErrorProjectionRegistry,
}

impl LinkedBytecodeAuthority {
    pub const fn name(self) -> &'static str {
        match self {
            Self::NativeValueLifecycleRegistry => "native value lifecycle registry",
            Self::ValueLifecyclePolicy => "value lifecycle policy",
            Self::HostEffectRegistry => "host effect registry",
            Self::IntrinsicRegistry => "intrinsic registry",
            Self::PlatformErrorProjectionRegistry => "platform error projection registry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedBytecodeAuthorityField {
    RegistryId,
    Version,
    Fingerprint,
}

impl LinkedBytecodeAuthorityField {
    pub const fn name(self) -> &'static str {
        match self {
            Self::RegistryId => "registry id",
            Self::Version => "version",
            Self::Fingerprint => "fingerprint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedBytecodeAuthorityPinsError {
    EmptyField {
        authority: LinkedBytecodeAuthority,
        field: LinkedBytecodeAuthorityField,
    },
    InvalidField {
        authority: LinkedBytecodeAuthority,
        field: LinkedBytecodeAuthorityField,
        value: String,
        character_index: usize,
    },
    InvalidPlatformErrorProjectionRegistry {
        error: PlatformErrorProjectionRegistryRefValidationError,
    },
}

impl From<PlatformErrorProjectionRegistryRefValidationError> for LinkedBytecodeAuthorityPinsError {
    fn from(error: PlatformErrorProjectionRegistryRefValidationError) -> Self {
        Self::InvalidPlatformErrorProjectionRegistry { error }
    }
}

impl fmt::Display for LinkedBytecodeAuthorityPinsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { authority, field } => write!(
                formatter,
                "bytecode {} {} must not be empty",
                authority.name(),
                field.name()
            ),
            Self::InvalidField {
                authority,
                field,
                value,
                character_index,
            } => write!(
                formatter,
                "bytecode {} {} {value:?} contains whitespace or a control character at character index {character_index}",
                authority.name(),
                field.name()
            ),
            Self::InvalidPlatformErrorProjectionRegistry { error } => write!(
                formatter,
                "bytecode platform error projection registry descriptor is invalid: {error}"
            ),
        }
    }
}

impl std::error::Error for LinkedBytecodeAuthorityPinsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPlatformErrorProjectionRegistry { error } => Some(error),
            Self::EmptyField { .. } | Self::InvalidField { .. } => None,
        }
    }
}
