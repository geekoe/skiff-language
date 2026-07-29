use crate::{ArtifactIdentityError, Result};

pub fn publication_storage_segment(value: &str, label: &str) -> Result<String> {
    validate_publication_id(value, label)?;
    Ok(value.replace('.', "~").replace('/', "~~"))
}

fn validate_publication_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > 63 || value == "std" {
        return Err(invalid_publication_id(value, label));
    }
    if value != value.trim()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        return Err(invalid_publication_id(value, label));
    }

    let Some((authority, local)) = value.split_once('/') else {
        return Err(invalid_publication_id(value, label));
    };
    let authority_labels = authority.split('.').collect::<Vec<_>>();
    if authority_labels.len() < 2
        || authority_labels
            .iter()
            .any(|item| !is_valid_authority_label(item))
        || local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        return Err(invalid_publication_id(value, label));
    }
    Ok(())
}

fn invalid_publication_id(value: &str, label: &str) -> ArtifactIdentityError {
    ArtifactIdentityError::InvalidPublicationId {
        label: label.to_string(),
        value: value.to_string(),
    }
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}
