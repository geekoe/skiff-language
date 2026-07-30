use std::{
    fs,
    path::{Path, PathBuf},
};

use super::*;
use crate::{
    package_config::{
        discover_package_manifests, package_manifest_key, read_user_package_manifest,
    },
    CompilerPlatformSources,
};

#[test]
fn official_package_sources_use_std_namespace_and_explicit_private_namespace() {
    let fixture = PlatformFixture::new("official-private-namespace");
    fs::write(
        fixture.std_dir().join("http.skiff"),
        r#"function request() -> string { return "ok" }"#,
    )
    .unwrap();
    fs::write(
        fixture.std_dir().join("helper.skiff"),
        r#"
            type HelperState { value: string }
            function helper() -> HelperState { return { value: "internal" } }
        "#,
    )
    .unwrap();
    fs::create_dir_all(fixture.std_dir().join("__private")).unwrap();
    fs::write(
        fixture.std_dir().join("__private").join("secret.skiff"),
        r#"function secret() -> string { return "internal" }"#,
    )
    .unwrap();

    let context = CompilerPlatformSources::new(fixture.root()).unwrap();
    let manifests = discover_package_manifests(&context, fixture.root()).unwrap();
    let manifest = &manifests[&package_manifest_key("skiff.run/std", "1.0.0")];
    let sources = read_official_package_sources(&context, manifest).unwrap();
    let module_paths = sources
        .files()
        .iter()
        .map(|source| source.meta.module_path.as_str())
        .collect::<Vec<_>>();

    assert!(module_paths.contains(&"std.http"));
    assert!(module_paths.contains(&"std.helper"));
    assert!(module_paths.contains(&"std.__private.secret"));
    assert!(module_paths
        .iter()
        .all(|module_path| !module_path.contains('/')));
    assert!(!module_paths
        .iter()
        .any(|module_path| module_path.starts_with("skiff.run/std")));
}

#[test]
fn official_package_source_module_path_normalizes_std_prefixes() {
    assert_eq!(
        official_package_source_module_path("skiff.run/std", "helper"),
        "std.helper"
    );
    assert_eq!(
        official_package_source_module_path("skiff.run/std", "std.helper"),
        "std.helper"
    );
    assert_eq!(
        official_package_source_module_path("skiff.run/std", "__private.helper"),
        "std.__private.helper"
    );
    assert_eq!(
        official_package_source_module_path("skiff.run/std", "std.__private.helper"),
        "std.__private.helper"
    );
}

#[test]
fn package_source_validation_remains_fail_closed_without_origin_metadata() {
    let source_path = PathBuf::from("source.skiff");
    let files = vec![CompilerRawSourceFile {
        meta: RawSourceFileMeta {
            relative_path: source_path.clone(),
            module_path: "source".to_string(),
            is_test_file: false,
            is_generated: false,
        },
        text: String::new(),
        role: CompilerSourceRole::Package,
    }];

    let missing_visibility =
        validate_package_publication_sources(&files, &BTreeMap::new()).unwrap_err();
    assert!(missing_visibility
        .to_string()
        .contains("source.skiff has no package visibility"));

    let visibility_by_path = BTreeMap::from([(
        PathBuf::from("missing.skiff"),
        PackageSourceVisibility::Export {
            public_module_path: String::new(),
        },
    )]);
    let incomplete_sources =
        validate_package_publication_sources(&[], &visibility_by_path).unwrap_err();
    let incomplete_sources = incomplete_sources.to_string();
    assert!(incomplete_sources.contains("missing.skiff has package visibility but no raw source"));
    assert!(incomplete_sources.contains("missing.skiff has empty public module path"));
}

#[test]
fn user_package_sources_exclude_tests_by_default_and_explicit_test_reader_preserves_them() {
    let fixture = PlatformFixture::new("user-test-sources");
    let package_root = fixture.root().join("widget");
    fs::create_dir_all(package_root.join("internal")).unwrap();
    fs::write(
        package_root.join("package.yml"),
        "id: example.com/widget-tests\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(package_root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        package_root.join("internal/widget.skiff"),
        "function helper() -> number { return 1 }\n",
    )
    .unwrap();
    fs::write(
        package_root.join("internal/widget.test.skiff"),
        "test \"helper\" { assert true }\n",
    )
    .unwrap();

    let manifest = read_user_package_manifest(&package_root.join("package.yml")).unwrap();
    let production_sources = read_package_sources(&manifest, &package_root).unwrap();
    assert_eq!(production_sources.files().len(), 1);
    assert!(production_sources
        .files()
        .iter()
        .all(|source| !source.meta.is_test_file));
    assert!(production_sources
        .source_tree()
        .sources
        .iter()
        .all(|source| !source.is_test_file));

    let sources = read_test_service_package_sources(&manifest, &package_root).unwrap();
    let production = sources
        .files()
        .iter()
        .find(|source| source.meta.relative_path == Path::new("internal/widget.skiff"))
        .expect("production source is retained");
    let test = sources
        .files()
        .iter()
        .find(|source| source.meta.relative_path == Path::new("internal/widget.test.skiff"))
        .expect("test source is retained");

    assert_eq!(production.meta.module_path, "internal.widget");
    assert_eq!(test.meta.module_path, "internal.widget");
    assert!(!production.meta.is_test_file);
    assert!(test.meta.is_test_file);

    let source_tree = sources.source_tree();
    let test_tree_entry = source_tree
        .sources
        .iter()
        .find(|source| source.file_path == Path::new("internal/widget.test.skiff"))
        .expect("source tree preserves test source");
    assert_eq!(test_tree_entry.module_path, "internal.widget");
    assert!(test_tree_entry.is_test_file);
}

#[test]
fn production_source_rejects_reserved_test_module_segment() {
    let fixture = PlatformFixture::new("reserved-test-module");
    let package_root = fixture.root().join("reserved");
    fs::create_dir_all(package_root.join("internal/__test")).unwrap();
    fs::write(
        package_root.join("package.yml"),
        "id: example.com/reserved-test-module\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(package_root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        package_root.join("internal/__test/helper.skiff"),
        "function helper() -> number { return 1 }\n",
    )
    .unwrap();

    let manifest = read_user_package_manifest(&package_root.join("package.yml")).unwrap();
    let error = read_package_sources(&manifest, &package_root)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains(
            "production source internal/__test/helper.skiff uses reserved compiler test module segment __test"
        ),
        "{error}"
    );
}

struct PlatformFixture {
    root: PathBuf,
}

impl PlatformFixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "skiff-package-sources-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
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
        fs::write(root.join("prelude/error.skiff"), "").unwrap();
        Self { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn std_dir(&self) -> PathBuf {
        self.root.join("std")
    }
}

impl Drop for PlatformFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
