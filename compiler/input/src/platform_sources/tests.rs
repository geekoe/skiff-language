use std::{fs, path::PathBuf};

use super::*;
use crate::{
    package_config::{
        discover_package_manifests, package_manifest_key, read_user_package_manifest,
    },
    package_sources::read_official_package_sources,
};

#[test]
fn platform_sources_require_absolute_root_and_canonicalize_it() {
    let fixture = PlatformFixture::new("absolute-canonical");
    let relative = CompilerPlatformSources::new(Path::new("relative/platform"));
    assert!(matches!(
        relative,
        Err(CompilerPlatformSourcesError::RootNotAbsolute { .. })
    ));

    let context = fixture.context().unwrap();
    assert!(context.root().is_absolute());
    assert_eq!(context.root(), fixture.root().canonicalize().unwrap());
    assert_eq!(context.std_dir(), context.root().join("std"));
    assert_eq!(context.prelude_dir(), context.root().join("prelude"));
    assert_eq!(
        context.error_projection_catalog_path(),
        context
            .std_dir()
            .join(PLATFORM_ERROR_PROJECTION_CATALOG_FILE)
    );
    assert_eq!(
        context.prelude_error_path(),
        context.prelude_dir().join("error.skiff")
    );
}

#[cfg(unix)]
#[test]
fn platform_sources_accept_absolute_symlink_to_same_canonical_root() {
    use std::os::unix::fs::symlink;

    let fixture = PlatformFixture::new("symlink-root");
    let link = fixture.base().join("platform-link");
    symlink(fixture.root(), &link).unwrap();

    let direct = fixture.context().unwrap();
    let linked = CompilerPlatformSources::new(&link).unwrap();

    assert_eq!(direct, linked);
}

#[test]
fn platform_sources_fail_closed_on_missing_required_files() {
    let missing_registry = PlatformFixture::new("missing-registry");
    fs::remove_file(missing_registry.root().join("std/registry.yml")).unwrap();
    assert!(matches!(
        missing_registry.context(),
        Err(CompilerPlatformSourcesError::Inspect { .. })
    ));

    let missing_error = PlatformFixture::new("missing-prelude-error");
    fs::remove_file(missing_error.root().join("prelude/error.skiff")).unwrap();
    assert!(matches!(
        missing_error.context(),
        Err(CompilerPlatformSourcesError::Inspect { .. })
    ));

    let missing_catalog = PlatformFixture::new("missing-error-projection-catalog");
    fs::remove_file(missing_catalog.root().join("std/error-projections.yml")).unwrap();
    assert!(matches!(
        missing_catalog.context(),
        Err(CompilerPlatformSourcesError::Inspect { .. })
    ));
}

#[cfg(unix)]
#[test]
fn platform_sources_revalidate_error_projection_catalog_provenance() {
    use std::os::unix::fs::symlink;

    let fixture = PlatformFixture::new("catalog-provenance-change");
    let context = fixture.context().unwrap();
    let catalog_path = fixture.root().join("std/error-projections.yml");
    let replacement_path = fixture.root().join("std/replacement-error-projections.yml");
    fs::rename(&catalog_path, &replacement_path).unwrap();
    symlink(&replacement_path, &catalog_path).unwrap();

    let error = context.revalidate().unwrap_err().to_string();
    assert!(error.contains("platform source provenance changed after validation"));
}

#[test]
fn platform_sources_reject_duplicate_and_unknown_registry_input() {
    let duplicate = PlatformFixture::new("duplicate-registry");
    duplicate.write_registry(
        "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/std\n    path: .\n  - id: skiff.run/std\n    path: .\n",
    );
    let error = duplicate.context().unwrap_err().to_string();
    assert!(error.contains("duplicate registry package id skiff.run/std"));

    let unknown = PlatformFixture::new("unknown-registry-field");
    unknown.write_registry(
        "schemaVersion: skiff-std-registry-v1\nunknown: true\npackages:\n  - id: skiff.run/std\n    path: .\n",
    );
    let error = unknown.context().unwrap_err().to_string();
    assert!(error.contains("unknown field `unknown`"));

    let unknown_package = PlatformFixture::new("unknown-registry-package");
    unknown_package.write_registry(
        "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/other\n    path: .\n",
    );
    let error = unknown_package.context().unwrap_err().to_string();
    assert!(error.contains("std registry can only declare skiff.run/std"));
}

#[cfg(unix)]
#[test]
fn platform_sources_reject_registry_member_outside_canonical_root() {
    use std::os::unix::fs::symlink;

    let fixture = PlatformFixture::new("cross-root-registry");
    let outside = fixture.base().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("package.yml"),
        "id: skiff.run/std\nversion: 1.0.0\n",
    )
    .unwrap();
    symlink(&outside, fixture.root().join("std/escape")).unwrap();
    fixture.write_registry(
        "schemaVersion: skiff-std-registry-v1\npackages:\n  - id: skiff.run/std\n    path: escape\n",
    );

    let error = fixture.context().unwrap_err().to_string();
    assert!(error.contains("escapes canonical platform root"));
}

#[cfg(unix)]
#[test]
fn platform_sources_reject_official_source_outside_authorized_package() {
    use std::os::unix::fs::symlink;

    let fixture = PlatformFixture::new("cross-root-source");
    let outside = fixture.base().join("outside.skiff");
    fs::write(&outside, "function escaped() -> string { return \"no\" }\n").unwrap();
    symlink(&outside, fixture.root().join("std/escaped.skiff")).unwrap();
    let context = fixture.context().unwrap();
    let manifests = discover_package_manifests(&context, fixture.root()).unwrap();
    let manifest = &manifests[&package_manifest_key("skiff.run/std", "1.0.0")];

    let error = read_official_package_sources(&context, manifest)
        .unwrap_err()
        .to_string();
    assert!(error.contains("escapes canonical root"));
}

#[cfg(unix)]
#[test]
fn p5_f18a_platform_snapshot_containment() {
    use std::os::unix::fs::symlink;

    for source_root in ["prelude", "std"] {
        let fixture = PlatformFixture::new(&format!("snapshot-outside-{source_root}"));
        let outside = fixture.base().join(format!("outside-{source_root}.skiff"));
        fs::write(&outside, "type EscapedPlatformSource {}\n").unwrap();
        symlink(
            &outside,
            fixture.root().join(source_root).join("escaped.skiff"),
        )
        .unwrap();

        let error = fixture.context().unwrap().prelude_registry_snapshot();
        assert!(matches!(
            error,
            Err(CompilerPlatformSourcesError::InvalidLayout { message })
                if message.contains("escapes canonical platform root")
        ));
    }

    let fixture = PlatformFixture::new("snapshot-same-root");
    symlink(
        fixture.root().join("prelude/error.skiff"),
        fixture.root().join("prelude/error_alias.skiff"),
    )
    .unwrap();
    symlink(
        fixture.root().join("std/http.skiff"),
        fixture.root().join("std/http_alias.skiff"),
    )
    .unwrap();
    fs::write(
        fixture.root().join("std/ignored.test.skiff"),
        "type IgnoredDotTest {}\n",
    )
    .unwrap();
    fs::write(
        fixture.root().join("std/ignored_test.skiff"),
        "type IgnoredUnderscoreTest {}\n",
    )
    .unwrap();

    let snapshot = fixture
        .context()
        .unwrap()
        .prelude_registry_snapshot()
        .unwrap();
    let paths = snapshot
        .sources()
        .iter()
        .map(|(path, _)| path.as_path())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            Path::new("prelude/error.skiff"),
            Path::new("prelude/error_alias.skiff"),
            Path::new("std/http.skiff"),
            Path::new("std/http_alias.skiff"),
        ]
    );
}

#[test]
fn platform_sources_authorize_manifest_and_source_provenance_only() {
    let fixture = PlatformFixture::new("manifest-provenance");
    let context = fixture.context().unwrap();
    let manifests = discover_package_manifests(&context, fixture.root()).unwrap();
    let manifest = &manifests[&package_manifest_key("skiff.run/std", "1.0.0")];

    let sources = read_official_package_sources(&context, manifest).unwrap();
    assert!(sources
        .files()
        .iter()
        .any(|source| source.meta.module_path == "std.http"));
    assert!(sources
        .files()
        .iter()
        .any(|source| source.meta.relative_path == Path::new("error.skiff")));

    let fake_root = fixture.base().join("fake-std");
    fs::create_dir_all(&fake_root).unwrap();
    fs::write(
        fake_root.join("package.yml"),
        "id: skiff.run/std\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(fake_root.join("api.yml"), "{}\n").unwrap();
    let user_error = read_user_package_manifest(&fake_root.join("package.yml"))
        .unwrap_err()
        .to_string();
    assert!(user_error.contains("package id skiff.run/std is reserved"));

    let mut forged = manifest.clone();
    forged.publication.provenance.path = fake_root.join("package.yml");
    let error = read_official_package_sources(&context, &forged)
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not match authorized path"));
}

#[test]
fn platform_sources_read_canonical_prelude_content_in_stable_order() {
    let fixture = PlatformFixture::new("prelude-content");
    fs::write(
        fixture.root().join("prelude/actor.skiff"),
        "type Actor {}\n",
    )
    .unwrap();
    let context = fixture.context().unwrap();

    let snapshot = context.prelude_registry_snapshot().unwrap();
    assert_eq!(
        snapshot
            .sources()
            .iter()
            .map(|(path, _)| path.as_path())
            .collect::<Vec<_>>(),
        vec![
            Path::new("prelude/actor.skiff"),
            Path::new("prelude/error.skiff"),
            Path::new("std/http.skiff"),
        ]
    );
}

#[test]
fn repository_error_projection_catalog_has_exact_policy_entries() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let catalog = CompilerPlatformSources::new(&root)
        .unwrap()
        .read_platform_error_projection_catalog()
        .unwrap();
    let expected = [
        (
            "config.DecodeError",
            "configDecode",
            "runtime.boundary.config",
        ),
        (
            "std.actor.ActivationTimeoutError",
            "actorActivationTimeout",
            "runtime.host.actor",
        ),
        (
            "std.actor.MethodInvocationTimeoutError",
            "actorMethodInvocationTimeout",
            "runtime.host.actor",
        ),
        (
            "std.bytes.DecodeError",
            "bytesDecode",
            "runtime.boundary.bytes",
        ),
        (
            "std.collection.ArrayIndexOutOfBoundsError",
            "collectionArrayIndex",
            "runtime.vm.collection",
        ),
        (
            "std.collection.JsonObjectPropertyNotFoundError",
            "collectionJsonObjectProperty",
            "runtime.vm.collection",
        ),
        (
            "std.collection.MapKeyNotFoundError",
            "collectionMapKey",
            "runtime.vm.collection",
        ),
        ("std.db.ConflictError", "dbConflict", "runtime.serviceDb"),
        (
            "std.db.ConstraintError",
            "dbConstraint",
            "runtime.serviceDb",
        ),
        ("std.db.DecodeError", "dbDecode", "runtime.serviceDb"),
        (
            "std.error.InstructionLimitExceededError",
            "instructionLimit",
            "runtime.vm.executionBudget",
        ),
        (
            "std.error.TimeoutError",
            "lexicalTimeout",
            "runtime.eval.timeout",
        ),
        ("std.file.FileError", "file", "runtime.host.file"),
        ("std.http.HttpError", "http", "runtime.host.http"),
        (
            "std.http.RequestTimeoutError",
            "httpRequestTimeout",
            "runtime.host.http",
        ),
        (
            "std.json.DecodeError",
            "jsonDecode",
            "runtime.boundary.json",
        ),
        (
            "std.number.DecodeError",
            "numberDecode",
            "runtime.boundary.number",
        ),
        (
            "std.service.ProtocolError",
            "serviceProtocol",
            "runtime.eval.service",
        ),
        (
            "std.service.ProviderUnavailableError",
            "serviceProviderUnavailable",
            "runtime.eval.service",
        ),
        (
            "std.time.DecodeError",
            "timeDecode",
            "runtime.boundary.time",
        ),
        (
            "std.websocket.WebSocketRequestError",
            "webSocketRequest",
            "runtime.host.websocket",
        ),
    ];

    assert_eq!(catalog.entries().len(), expected.len());
    for (entry, (key, producer, owner)) in catalog.entries().iter().zip(expected) {
        assert_eq!(entry.projection_key(), key);
        assert!(!entry.projection_key().ends_with(".v1"));
        assert_eq!(entry.producer_family(), producer);
        assert_eq!(entry.semantic_adapter_owner(), owner);
        assert_eq!(entry.public_message_policy(), "semanticAdapterSanitized");
        assert_eq!(entry.envelope_kind(), "platformError");
        assert_eq!(entry.fallback_policy(), "fixedInternalErrorBeforeEnvelope");
    }
}

struct PlatformFixture {
    base: PathBuf,
    root: PathBuf,
}

impl PlatformFixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "skiff-platform-sources-{name}-{}-{}",
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
            root.join("std/error-projections.yml"),
            minimal_error_projection_catalog(),
        )
        .unwrap();
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

    fn context(&self) -> Result<CompilerPlatformSources, CompilerPlatformSourcesError> {
        CompilerPlatformSources::new(&self.root)
    }

    fn write_registry(&self, contents: &str) {
        fs::write(self.root.join("std/registry.yml"), contents).unwrap();
    }
}

fn minimal_error_projection_catalog() -> &'static str {
    "schemaVersion: skiff-platform-error-projection-catalog-v1\nentries:\n  - projectionKey: test.Error\n    producerFamily: test\n    semanticAdapterOwner: runtime.test\n    publicMessagePolicy: semanticAdapterSanitized\n    envelopeKind: platformError\n    fallbackPolicy: fixedInternalErrorBeforeEnvelope\n"
}

impl Drop for PlatformFixture {
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
