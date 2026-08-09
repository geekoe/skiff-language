use std::fmt;

use skiff_artifact_model::BuiltinReceiverOp;

use crate::{IntrinsicIndex, LinkedNativeCallableSignature};

/// Validated static intrinsic key. It is an untrusted registry claim, not an
/// authority token.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkedIntrinsicCanonicalKey(Box<str>);

impl LinkedIntrinsicCanonicalKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, LinkedIntrinsicTargetError> {
        let value = value.into();
        if value.is_empty() {
            return Err(LinkedIntrinsicTargetError::EmptyCanonicalKey);
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_whitespace() || character.is_control())
        {
            return Err(LinkedIntrinsicTargetError::InvalidCanonicalKey {
                value,
                character_index,
            });
        }
        Ok(Self(value.into_boxed_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedStaticIntrinsicTarget {
    canonical_key: LinkedIntrinsicCanonicalKey,
    signature_version: u32,
}

impl LinkedStaticIntrinsicTarget {
    pub fn new(
        canonical_key: LinkedIntrinsicCanonicalKey,
        signature_version: u32,
    ) -> Result<Self, LinkedIntrinsicTargetError> {
        if signature_version == 0 {
            return Err(LinkedIntrinsicTargetError::ZeroSignatureVersion);
        }
        Ok(Self {
            canonical_key,
            signature_version,
        })
    }

    pub const fn canonical_key(&self) -> &LinkedIntrinsicCanonicalKey {
        &self.canonical_key
    }

    pub const fn signature_version(&self) -> u32 {
        self.signature_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedIntrinsicKind {
    Static(LinkedStaticIntrinsicTarget),
    Receiver(BuiltinReceiverOp),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedIntrinsicTarget {
    index: IntrinsicIndex,
    kind: LinkedIntrinsicKind,
    signature: LinkedNativeCallableSignature,
}

impl LinkedIntrinsicTarget {
    pub fn new(
        index: IntrinsicIndex,
        kind: LinkedIntrinsicKind,
        signature: LinkedNativeCallableSignature,
    ) -> Self {
        Self {
            index,
            kind,
            signature,
        }
    }

    pub const fn index(&self) -> IntrinsicIndex {
        self.index
    }

    pub const fn kind(&self) -> &LinkedIntrinsicKind {
        &self.kind
    }

    pub const fn signature(&self) -> &LinkedNativeCallableSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedIntrinsicTargetError {
    EmptyCanonicalKey,
    InvalidCanonicalKey {
        value: String,
        character_index: usize,
    },
    ZeroSignatureVersion,
}

impl fmt::Display for LinkedIntrinsicTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCanonicalKey => {
                formatter.write_str("intrinsic canonical key must not be empty")
            }
            Self::InvalidCanonicalKey {
                value,
                character_index,
            } => write!(
                formatter,
                "intrinsic canonical key {value:?} contains whitespace or a control character at character index {character_index}"
            ),
            Self::ZeroSignatureVersion => {
                formatter.write_str("intrinsic signature version must be non-zero")
            }
        }
    }
}

impl std::error::Error for LinkedIntrinsicTargetError {}
