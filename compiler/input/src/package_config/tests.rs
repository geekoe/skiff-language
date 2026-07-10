use std::{
    env, fs,
    path::Path,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

#[test]
fn std_registry_rejects_non_std_package_entries() {
    let temp = temp_dir("std-registry-mismatch");
    let std_dir = temp.join("std");
    let other_dir = temp.join("other");
    fs::create_dir_all(&std_dir).unwrap();
    fs::create_dir_all(&other_dir).unwrap();
    fs::write(
        std_dir.join("registry.yml"),
        r#"
schemaVersion: skiff-std-registry-v1
packages:
  - id: other
    path: ../other
"#,
    )
    .unwrap();
    fs::write(other_dir.join("package.yml"), "id: other\nversion: 1.0.0\n").unwrap();

    let error = discover_builtin_std_registry_manifests(&std_dir, &std_dir.join("registry.yml"))
        .unwrap_err()
        .to_string();

    assert!(error.contains("std registry package other is invalid"));
    assert!(error.contains("std registry can only declare skiff.run/std"));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn rejects_package_dependency_missing_version() {
    let error = read_temp_manifest(
        "missing-version",
        r#"
id: example.com/app
version: 1.0.0
packages:
  - id: skiff.run/std
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("packages entry requires id and version"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_complex_package_dependencies_and_rejects_explicit_std_dependency() {
    let error = read_temp_manifest(
        "explicit-std-dependency",
        r#"
id: example.com/app
version: 1.0.0
packages:
  - id: skiff.run/std
    version: 1.0.0
    alias: std
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("platform std is built into the compiler"),
        "unexpected error: {error}"
    );

    let manifest = read_temp_manifest(
        "complex-example",
        r#"
id: example.com/app
version: 1.2.3-beta.1+build.2
packages:
  - id: skiff.run/example
    version: 1.0.0
    alias: example
"#,
    )
    .unwrap();

    assert_eq!(manifest.dependencies[0].id, "skiff.run/example");
    assert_eq!(manifest.dependencies[0].version, "1.0.0");
    assert_eq!(manifest.dependencies[0].effective_alias(), "example");
    assert_eq!(manifest.version, "1.2.3-beta.1+build.2");
}

#[test]
fn rejects_canonical_std_package_dependency_with_std_alias() {
    let error = read_temp_manifest(
        "canonical-std-dependency",
        r#"
id: example.com/app
version: 1.0.0
packages:
  - id: skiff.run/std
    version: 1.0.0
    alias: std
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("platform std is built into the compiler"),
        "unexpected error: {error}"
    );
}

#[test]
fn accepts_canonical_std_package_manifest_id() {
    let temp = temp_dir("canonical-std-manifest");
    let manifest_path = temp.join("package.yml");
    fs::write(
        &manifest_path,
        r#"
id: skiff.run/std
version: 1.0.0
"#,
    )
    .unwrap();

    let manifest = manifest_io::read_package_manifest(
        &manifest_path,
        manifest_validation::PackageManifestOwner::CompilerStandardPackage,
    )
    .unwrap();

    assert_eq!(manifest.id.as_str(), "skiff.run/std");
    assert_eq!(
        manifest.provenance.owner,
        crate::ManifestOwner::CompilerStandardPackage
    );
    assert!(!manifest.provenance.synthetic);
    let _ = fs::remove_dir_all(temp);
}

#[test]
fn rejects_package_manifest_top_level_api() {
    let error = read_temp_manifest(
        "api-removed",
        r#"
id: example.com/app
version: 1.0.0
api:
  - path: ""
    file: api
  - { path: storage, file: storage_api }
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("api has been removed; declare public API in api.yml"),
        "unexpected error: {error}"
    );
}

#[test]
fn reads_package_api_yml_entries() {
    let temp = temp_dir("api-yml");
    let manifest_path = temp.join("package.yml");
    fs::write(
        &manifest_path,
        r#"
id: example.com/app
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        temp.join("api.yml"),
        r#"
http:
  Request: http.HttpRequest
decode: codec.decode
"#,
    )
    .unwrap();

    let manifest = read_user_package_manifest(&manifest_path).unwrap();
    let entries = manifest.api.entries().collect::<Vec<_>>();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].public_path_string(), "http.Request");
    assert_eq!(entries[0].source_module_hint(), "http");
    assert_eq!(entries[0].source_symbol(), "HttpRequest");
    assert_eq!(entries[1].public_path_string(), "decode");
    assert!(manifest.api.source.is_some());
    let _ = fs::remove_dir_all(temp);
}

#[test]
fn rejects_package_api_yml_dotted_public_key() {
    let error = read_temp_manifest_with_api_yml(
        "api-yml-dotted-key",
        r#"
id: skiff.run/account
version: 1.0.0
"#,
        r#"
skiff.run.account: api.Account
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("dotted public keys are not supported"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_api_yml_invalid_selector() {
    let error = read_temp_manifest_with_api_yml(
        "api-yml-invalid-selector",
        r#"
id: example.com/app
version: 1.0.0
"#,
        r#"
storage: Storage
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("api.yml selector for public path storage is invalid")
            && error.contains("module.path.Symbol"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_api_yml_unknown_key_shape() {
    let error = read_temp_manifest_with_api_yml(
        "api-yml-unknown-key-shape",
        r#"
id: example.com/app
version: 1.0.0
"#,
        r#"
1: api.Symbol
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("api.yml key under <root> must be an identifier segment"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_api_yml_invalid_leaf_shape() {
    let error = read_temp_manifest_with_api_yml(
        "api-yml-invalid-leaf-shape",
        r#"
id: example.com/app
version: 1.0.0
"#,
        r#"
chat:
  send: ["chat.send"]
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains(
            "api.yml public path chat.send must map to a string source selector or nested mapping"
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_api_yml_non_mapping_root() {
    let error = read_temp_manifest_with_api_yml(
        "api-yml-non-mapping-root",
        r#"
id: example.com/app
version: 1.0.0
"#,
        "[]\n",
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("api.yml root must be a mapping"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_package_exports() {
    let error = read_temp_manifest(
        "legacy-exports",
        r#"
id: example.com/app
version: 1.0.0
exports:
  - api
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("exports has been removed"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_package_dependencies_packages() {
    let error = read_temp_manifest(
        "legacy-dependencies-packages",
        r#"
id: example.com/app
version: 1.0.0
dependencies:
  packages:
    - id: skiff.run/std
      version: 1.0.0
      alias: std
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("dependencies.packages has been removed; use top-level packages"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_binding_requirements() {
    let error = read_temp_manifest(
        "binding-requirements",
        r#"
id: example.com/app
version: 1.0.0
requires:
  bindings:
    - alias: managedLlm
      interface: llm.ManagedLlmService
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains(
            "requires.bindings has been removed; pass any interface values as package entry parameters"
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_legacy_package_requires_bindings_shapes() {
    for (name, yaml, expected) in [
        (
            "legacy-bindings",
            r#"
id: example.com/app
version: 1.0.0
requires:
  bindings:
    - interface: llm.ManagedLlmService
"#,
            "requires.bindings has been removed",
        ),
        (
            "legacy-bindings-mapping",
            r#"
id: example.com/app
version: 1.0.0
requires:
  bindings:
    managedLlm: llm.ManagedLlmService
"#,
            "requires.bindings has been removed",
        ),
    ] {
        let error = read_temp_manifest(name, yaml).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_legacy_package_requires_services_and_dependencies_services() {
    for (name, yaml, expected) in [
        (
            "requires-services",
            r#"
id: example.com/app
version: 1.0.0
requires:
  services:
    - alias: account
"#,
            "requires.services has been removed",
        ),
        (
            "dependencies-services",
            r#"
id: example.com/app
version: 1.0.0
dependencies:
  services:
    - id: skiff.run/account
      version: 0.1.0
      alias: account
"#,
            "dependencies.services has been removed",
        ),
    ] {
        let error = read_temp_manifest(name, yaml).unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_package_manifest_versions_that_are_not_safe_artifact_segments() {
    for (name, version) in [
        ("blank-version", " "),
        ("slash-version", "1.0/evil"),
        ("backslash-version", r"1.0\evil"),
        ("dot-version", "."),
        ("dotdot-version", ".."),
        ("whitespace-version", "1.0.0 beta"),
    ] {
        let error = read_temp_manifest(
            name,
            &format!(
                r#"
id: example.com/app
version: '{version}'
"#
            ),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("version cannot be empty")
                || error.contains("package version")
                    && error.contains("must be safe for package artifact paths"),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_package_dependency_versions_that_are_not_safe_artifact_segments() {
    for (name, version) in [
        ("slash-version", "1.0/evil"),
        ("backslash-version", r"1.0\evil"),
        ("dot-version", "."),
        ("dotdot-version", ".."),
        ("whitespace-version", "1.0.0 beta"),
    ] {
        let error = read_temp_manifest(
            name,
            &format!(
                r#"
id: example.com/app
version: 1.0.0
packages:
  - id: skiff.run/example
    version: '{version}'
    alias: example
"#
            ),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("packages entry skiff.run/example version")
                && error.contains("must be safe for package artifact paths"),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_package_ids_that_are_not_safe_artifact_paths() {
    for (name, id, expected) in [
        (
            "backslash-id",
            r"example.com\app",
            "must be a publication id",
        ),
        (
            "dotdot-id",
            "example.com/../app",
            "must be a publication id",
        ),
        (
            "empty-path-segment",
            "example.com//app",
            "must be a publication id",
        ),
    ] {
        let error = read_temp_manifest(
            name,
            &format!(
                r#"
id: '{id}'
version: 1.0.0
"#
            ),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(expected),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_package_dependency_config_objects() {
    let error = read_temp_manifest(
        "dependency-config",
        r#"
id: example.com/app
version: 1.0.0
packages:
  - id: skiff.run/http-session
    version: 1.0.0
    alias: httpSession
    config:
      cookieName: session
      cookieDomain: .example.test
      maxAgeSeconds: 2592000
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("unknown field `config`"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_package_dependency_config_non_object() {
    let error = read_temp_manifest(
        "dependency-config-array",
        r#"
id: example.com/app
version: 1.0.0
packages:
  - id: example.com/session
    version: 0.1.0
    config: []
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("unknown field `config`"),
        "unexpected error: {error}"
    );
}

#[test]
fn accepts_authority_path_package_ids_with_hyphenated_path_segments() {
    let manifest = read_temp_manifest(
        "camel-path",
        r#"
id: skiff.run/http-session
version: 1.0.0
"#,
    )
    .unwrap();

    assert_eq!(manifest.id.as_str(), "skiff.run/http-session");
}

#[test]
fn discovers_multiple_versions_for_same_package_id() {
    let temp = temp_dir("multi-version-store");
    let store = temp.join("store");
    write_package_store_manifest(&store, "skiff.run/llm", "1.0.0");
    write_package_store_manifest(&store, "skiff.run/llm", "2.0.0");

    let manifests = discover_package_manifests_with_dependency_dirs(
        &temp,
        &PackageResolutionDirs {
            package_dirs: vec![store],
        },
        &[
            package_dependency("skiff.run/llm", "1.0.0"),
            package_dependency("skiff.run/llm", "2.0.0"),
        ],
    )
    .unwrap();

    assert!(manifests.contains_key(&("skiff.run/llm".to_string(), "1.0.0".to_string())));
    assert!(manifests.contains_key(&("skiff.run/llm".to_string(), "2.0.0".to_string())));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn discovers_package_manifest_from_concrete_package_root() {
    let temp = temp_dir("current-package");
    write_package_manifest(&temp, "skiff.run/llm", "1.0.0");

    let manifests = discover_package_manifests(&temp).unwrap();

    assert!(manifests.contains_key(&("skiff.run/llm".to_string(), "1.0.0".to_string())));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn discovers_package_store_version_path() {
    let temp = temp_dir("package-store-version-path");
    let service = temp.join("service");
    let store = temp.join("store");
    write_package_store_manifest(&store, "skiff.run/llm", "1.0.0");
    write_package_store_manifest(&store, "skiff.run/llm", "2.0.0");
    fs::create_dir_all(&service).unwrap();

    let manifests = discover_package_manifests_with_dependency_dirs(
        &service,
        &PackageResolutionDirs {
            package_dirs: vec![store],
        },
        &[
            package_dependency("skiff.run/llm", "1.0.0"),
            package_dependency("skiff.run/llm", "2.0.0"),
        ],
    )
    .unwrap();

    assert!(manifests.contains_key(&("skiff.run/llm".to_string(), "1.0.0".to_string())));
    assert!(manifests.contains_key(&("skiff.run/llm".to_string(), "2.0.0".to_string())));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn does_not_scan_unrequested_package_store_entries() {
    let temp = temp_dir("package-store-exact-lookup");
    let service = temp.join("service");
    let store = temp.join("store");
    write_package_store_manifest(&store, "skiff.run/llm", "1.0.0");
    fs::create_dir_all(&service).unwrap();

    let manifests = discover_package_manifests_with_dependency_dirs(
        &service,
        &PackageResolutionDirs {
            package_dirs: vec![store],
        },
        &[],
    )
    .unwrap();

    assert!(!manifests.contains_key(&("skiff.run/llm".to_string(), "1.0.0".to_string())));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn package_dir_is_not_treated_as_a_single_dependency_package() {
    let temp = temp_dir("package-dir-not-single-root");
    let service = temp.join("service");
    let package_root = temp.join("llm");
    write_package_manifest(&package_root, "skiff.run/llm", "1.0.0");
    fs::create_dir_all(&service).unwrap();

    let manifests = discover_package_manifests_with_dependency_dirs(
        &service,
        &PackageResolutionDirs {
            package_dirs: vec![package_root],
        },
        &[package_dependency("skiff.run/llm", "1.0.0")],
    )
    .unwrap();

    assert!(!manifests.contains_key(&("skiff.run/llm".to_string(), "1.0.0".to_string())));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn does_not_implicitly_discover_ancestor_package_stores() {
    let temp = temp_dir("no-implicit-ancestor-store");
    let service = temp.join("service");
    let store = temp.join(".skiff-packages");
    fs::create_dir_all(&service).unwrap();
    write_package_store_manifest(&store, "skiff.run/llm", "1.0.0");

    let manifests = discover_package_manifests_with_dependency_dirs(
        &service,
        &PackageResolutionDirs {
            package_dirs: Vec::new(),
        },
        &[package_dependency("skiff.run/llm", "1.0.0")],
    )
    .unwrap();

    assert!(!manifests.contains_key(&("skiff.run/llm".to_string(), "1.0.0".to_string())));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn ordered_package_dirs_shadow_lower_priority_duplicates() {
    let temp = temp_dir("ordered-store-shadow");
    let first = temp.join("first-store");
    let second = temp.join("second-store");
    write_package_store_manifest(&first, "skiff.run/llm", "1.0.0");
    write_package_store_manifest(&second, "skiff.run/llm", "1.0.0");

    let manifests = discover_package_manifests_with_dependency_dirs(
        &temp,
        &PackageResolutionDirs {
            package_dirs: vec![first.clone(), second],
        },
        &[package_dependency("skiff.run/llm", "1.0.0")],
    )
    .unwrap();

    let manifest = manifests
        .get(&("skiff.run/llm".to_string(), "1.0.0".to_string()))
        .expect("dependency manifest should be loaded");
    assert!(manifest.provenance.path.starts_with(first));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn resolves_direct_dependency_by_exact_package_id_and_version() {
    let temp = temp_dir("exact-version-selection");
    let store = temp.join("store");
    write_package_store_manifest(&store, "skiff.run/llm", "1.0.0");
    write_package_store_manifest(&store, "skiff.run/llm", "2.0.0");
    let dependency = package_dependency("skiff.run/llm", "2.0.0");
    let available = discover_package_manifests_with_dependency_dirs(
        &temp,
        &PackageResolutionDirs {
            package_dirs: vec![store],
        },
        std::slice::from_ref(&dependency),
    )
    .unwrap();

    let resolved = resolve_package_imports(&[], &[dependency], &available).unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].manifest.id.as_str(), "skiff.run/llm");
    assert_eq!(resolved[0].manifest.version, "2.0.0");

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn rejects_complex_package_dependency_without_alias() {
    let error = read_temp_manifest(
        "complex-missing-alias",
        r#"
id: example.com/app
version: 1.0.0
packages:
  - id: skiff.run/example
    version: 1.0.0
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("packages entry skiff.run/example requires alias"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_package_manifest_resources() {
    let manifest = read_temp_manifest(
        "resources",
        r#"
id: example.com/app
version: 1.0.0
resources:
  - prompts/system.md
  - schemas/tool_input.schema.json
"#,
    )
    .unwrap();

    assert_eq!(
        manifest
            .resources
            .iter()
            .map(|resource| resource.path.as_str())
            .collect::<Vec<_>>(),
        vec!["prompts/system.md", "schemas/tool_input.schema.json"]
    );
}

#[test]
fn rejects_package_manifest_invalid_resources() {
    let error = read_temp_manifest(
        "invalid-resources",
        r#"
id: example.com/app
version: 1.0.0
resources:
  - ./prompts/system.md
  - prompts/system.md
  - prompts/system.md
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("resources[0] ./prompts/system.md is invalid"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("resources[2] prompts/system.md is declared more than once"),
        "unexpected error: {error}"
    );
}

fn read_temp_manifest(name: &str, text: &str) -> Result<PackageManifest, PackageConfigError> {
    let temp = temp_dir(name);
    let manifest_path = temp.join("package.yml");
    fs::write(&manifest_path, text).unwrap();
    let result = read_user_package_manifest(&manifest_path);
    let _ = fs::remove_dir_all(temp);
    result
}

fn read_temp_manifest_with_api_yml(
    name: &str,
    manifest_text: &str,
    api_yml_text: &str,
) -> Result<PackageManifest, PackageConfigError> {
    let temp = temp_dir(name);
    let manifest_path = temp.join("package.yml");
    fs::write(&manifest_path, manifest_text).unwrap();
    fs::write(temp.join("api.yml"), api_yml_text).unwrap();
    let result = read_user_package_manifest(&manifest_path);
    let _ = fs::remove_dir_all(temp);
    result
}

fn write_package_manifest(dir: &Path, id: &str, version: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("package.yml"),
        format!("id: {id}\nversion: {version}\n"),
    )
    .unwrap();
}

fn write_package_store_manifest(store: &Path, id: &str, version: &str) {
    let id_dir = skiff_compiler_core::id::PublicationId::parse(id)
        .expect("test package id should be valid")
        .artifact_path();
    write_package_manifest(&store.join(id_dir).join(version), id, version);
}

fn package_dependency(id: &str, version: &str) -> PackageDependency {
    PackageDependency {
        id: id.to_string(),
        version: version.to_string(),
        alias: Some("pkg".to_string()),
        config: empty_dependency_config(),
        collection_name_mapping: BTreeMap::new(),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = env::temp_dir().join(format!(
        "skiff-package-config-{name}-{}-{unique}",
        process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
