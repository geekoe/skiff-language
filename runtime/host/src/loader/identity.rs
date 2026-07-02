use super::utils::is_sha256_hash;

pub(super) fn validate_identity_alias(
    alias: &str,
    expected: &str,
    label: &str,
) -> anyhow::Result<()> {
    if alias != expected {
        anyhow::bail!("{label} {alias} does not match serviceAssembly assemblyIdentity {expected}");
    }
    Ok(())
}

pub(super) fn validate_identity_prefix(
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

pub(super) fn identity_hash_with_label<'a>(
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
