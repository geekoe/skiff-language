use super::utils::is_sha256_hash;

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
