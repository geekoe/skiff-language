use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use skiff_artifact_identity::{
    assign_package_unit_identities, package_unit_content_hash, publication_storage_segment,
};
use skiff_artifact_model::PackageUnit;

use super::ArtifactGraphLoader;
use crate::{ArtifactGraphCache, FileIrCache, PackageCache};

#[test]
fn package_unit_path_hash_uses_raw_json_even_when_typed_defaults_are_equal() {
    let root = TempArtifactRoot::new("raw-package-unit-hash");
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    assign_package_unit_identities(&mut unit).expect("package identities");

    let mut original = serde_json::to_value(&unit).expect("package unit JSON");
    original
        .as_object_mut()
        .expect("package unit object")
        .insert("resources".to_string(), Value::Array(Vec::new()));
    let original_hash = package_unit_content_hash(&original).expect("raw package unit hash");
    let package_segment =
        publication_storage_segment(&unit.package_id, "package id").expect("package segment");
    let unit_path = format!("units/packages/{package_segment}/{original_hash}.json");

    let mut tampered = original.clone();
    tampered
        .as_object_mut()
        .expect("package unit object")
        .remove("resources");
    let original_typed: PackageUnit =
        serde_json::from_value(original).expect("original typed package unit");
    let tampered_typed: PackageUnit =
        serde_json::from_value(tampered.clone()).expect("tampered typed package unit");
    assert_eq!(
        original_typed, tampered_typed,
        "removing a default-valued field must preserve the typed PackageUnit fixture",
    );

    write_json(root.path(), &unit_path, &tampered);
    let file_cache = FileIrCache::new();
    let package_cache = PackageCache::new();
    let loader = ArtifactGraphLoader::new(
        root.path(),
        ArtifactGraphCache::new(&file_cache, &package_cache),
    );
    let error = loader
        .load_package_unit_at_path(Path::new(&unit_path))
        .expect_err("raw content hash mismatch must fail closed");
    assert!(
        error
            .to_string()
            .contains("artifact path validation failed"),
        "unexpected error: {error:#}",
    );
}

fn write_json(root: &Path, relative_path: &str, value: &Value) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().expect("artifact parent")).expect("artifact directory");
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("artifact JSON"),
    )
    .expect("write artifact");
}

struct TempArtifactRoot {
    path: std::path::PathBuf,
}

impl TempArtifactRoot {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-loader-{label}-{}-{nanos}",
            std::process::id(),
        ));
        fs::create_dir_all(&path).expect("temp artifact root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempArtifactRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
