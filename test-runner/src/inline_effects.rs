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
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;

    #[test]
    fn legacy_manifest_is_rejected_without_parsing_it() {
        let root = std::env::temp_dir().join(format!(
            "skiff-inline-effects-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp root");
        fs::write(root.join(LEGACY_MANIFEST), b"not json").expect("legacy file");
        let error = reject_legacy_manifest(&root).expect_err("legacy file must fail");
        assert!(error.to_string().contains("unsupported"));
        fs::remove_dir_all(root).expect("cleanup");
    }
}
