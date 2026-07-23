use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use skiff_artifact_identity::{
    assign_package_artifact_identities, package_artifact_ref, PackageArtifactPointerPath,
};
use skiff_compiler::authoring::{author_official_std_package, publish_package_artifact_records};
use skiff_deployment::storage::{CanonicalArtifactStore, PackageArtifactPointer};

use super::*;

#[test]
fn exact_candidate_is_idempotent_and_receipt_comes_from_the_f27a_writer() {
    let root = TestRoot::new("idempotent");
    let platform_sources = repository_platform_sources();

    let first = seed_canonical_std(&platform_sources, root.path()).unwrap();
    let repeated = seed_canonical_std(&platform_sources, root.path()).unwrap();
    let authored = author_official_std_package(&platform_sources).unwrap();

    assert_eq!(first, repeated);
    assert_eq!(
        first.package.artifact,
        package_artifact_ref(&authored.artifact).unwrap()
    );
    assert_eq!(first.pointer.artifact, first.package.artifact);
    assert_eq!(first.pointer.record_path, first.package.record_path);
    assert_eq!(
        first.pointer_path,
        PackageArtifactPointerPath::new(
            &first.package.artifact.package_id,
            &first.package.artifact.package_version,
        )
        .unwrap()
        .as_str()
    );

    let store = CanonicalArtifactStore::open(root.path()).unwrap();
    assert_eq!(
        store
            .read_package_artifact_pointer(
                &first.package.artifact.package_id,
                &first.package.artifact.package_version,
            )
            .unwrap(),
        Some(first.pointer.clone())
    );
    let stored = store
        .read_package_artifact(&first.package.artifact)
        .unwrap();
    for file in &stored.files {
        store.read_file_ir(&first.package.artifact, file).unwrap();
    }
    for resource in &stored.static_resources {
        store
            .read_static_resource(&first.package.artifact, resource)
            .unwrap();
    }
}

#[test]
fn concurrent_same_candidate_seeds_converge() {
    let root = Arc::new(TestRoot::new("concurrent"));
    let platform_root = repository_platform_root();
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let root = Arc::clone(&root);
            let platform_root = platform_root.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let platform_sources = CompilerPlatformSources::new(&platform_root).unwrap();
                barrier.wait();
                seed_canonical_std(&platform_sources, root.path())
            })
        })
        .collect::<Vec<_>>();

    let receipts = handles
        .into_iter()
        .map(|handle| handle.join().unwrap().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(receipts[0], receipts[1]);
}

#[test]
fn orphan_records_recover_but_same_identity_different_bytes_never_install_pointer() {
    let platform_sources = repository_platform_sources();
    let authored = author_official_std_package(&platform_sources).unwrap();

    let orphan = TestRoot::new("orphan");
    let orphan_store = CanonicalArtifactStore::create(orphan.path()).unwrap();
    let orphan_receipt = publish_package_artifact_records(&orphan_store, &authored).unwrap();
    assert!(orphan_store
        .read_package_artifact_pointer(
            &orphan_receipt.artifact.package_id,
            &orphan_receipt.artifact.package_version,
        )
        .unwrap()
        .is_none());
    let recovered = seed_canonical_std(&platform_sources, orphan.path()).unwrap();
    assert_eq!(recovered.package, orphan_receipt);
    assert_eq!(
        orphan_store
            .read_package_artifact_pointer(
                &recovered.package.artifact.package_id,
                &recovered.package.artifact.package_version,
            )
            .unwrap(),
        Some(recovered.pointer)
    );

    let conflict = TestRoot::new("same-id-different-bytes");
    CanonicalArtifactStore::create(conflict.path()).unwrap();
    let first_file = orphan_receipt
        .file_ir_record_paths
        .first()
        .expect("official std has File IR");
    let conflicting_path = conflict.path().join(first_file);
    fs::create_dir_all(conflicting_path.parent().unwrap()).unwrap();
    let mut bytes = fs::read(orphan.path().join(first_file)).unwrap();
    bytes.push(b'\n');
    fs::write(&conflicting_path, bytes).unwrap();

    let error = seed_canonical_std(&platform_sources, conflict.path())
        .unwrap_err()
        .to_string();

    assert!(error.contains("immutable record conflict"), "{error}");
    assert!(CanonicalArtifactStore::open(conflict.path())
        .unwrap()
        .read_package_artifact_pointer(
            &orphan_receipt.artifact.package_id,
            &orphan_receipt.artifact.package_version,
        )
        .unwrap()
        .is_none());
}

#[test]
fn malformed_dangling_and_different_existing_pointers_fail_before_store_writes() {
    let platform_sources = repository_platform_sources();
    let expected = author_official_std_package(&platform_sources).unwrap();
    let expected_ref = package_artifact_ref(&expected.artifact).unwrap();

    let malformed = TestRoot::new("malformed-pointer");
    CanonicalArtifactStore::create(malformed.path()).unwrap();
    let malformed_path = pointer_path(
        malformed.path(),
        &expected_ref.package_id,
        &expected_ref.package_version,
    );
    fs::create_dir_all(malformed_path.parent().unwrap()).unwrap();
    fs::write(&malformed_path, b"{").unwrap();
    let before = tree_bytes(malformed.path());
    assert!(seed_canonical_std(&platform_sources, malformed.path()).is_err());
    assert_eq!(tree_bytes(malformed.path()), before);

    let source = TestRoot::new("dangling-pointer-source");
    let source_receipt = seed_canonical_std(&platform_sources, source.path()).unwrap();
    let dangling = TestRoot::new("dangling-pointer");
    CanonicalArtifactStore::create(dangling.path()).unwrap();
    let dangling_path = pointer_path(
        dangling.path(),
        &expected_ref.package_id,
        &expected_ref.package_version,
    );
    fs::create_dir_all(dangling_path.parent().unwrap()).unwrap();
    fs::copy(
        source.path().join(source_receipt.pointer_path),
        &dangling_path,
    )
    .unwrap();
    let before = tree_bytes(dangling.path());
    assert!(seed_canonical_std(&platform_sources, dangling.path()).is_err());
    assert_eq!(tree_bytes(dangling.path()), before);

    let different = TestRoot::new("different-pointer");
    let different_store = CanonicalArtifactStore::create(different.path()).unwrap();
    let mut alternative = expected;
    alternative
        .artifact
        .package_local_abi
        .public_symbols
        .pop_first()
        .expect("official std has a public symbol");
    assign_package_artifact_identities(&mut alternative.artifact).unwrap();
    let alternative_receipt =
        publish_package_artifact_records(&different_store, &alternative).unwrap();
    let alternative_pointer =
        PackageArtifactPointer::new(alternative_receipt.artifact.clone()).unwrap();
    different_store
        .compare_and_swap_package_artifact_pointer(None, &alternative_pointer)
        .unwrap();
    let before = tree_bytes(different.path());
    let error = seed_canonical_std(&platform_sources, different.path())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("canonical std pointer already selects"),
        "{error}"
    );
    assert_eq!(tree_bytes(different.path()), before);
}

fn repository_platform_sources() -> CompilerPlatformSources {
    CompilerPlatformSources::new(&repository_platform_root()).unwrap()
}

fn repository_platform_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
}

fn pointer_path(root: &Path, package_id: &str, package_version: &str) -> PathBuf {
    root.join(
        PackageArtifactPointerPath::new(package_id, package_version)
            .unwrap()
            .as_relative_path()
            .as_path(),
    )
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, output);
            } else {
                output.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    collect(root, root, &mut output);
    output
}

struct TestRoot {
    path: PathBuf,
}

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-p5-f27b-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
