use std::{fmt, str::FromStr};

use thiserror::Error;

pub const STD_SOURCE_ALIAS: &str = "std";
pub const SKIFF_STD_PUBLICATION_ID: &str = "skiff.run/std";
pub const MAX_PUBLICATION_ID_BYTES: usize = 63;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicationId {
    canonical: String,
}

impl PublicationId {
    pub fn parse(value: &str) -> Result<Self, PublicationIdError> {
        if value.is_empty() {
            return Err(PublicationIdError::Empty);
        }
        if value.len() > MAX_PUBLICATION_ID_BYTES {
            return Err(PublicationIdError::TooLong {
                length: value.len(),
            });
        }
        if value != value.trim() || value.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(PublicationIdError::UnsafePathForm);
        }
        if value.contains('\\')
            || value.contains("://")
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains("//")
            || value.contains('~')
        {
            return Err(PublicationIdError::UnsafePathForm);
        }

        if value == STD_SOURCE_ALIAS {
            return Err(PublicationIdError::InvalidLocalSegment {
                segment: value.to_string(),
            });
        }

        let Some((authority, path)) = value.split_once('/') else {
            return Err(PublicationIdError::UnsafePathForm);
        };
        validate_authority(authority)?;
        validate_local_path(path)?;

        Ok(Self {
            canonical: value.to_string(),
        })
    }

    pub fn skiff_std() -> Self {
        Self {
            canonical: SKIFF_STD_PUBLICATION_ID.to_string(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    pub fn into_string(self) -> String {
        self.canonical
    }

    pub fn storage_safe_component(&self) -> String {
        self.canonical.replace('.', "~").replace('/', "~~")
    }

    pub fn runtime_target_component(&self) -> String {
        self.storage_safe_component()
    }

    pub fn artifact_path(&self) -> String {
        self.storage_safe_component()
    }

    pub fn dotted_public_prefix(&self) -> String {
        self.canonical.replace('/', ".")
    }

    pub fn authority(&self) -> Option<&str> {
        if let Some((authority, _)) = self.canonical.split_once('/') {
            Some(authority)
        } else if self.canonical.contains('.') {
            Some(&self.canonical)
        } else {
            None
        }
    }

    pub fn display_form(&self) -> String {
        self.canonical.clone()
    }

    pub fn from_display_form(value: &str) -> Result<Self, PublicationIdError> {
        Self::parse(value)
    }
}

impl fmt::Display for PublicationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical)
    }
}

impl FromStr for PublicationId {
    type Err = PublicationIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PublicationIdError {
    #[error("publication id must not be empty")]
    Empty,
    #[error("publication id must be at most 63 bytes, got {length}")]
    TooLong { length: usize },
    #[error("publication id contains an empty path segment")]
    EmptyPathSegment,
    #[error("publication id contains an invalid local segment `{segment}`")]
    InvalidLocalSegment { segment: String },
    #[error("publication id contains an invalid authority `{authority}`")]
    InvalidAuthority { authority: String },
    #[error("publication id uses an unsafe path-like form")]
    UnsafePathForm,
}

fn validate_authority(authority: &str) -> Result<(), PublicationIdError> {
    if authority.is_empty() {
        return Err(PublicationIdError::InvalidAuthority {
            authority: authority.to_string(),
        });
    }
    let mut label_count = 0;
    for label in authority.split('.') {
        label_count += 1;
        if !is_valid_authority_label(label) {
            return Err(PublicationIdError::InvalidAuthority {
                authority: authority.to_string(),
            });
        }
    }
    if label_count < 2 {
        return Err(PublicationIdError::InvalidAuthority {
            authority: authority.to_string(),
        });
    }
    Ok(())
}

fn validate_local_path(local: &str) -> Result<(), PublicationIdError> {
    if local.is_empty() {
        return Err(PublicationIdError::EmptyPathSegment);
    }
    for segment in local.split('/') {
        validate_local_segment(segment)?;
    }
    Ok(())
}

fn validate_local_segment(segment: &str) -> Result<(), PublicationIdError> {
    if is_valid_local_segment(segment) {
        return Ok(());
    }
    if segment.is_empty() {
        return Err(PublicationIdError::EmptyPathSegment);
    }
    Err(PublicationIdError::InvalidLocalSegment {
        segment: segment.to_string(),
    })
}

fn is_valid_authority_label(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if bytes[0] == b'-' || bytes.last() == Some(&b'-') {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() || bytes.last() == Some(&b'-') {
        return false;
    }
    bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
    })
}
