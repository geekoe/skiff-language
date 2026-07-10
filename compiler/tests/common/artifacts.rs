#![allow(dead_code)]

use std::{fs, path::Path};

use skiff_compiler::{
    build_service_publication, collect_source_tree, read_service_config, BuiltServicePublication,
    PackageResolutionDirs, PublishedArtifactPayload, PublishedArtifactVisitOptions,
    PublishedFileIrArtifact, PublishedJsonArtifact, ServicePublicationBuildInput,
};

pub fn build_temp_service_publication(root: &Path) -> BuiltServicePublication {
    let config = read_service_config(root).unwrap();
    let source_tree = collect_source_tree(root).unwrap();
    build_service_publication(ServicePublicationBuildInput {
        config: &config,
        source_tree: &source_tree,
        package_dirs: test_package_dirs(root),
        ..ServicePublicationBuildInput::new(&config, &source_tree)
    })
    .unwrap()
}

pub fn build_temp_service_publication_without_package_dirs(root: &Path) -> BuiltServicePublication {
    let config = read_service_config(root).unwrap();
    let source_tree = collect_source_tree(root).unwrap();
    build_service_publication(ServicePublicationBuildInput::new(&config, &source_tree)).unwrap()
}

pub fn assert_publish_error_contains(root: &Path, fragments: &[&str]) {
    let config = read_service_config(root).unwrap();
    let source_tree = collect_source_tree(root).unwrap();
    let error = build_service_publication(ServicePublicationBuildInput {
        config: &config,
        source_tree: &source_tree,
        package_dirs: test_package_dirs(root),
        ..ServicePublicationBuildInput::new(&config, &source_tree)
    })
    .unwrap_err();
    let message = error.to_string();

    for fragment in fragments {
        assert!(
            message.contains(fragment),
            "expected error to contain {fragment:?}, got:\n{message}"
        );
    }
}

pub fn assert_publish_error_contains_without_package_dirs(root: &Path, fragments: &[&str]) {
    let message = publish_error_message_without_package_dirs(root);

    for fragment in fragments {
        assert!(
            message.contains(fragment),
            "expected error to contain {fragment:?}, got:\n{message}"
        );
    }
}

pub fn publish_error_message_without_package_dirs(root: &Path) -> String {
    let config = read_service_config(root).unwrap();
    let source_tree = collect_source_tree(root).unwrap();
    let error = build_service_publication(ServicePublicationBuildInput::new(&config, &source_tree))
        .unwrap_err();
    error.to_string()
}

fn test_package_dirs(root: &Path) -> PackageResolutionDirs {
    let store = root.join(".skiff-packages");
    PackageResolutionDirs {
        package_dirs: store.is_dir().then_some(store).into_iter().collect(),
    }
}

pub fn source_artifact<'a>(
    published: &'a BuiltServicePublication,
    source_path: &str,
) -> &'a PublishedFileIrArtifact {
    published
        .artifacts
        .file_ir_units
        .iter()
        .find(|artifact| artifact.source_path == source_path)
        .expect("source artifact should be published")
}

pub fn package_source_artifact<'a>(
    published: &'a BuiltServicePublication,
    source_path: &str,
) -> &'a PublishedFileIrArtifact {
    published
        .artifacts
        .package_file_ir_units
        .iter()
        .find(|artifact| artifact.source_path == source_path)
        .expect("package source artifact should be published")
}

pub fn package_assembly<'a>(
    published: &'a BuiltServicePublication,
    package_id: &str,
) -> &'a PublishedJsonArtifact {
    published
        .artifacts
        .package_assemblies
        .iter()
        .find(|artifact| artifact.value["package"]["id"] == package_id)
        .expect("package assembly should be published")
}

pub fn service_assembly_value(published: &BuiltServicePublication) -> &serde_json::Value {
    &published.artifacts.service_assembly.value
}

pub fn service_package<'a>(
    published: &'a BuiltServicePublication,
    package_id: &str,
) -> &'a serde_json::Value {
    published
        .artifacts
        .service_unit
        .value
        .get("packageDependencies")
        .and_then(serde_json::Value::as_array)
        .and_then(|dependencies| {
            dependencies
                .iter()
                .find(|dependency| dependency["id"] == package_id)
        })
        .or_else(|| {
            service_assembly_value(published)
                .get("packageConfigs")
                .and_then(|configs| configs.get(package_id))
        })
        .unwrap_or_else(|| panic!("service should include package {package_id}"))
}

pub fn assert_service_package_id(published: &BuiltServicePublication, package_id: &str) {
    assert_eq!(service_package(published, package_id)["id"], package_id);
}

pub fn assert_service_package_absent(published: &BuiltServicePublication, package_id: &str) {
    assert!(
        published
            .artifacts
            .service_unit
            .value
            .get("packageDependencies")
            .and_then(serde_json::Value::as_array)
            .is_none_or(|dependencies| dependencies
                .iter()
                .all(|dependency| dependency["id"] != package_id)),
        "Service Unit should not include package {package_id}"
    );
}

pub fn assert_config_uses(published: &BuiltServicePublication, expected: serde_json::Value) {
    assert_eq!(service_assembly_value(published)["configUses"], expected);
}

pub fn assert_file_ir_contains_package_symbol(
    published: &BuiltServicePublication,
    module_path: &str,
    dependency_ref: &str,
    symbol_path: &str,
) {
    let artifact = published
        .artifacts
        .file_ir_units
        .iter()
        .find(|artifact| artifact.module_path == module_path)
        .unwrap_or_else(|| panic!("file IR unit {module_path} should be published"));
    let artifact_value = artifact.value();
    assert!(
        json_contains_package_symbol(&artifact_value, dependency_ref, symbol_path),
        "file IR unit {module_path} should contain package symbol {dependency_ref}:{symbol_path}: {artifact_value}",
    );
}

pub fn write_test_artifact_root(root: &Path, published: &BuiltServicePublication) {
    published
        .artifacts
        .try_visit_artifacts(
            PublishedArtifactVisitOptions {
                include_contract_schema: false,
            },
            |artifact| {
                match artifact.payload {
                    PublishedArtifactPayload::Json(value) => {
                        write_json(&root.join(&artifact.path), &value)
                    }
                    PublishedArtifactPayload::Bytes(bytes) => {
                        write_bytes(&root.join(&artifact.path), &bytes)
                    }
                }
                Ok::<_, std::convert::Infallible>(())
            },
        )
        .expect("test artifact visitor should not fail");
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

pub fn json_contains_package_symbol(
    value: &serde_json::Value,
    dependency_ref: &str,
    symbol_path: &str,
) -> bool {
    if package_symbol_json(value, dependency_ref, symbol_path) {
        return true;
    }
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_package_symbol(value, dependency_ref, symbol_path)),
        serde_json::Value::Object(object) => object
            .values()
            .any(|value| json_contains_package_symbol(value, dependency_ref, symbol_path)),
        _ => false,
    }
}

fn package_symbol_json(value: &serde_json::Value, dependency_ref: &str, symbol_path: &str) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("kind").and_then(serde_json::Value::as_str) != Some("packageSymbol") {
        return false;
    }
    let Some(operation) = object
        .get("operation")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    if operation
        .get("publicPath")
        .and_then(serde_json::Value::as_str)
        != Some(symbol_path)
    {
        return false;
    }
    let Some(package) = object
        .get("packageRef")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    package.get("kind").and_then(serde_json::Value::as_str) == Some("dependency")
        && package
            .get("dependencyRef")
            .and_then(serde_json::Value::as_str)
            == Some(dependency_ref)
}

fn write_json(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}
