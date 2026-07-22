use std::{fs, path::PathBuf};

use super::*;
use crate::{
    package_config::{discover_package_manifests, package_manifest_key},
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
        fs::write(
            root.join("prelude/error.skiff"),
            "native type ErrorPayload\n",
        )
        .unwrap();
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
