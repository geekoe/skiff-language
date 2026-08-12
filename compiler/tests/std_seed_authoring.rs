//! Formal-tooling std seed (`skiff stack init` production bootstrap producer).
//!
//! The internal `std-seed` compiler action authors the compiler-owned official
//! std candidate through `author_official_std_package` +
//! `publish_package_artifact_records`, then installs the exact PackageArtifact
//! pointer with the same idempotent fail-closed semantics as the test-runner
//! canonical seed. These tests drive the shared `seed_official_std_package`
//! entrypoint and verify exact receipt/pointer/record materialization without
//! invoking the smoke fixture.

#[cfg(test)]
mod tests {

    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use skiff_artifact_identity::{package_artifact_ref, PackageArtifactPointerPath};
    use skiff_compiler::{
        authoring::{
            author_official_std_package, author_official_std_package_with_bytecode,
            seed_official_std_package,
        },
        CompilerPlatformSources,
    };
    use skiff_deployment::storage::CanonicalArtifactStore;

    #[test]
    fn fresh_seed_is_idempotent_and_materializes_exact_std_records() {
        let root = TestRoot::new("std-seed-idempotent");
        let platform_sources = repository_platform_sources();

        let first = seed_official_std_package(&platform_sources, root.path()).unwrap();
        let repeated = seed_official_std_package(&platform_sources, root.path()).unwrap();
        assert_eq!(first, repeated);

        let authored = author_official_std_package_with_bytecode(&platform_sources)
            .unwrap()
            .0;
        let exact = package_artifact_ref(&authored.artifact).unwrap();
        assert_eq!(
            first["package"]["artifact"],
            serde_json::to_value(&exact).unwrap()
        );
        assert_eq!(first["pointer"]["artifact"], first["package"]["artifact"]);
        assert_eq!(
            first["pointer"]["recordPath"],
            first["package"]["recordPath"]
        );
        let package_id = first["package"]["artifact"]["packageId"].as_str().unwrap();
        let package_version = first["package"]["artifact"]["packageVersion"]
            .as_str()
            .unwrap();
        let expected_pointer_path =
            PackageArtifactPointerPath::new(package_id, package_version).unwrap();
        assert_eq!(
            first["pointerPath"],
            serde_json::json!(expected_pointer_path.as_str())
        );

        let store = CanonicalArtifactStore::open(root.path()).unwrap();
        let pointer = store
            .read_package_artifact_pointer(&exact.package_id, &exact.package_version)
            .unwrap()
            .expect("std pointer must be installed");
        assert_eq!(
            pointer.artifact, exact,
            "installed pointer must select the exact std candidate"
        );
        let stored = store.read_package_artifact(&pointer.artifact).unwrap();
        for file in &stored.files {
            store.read_file_ir(&pointer.artifact, file).unwrap();
        }
        for resource in &stored.static_resources {
            store
                .read_static_resource(&pointer.artifact, resource)
                .unwrap();
        }
    }

    #[test]
    fn malformed_existing_pointer_fails_closed_before_store_writes() {
        let platform_sources = repository_platform_sources();
        let authored = author_official_std_package(&platform_sources).unwrap();
        let exact = package_artifact_ref(&authored.artifact).unwrap();

        let root = TestRoot::new("std-seed-malformed-pointer");
        CanonicalArtifactStore::create(root.path()).unwrap();
        let pointer_path = pointer_path(root.path(), &exact.package_id, &exact.package_version);
        fs::create_dir_all(pointer_path.parent().unwrap()).unwrap();
        fs::write(&pointer_path, b"{").unwrap();

        let before = tree_bytes(root.path());
        let error = seed_official_std_package(&platform_sources, root.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("strict JSON parse failed"), "{error}");
        assert_eq!(tree_bytes(root.path()), before);
    }

    #[test]
    fn different_existing_pointer_fails_closed_before_store_writes() {
        let platform_sources = repository_platform_sources();
        let expected = author_official_std_package(&platform_sources).unwrap();

        let conflict = TestRoot::new("std-seed-different-pointer");
        let conflict_store = CanonicalArtifactStore::create(conflict.path()).unwrap();
        let mut alternative = expected;
        let first_public_symbol = alternative
            .artifact
            .package_local_abi
            .public_symbols
            .keys()
            .next()
            .cloned()
            .expect("std exports at least one public symbol");
        let removed_symbol = alternative
            .artifact
            .package_local_abi
            .public_symbols
            .remove(&first_public_symbol)
            .expect("selected public symbol must exist");
        if let skiff_artifact_model::PackageLocalAbiSymbol::Callable { callable_id, .. } =
            removed_symbol
        {
            alternative.artifact.callable_links.remove(&callable_id);
            alternative
                .artifact
                .callable_semantic_facts
                .remove(&callable_id);
            alternative
                .artifact
                .boundary_projections
                .remove(&callable_id);
            alternative
                .artifact
                .implementation_links
                .functions
                .remove(&first_public_symbol);
        }
        skiff_artifact_identity::assign_package_artifact_identities(&mut alternative.artifact)
            .unwrap();
        let alternative_receipt = skiff_compiler::authoring::publish_package_artifact_records(
            conflict_store.root(),
            &alternative,
        )
        .unwrap();
        let alternative_pointer =
            skiff_deployment::storage::PackageArtifactPointer::new(alternative_receipt.artifact)
                .unwrap();
        conflict_store
            .compare_and_swap_package_artifact_pointer(None, &alternative_pointer)
            .unwrap();

        let before = tree_bytes(conflict.path());
        let error = seed_official_std_package(&platform_sources, conflict.path())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("canonical std pointer already selects"),
            "{error}"
        );
        assert_eq!(tree_bytes(conflict.path()), before);
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
                "skiff-stack-cmd-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
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
}
