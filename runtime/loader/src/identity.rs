use super::{utils::is_sha256_hash, PROTOCOL_IDENTITY_PREFIX};

pub(crate) fn validate_identity_prefix(
    identity: &str,
    expected_prefix: &str,
    label: &str,
) -> anyhow::Result<()> {
    let Some((prefix, hash)) = identity.rsplit_once(":sha256:") else {
        anyhow::bail!("{label} assemblyIdentity must include :sha256:");
    };
    if prefix != expected_prefix {
        anyhow::bail!(
            "{label} assemblyIdentity prefix must be {}, got {}",
            expected_prefix,
            prefix
        );
    }
    if !is_sha256_hash(hash) {
        anyhow::bail!("{label} assemblyIdentity sha256 hash must be 64 lowercase hex characters");
    }
    Ok(())
}

pub(crate) fn identity_hash_with_label<'a>(
    identity: &'a str,
    label: &str,
) -> anyhow::Result<&'a str> {
    let Some((_, hash)) = identity.rsplit_once(":sha256:") else {
        anyhow::bail!("{label} identity must include :sha256:");
    };
    if !is_sha256_hash(hash) {
        anyhow::bail!("{label} identity sha256 hash must be 64 lowercase hex characters");
    }
    Ok(hash)
}

pub(crate) fn identity_hash(identity: &str) -> anyhow::Result<&str> {
    let Some((prefix, hash)) = identity.rsplit_once(":sha256:") else {
        anyhow::bail!("contractIdentity must include :sha256:");
    };
    if prefix != PROTOCOL_IDENTITY_PREFIX {
        anyhow::bail!(
            "contractIdentity prefix must be {}, got {}",
            PROTOCOL_IDENTITY_PREFIX,
            prefix
        );
    }
    if !is_sha256_hash(hash) {
        anyhow::bail!("contractIdentity sha256 hash must be 64 lowercase hex characters");
    }
    Ok(hash)
}
