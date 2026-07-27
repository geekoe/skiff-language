use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use skiff_artifact_identity::{
    assign_package_artifact_identities, PackageArtifactRecordPath, PackageFileIrRecordPath,
    PackageResourceRecordPath,
};
use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeNameability, PackageLocalAbiSymbol, PublicationResourceRef,
};
use skiff_compiler_core::json_utils::sha256_hex;
use skiff_compiler_emission::artifact::PublishedResourceArtifact;
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::prelude_registry::prelude_identity;
use skiff_deployment::storage::CanonicalArtifactStore;

use super::*;
use crate::authoring::{build_authoring_object, AuthoringObject};

const EXPECTED_STD_BUILD_ID: &str =
    "skiff-package-build-v10:sha256:b3a2d0e8059cbad6f90c9e9dd48376e1d7c7a9c18de6063a60c2c24b8653a112";
const EXPECTED_PRELUDE_ID: &str =
    "skiff-prelude-v1:sha256:8ec6c2b3f4411b159d8b1b8dd2d55d036713a2533dd3aba043eb3d7fb020c76e";

#[test]
fn official_std_authoring_and_record_writer_are_fixed_and_deterministic() {
    let published = repository_std();
    assert_eq!(published.artifact.package_id, SKIFF_STD_PUBLICATION_ID);
    assert_eq!(published.artifact.package_version, "1.0.0");
    assert_eq!(
        published.artifact.package_build_id.as_str(),
        EXPECTED_STD_BUILD_ID
    );
    assert_eq!(prelude_identity(), EXPECTED_PRELUDE_ID);
    assert!(published
        .artifact
        .package_local_abi
        .public_symbols
        .contains_key("std.websocket.WebSocketConnectRequest"));
    assert!(published
        .artifact
        .package_local_abi
        .public_symbols
        .contains_key("std.websocket.WebSocketConnectionPolicy"));
    assert!(published
        .artifact
        .package_local_abi
        .public_symbols
        .contains_key("std.websocket.WebSocketConnectResult"));
    for removed in [
        "std.websocket.TextConnectionMessage",
        "std.websocket.BinaryConnectionMessage",
        "std.websocket.ConnectionMessage",
        "std.websocket.WebSocketConnection",
        "std.websocket.WebSocketReceiveEvent",
        "std.websocket.WebSocketIngressEvent",
        "std.websocket.WebSocketCloseEvent",
    ] {
        assert!(
            !published
                .artifact
                .package_local_abi
                .public_symbols
                .contains_key(removed),
            "{removed}"
        );
    }
    assert!(published
        .artifact
        .package_local_abi
        .public_symbols
        .contains_key("std.service.InternalError"));
    let PackageLocalAbiSymbol::Callable {
        signature: actor_find,
        ..
    } = &published.artifact.package_local_abi.public_symbols["std.actor.find"]
    else {
        panic!("std.actor.find must remain a callable");
    };
    assert_eq!(actor_find.type_params, ["T", "Id"]);
    let internal_error_entry = &published.package_schema_index.types["std.service.InternalError"];
    assert_eq!(
        internal_error_entry.public_path.as_deref(),
        Some("std.service.InternalError")
    );
    assert_eq!(
        internal_error_entry.nameability,
        ContractTypeNameability::PublicNameable
    );
    let internal_error_record =
        &published.package_schema_type_records[&internal_error_entry.package_schema_type_id];
    let ContractTypeDescriptor::Record { fields } =
        &internal_error_record.canonical_descriptor.descriptor
    else {
        panic!("std.service.InternalError must remain a nominal record");
    };
    assert_eq!(
        fields.keys().map(String::as_str).collect::<Vec<_>>(),
        ["errorId", "message", "traceId"]
    );
    let websocket_file_ir = published
        .file_ir_units
        .iter()
        .find(|file| file.module_path == "std.websocket")
        .expect("official std must emit std.websocket File IR");
    assert!(websocket_file_ir
        .unit
        .declarations
        .types
        .contains_key("WebSocketConnectResult"));

    let first_root = TestDir::new("official-std-records-a");
    let second_root = TestDir::new("official-std-records-b");
    let first_store = CanonicalArtifactStore::create(first_root.path()).unwrap();
    let second_store = CanonicalArtifactStore::create(second_root.path()).unwrap();
    let first = publish_package_artifact_records(&first_store, &published).unwrap();
    let first_bytes = record_bytes(first_root.path());
    let repeated = publish_package_artifact_records(&first_store, &published).unwrap();
    let second = publish_package_artifact_records(&second_store, &published).unwrap();

    assert_eq!(first, repeated);
    assert_eq!(first, second);
    assert_eq!(first_bytes, record_bytes(first_root.path()));
    assert_eq!(first_bytes, record_bytes(second_root.path()));
    assert_eq!(
        first.record_path,
        PackageArtifactRecordPath::new(&first.artifact)
            .unwrap()
            .to_string()
    );
    assert!(!first_root.path().join("pointers").exists());
    assert!(!second_root.path().join("pointers").exists());

    let stored = first_store.read_package_artifact(&first.artifact).unwrap();
    assert_eq!(
        first.file_ir_record_paths,
        stored
            .files
            .iter()
            .map(|file| {
                let path = PackageFileIrRecordPath::new(&first.artifact, file).unwrap();
                assert_eq!(file.artifact_path.as_deref(), Some(path.as_str()));
                path.to_string()
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first.resource_record_paths,
        stored
            .static_resources
            .iter()
            .map(|resource| {
                let path = PackageResourceRecordPath::new(&first.artifact, resource).unwrap();
                assert_eq!(resource.artifact_path.as_deref(), Some(path.as_str()));
                path.to_string()
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn package_record_writer_validates_the_complete_candidate_before_writing() {
    let mut incomplete = repository_std();
    incomplete.file_ir_units.pop();
    let root = TestDir::new("incomplete-record-candidate");
    let store = CanonicalArtifactStore::create(root.path()).unwrap();

    let error = publish_package_artifact_records(&store, &incomplete)
        .unwrap_err()
        .to_string();

    assert!(error.contains("File IR"), "{error}");
    assert!(record_bytes(root.path()).is_empty());
}

#[test]
fn package_record_writer_uses_one_canonical_blob_for_equal_resource_content() {
    let mut published = repository_std();
    let bytes = b"shared canonical resource".to_vec();
    let sha256 = sha256_hex(&bytes);
    for path in ["assets/first.txt", "assets/second.txt"] {
        published
            .artifact
            .static_resources
            .push(PublicationResourceRef {
                path: path.to_string(),
                sha256: sha256.clone(),
                byte_len: bytes.len() as u64,
                content_type: Some("text/plain".to_string()),
                artifact_path: None,
            });
    }
    published.resource_blobs.push(PublishedResourceArtifact {
        logical_path: "assets/first.txt".to_string(),
        artifact_path: format!("resources/sha256/{sha256}"),
        sha256,
        byte_len: bytes.len() as u64,
        bytes,
    });
    assign_package_artifact_identities(&mut published.artifact).unwrap();
    let root = TestDir::new("equal-resource-content");
    let store = CanonicalArtifactStore::create(root.path()).unwrap();

    let receipt = publish_package_artifact_records(&store, &published).unwrap();

    assert_eq!(receipt.resource_record_paths.len(), 2);
    assert_eq!(
        receipt.resource_record_paths[0],
        receipt.resource_record_paths[1]
    );
    let stored = store.read_package_artifact(&receipt.artifact).unwrap();
    assert_eq!(stored.static_resources.len(), 2);
    assert_ne!(
        stored.static_resources[0].path,
        stored.static_resources[1].path
    );
    assert_eq!(
        stored.static_resources[0].artifact_path,
        stored.static_resources[1].artifact_path
    );
}

#[test]
fn copied_std_remains_a_rejected_user_package_with_zero_record_writes() {
    let platform_sources = repository_platform_sources();
    let copied = MinimalPlatformFixture::new("copied-user-std");
    let artifact_root = copied.base().join("artifact-store");

    let error = build_authoring_object(
        &platform_sources,
        AuthoringObject::Package,
        &copied.root().join("std"),
        &artifact_root,
        "dev",
        false,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("package id skiff.run/std is reserved"),
        "{error}"
    );
    assert!(!artifact_root.exists());
}

#[test]
fn official_std_route_fails_closed_on_wrong_platform_manifest_and_source_facts() {
    let changed_registry = MinimalPlatformFixture::new("changed-registry");
    let changed_registry_context = changed_registry.context();
    fs::write(
        changed_registry.root().join("std/registry.yml"),
        "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/not-std\n    path: .\n",
    )
    .unwrap();
    let registry_error =
        author_official_std_package_after_platform_context_guard(&changed_registry_context)
            .unwrap_err()
            .to_string();
    assert!(
        registry_error.contains("std registry can only declare skiff.run/std"),
        "{registry_error}"
    );

    let changed_manifest = MinimalPlatformFixture::new("changed-manifest");
    let changed_manifest_context = changed_manifest.context();
    fs::write(
        changed_manifest.root().join("std/package.yml"),
        "id: example.com/not-std\nversion: 1.0.0\n",
    )
    .unwrap();
    let manifest_error =
        author_official_std_package_after_platform_context_guard(&changed_manifest_context)
            .unwrap_err()
            .to_string();
    assert!(
        manifest_error.contains("standard package id example.com/not-std is not enabled")
            || manifest_error.contains("platform registry grants skiff.run/std"),
        "{manifest_error}"
    );

    #[cfg(unix)]
    {
        let escaped_source = MinimalPlatformFixture::new("escaped-source");
        let escaped_source_context = escaped_source.context();
        let outside = escaped_source.base().join("outside.skiff");
        fs::write(
            &outside,
            "function request() -> string { return \"escaped\" }\n",
        )
        .unwrap();
        fs::remove_file(escaped_source.root().join("std/http.skiff")).unwrap();
        symlink(&outside, escaped_source.root().join("std/http.skiff")).unwrap();
        let source_error =
            author_official_std_package_after_platform_context_guard(&escaped_source_context)
                .unwrap_err()
                .to_string();
        assert!(
            source_error.contains("escapes canonical root"),
            "{source_error}"
        );
    }
}

fn repository_std() -> PublishedPackageArtifact {
    static PUBLISHED: OnceLock<PublishedPackageArtifact> = OnceLock::new();
    PUBLISHED
        .get_or_init(|| {
            author_official_std_package(&repository_platform_sources())
                .expect("repository official std must author")
        })
        .clone()
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    CompilerPlatformSources::new(&root).unwrap()
}

fn record_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut records = BTreeMap::new();
    collect_record_bytes(root, root, &mut records);
    records
}

fn collect_record_bytes(root: &Path, path: &Path, records: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_record_bytes(root, &path, records);
        } else {
            records.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                fs::read(path).unwrap(),
            );
        }
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-p5-f27a-{name}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct MinimalPlatformFixture {
    base: PathBuf,
    root: PathBuf,
}

impl MinimalPlatformFixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "skiff-p5-f27a-platform-{name}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let root = base.join("platform");
        fs::create_dir_all(root.join("std")).unwrap();
        fs::create_dir_all(root.join("prelude")).unwrap();
        fs::write(
            root.join("std/registry.yml"),
            "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/std\n    path: .\n",
        )
        .unwrap();
        fs::write(
            root.join("std/package.yml"),
            "id: skiff.run/std\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(root.join("std/api.yml"), "http:\n  request: http.request\n").unwrap();
        fs::write(
            root.join("std/http.skiff"),
            "function request() -> string { return \"ok\" }\n",
        )
        .unwrap();
        fs::write(root.join("prelude/error.skiff"), "").unwrap();
        Self { base, root }
    }

    fn base(&self) -> &Path {
        &self.base
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn context(&self) -> CompilerPlatformSources {
        CompilerPlatformSources::new(&self.root).unwrap()
    }
}

impl Drop for MinimalPlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
