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
