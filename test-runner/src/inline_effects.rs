use std::path::Path;

use crate::canonical_fixture::CanonicalFixtureError;

const LEGACY_MANIFEST: &str = "skiff.test-doubles.json";

/// The compiler owns inline effect typing, lowering, and setup registration.
/// The runner retains only this explicit legacy-file rejection so an obsolete
/// sidecar can never be silently ignored.
pub(crate) fn reject_legacy_manifest(package_root: &Path) -> Result<(), CanonicalFixtureError> {
    let mut candidates = vec![package_root.join(LEGACY_MANIFEST)];
    if let Some(parent) = package_root.parent() {
        candidates.push(parent.join(LEGACY_MANIFEST));
    }
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "{} is unsupported; move effect doubles into `test ... effects {{ ... }}` and config into Package-ID-root config files",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
