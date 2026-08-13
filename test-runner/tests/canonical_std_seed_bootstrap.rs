use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use skiff_artifact_identity::{package_artifact_ref, PackageArtifactPointerPath};
use skiff_artifact_model::PackageArtifactRef;
use skiff_compiler::{authoring::author_official_std_package, CompilerPlatformSources};
use skiff_deployment::storage::CanonicalArtifactStore;

fn platform_source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-runner must live directly below the Skiff root")
        .to_path_buf()
}

fn assert_json_keys(value: &serde_json::Value, expected: &[&str]) {
    let mut actual = value
        .as_object()
        .expect("receipt section must be an object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-p5-f27b-bootstrap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_only_seeds_the_exact_std_records_and_pointer_receipt() {
        let root = TestRoot::new();
        let artifacts = root.path().join("artifacts");
        let platform_root = platform_source_root();
        let binary = env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture");
        let run = |profile: &str| {
            let output = Command::new(binary)
                .args([
                    "--bootstrap-only",
                    "--artifact-root",
                    artifacts.to_str().unwrap(),
                    "--profile",
                    profile,
                    "--platform-source-root",
                    platform_root.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
        };

        let first = run("bootstrap-std-a");
        let repeated = run("bootstrap-std-b");
        assert_eq!(first["schemaVersion"], "skiff-package-service-bootstrap-v3");
        assert_json_keys(&first["bootstrap"], &["configSnapshot", "std"]);
        assert_json_keys(
            &first["bootstrap"]["std"],
            &["package", "pointer", "pointerPath"],
        );
        assert_json_keys(
            &first["bootstrap"]["std"]["package"],
            &[
                "artifact",
                "fileIrRecordPaths",
                "recordPath",
                "resourceRecordPaths",
            ],
        );
        assert_eq!(first["bootstrap"]["std"], repeated["bootstrap"]["std"]);

        let platform_sources = CompilerPlatformSources::new(&platform_root).unwrap();
        let authored = author_official_std_package(&platform_sources).unwrap();
        assert!(
            authored.artifact.bytecode.is_none(),
            "official std must remain a source/type-only dependency"
        );
        let expected = package_artifact_ref(&authored.artifact).unwrap();
        assert_eq!(
            first["bootstrap"]["std"]["package"]["artifact"],
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            first["bootstrap"]["std"]["package"]["artifact"],
            first["bootstrap"]["std"]["pointer"]["artifact"]
        );
        assert_eq!(
            first["bootstrap"]["std"]["package"]["recordPath"],
            first["bootstrap"]["std"]["pointer"]["recordPath"]
        );
        assert_eq!(
            first["bootstrap"]["std"]["pointerPath"],
            PackageArtifactPointerPath::new(&expected.package_id, &expected.package_version)
                .unwrap()
                .as_str()
        );

        let artifact: PackageArtifactRef =
            serde_json::from_value(first["bootstrap"]["std"]["package"]["artifact"].clone())
                .unwrap();
        let store = CanonicalArtifactStore::open(&artifacts).unwrap();
        let pointer = store
            .read_package_artifact_pointer(&artifact.package_id, &artifact.package_version)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(pointer).unwrap(),
            first["bootstrap"]["std"]["pointer"]
        );
        let stored = store.read_package_artifact(&artifact).unwrap();
        assert!(
            stored.bytecode.is_none(),
            "bootstrap std PackageArtifact must not carry a bytecode identity"
        );
        let stored_value = serde_json::to_value(stored.as_ref()).unwrap();
        assert!(
            stored_value.get("bytecode").is_none(),
            "bytecode-free bootstrap std record must omit the bytecode field"
        );
        let package_record = artifacts.join(
            first["bootstrap"]["std"]["package"]["recordPath"]
                .as_str()
                .unwrap(),
        );
        assert!(
            !package_record.parent().unwrap().join("bytecode").exists(),
            "bootstrap std seed must not materialize a bytecode record directory"
        );
    }
}
