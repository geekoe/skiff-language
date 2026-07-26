use std::fmt;
use std::str::FromStr;

use serde::{de, Deserialize, Deserializer, Serialize};

macro_rules! string_identity {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

// These identities are intentionally distinct Rust types even though their
// artifact representation is a framed string. Package schema nominal equality
// is owned by the declaring package and is distinct from package-local ABI.
string_identity!(PackageSchemaTypeId);
string_identity!(PackageSchemaIndexIdentity);
string_identity!(ContractOperationId);
string_identity!(ServiceProtocolIdentity);
string_identity!(PackageBuildId);
string_identity!(PackageLocalAbiIdentity);
string_identity!(PackageCallableId);
string_identity!(DeploymentRevision);
string_identity!(DeploymentArtifactIdentity);
string_identity!(AssemblyIdentity);

pub const GATEWAY_ENTRY_IDENTITY_PREFIX: &str = "skiff-gateway-entry-v1:sha256";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayEntryKeyParseError {
    value: String,
}

impl GatewayEntryKeyParseError {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for GatewayEntryKeyParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "gateway entry key {:?} must be non-empty and contain no whitespace or control characters",
            self.value
        )
    }
}

impl std::error::Error for GatewayEntryKeyParseError {}

/// A stable, service-owner-local opaque key.
///
/// It is deliberately not a content identity and therefore has no generation
/// prefix. Its lexical validation only protects artifact framing; consumers
/// must not infer routing or source semantics from its contents.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GatewayEntryKey(String);

impl GatewayEntryKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayEntryKeyParseError> {
        let value = value.into();
        if value.is_empty()
            || value
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(GatewayEntryKeyParseError::new(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for GatewayEntryKey {
    type Error = GatewayEntryKeyParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for GatewayEntryKey {
    type Error = GatewayEntryKeyParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for GatewayEntryKey {
    type Err = GatewayEntryKeyParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for GatewayEntryKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

impl AsRef<str> for GatewayEntryKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for GatewayEntryKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayEntryIdentityParseError {
    value: String,
}

impl GatewayEntryIdentityParseError {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for GatewayEntryIdentityParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "gateway entry identity {:?} must use {GATEWAY_ENTRY_IDENTITY_PREFIX}:<64 lowercase hex>",
            self.value
        )
    }
}

impl std::error::Error for GatewayEntryIdentityParseError {}

/// Content identity for a normalized external gateway protocol surface.
///
/// Construction is intentionally parse-only. The canonical producer lives in
/// `skiff-artifact-identity`; this type merely prevents unvalidated artifact
/// strings and owner-local keys from being used interchangeably.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GatewayEntryIdentity(String);

impl GatewayEntryIdentity {
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayEntryIdentityParseError> {
        let value = value.into();
        let valid = value
            .strip_prefix(GATEWAY_ENTRY_IDENTITY_PREFIX)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            });
        if !valid {
            return Err(GatewayEntryIdentityParseError::new(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for GatewayEntryIdentity {
    type Error = GatewayEntryIdentityParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for GatewayEntryIdentity {
    type Error = GatewayEntryIdentityParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for GatewayEntryIdentity {
    type Err = GatewayEntryIdentityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for GatewayEntryIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

impl AsRef<str> for GatewayEntryIdentity {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for GatewayEntryIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
