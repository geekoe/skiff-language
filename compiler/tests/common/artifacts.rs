#![allow(dead_code)]

use std::{fs, path::Path};

use skiff_compiler::PublishedPackageArtifact;
use skiff_compiler_emission::{PublishedFileIrArtifact, PublishedResourceArtifact};

pub fn source_artifact<'a>(
    package: &'a PublishedPackageArtifact,
    source_path: &str,
) -> &'a PublishedFileIrArtifact {
    package
        .file_ir_units
        .iter()
        .find(|artifact| artifact.source_path == source_path)
        .unwrap_or_else(|| panic!("package should contain File IR source {source_path}"))
}

pub fn module_artifact<'a>(
    package: &'a PublishedPackageArtifact,
    module_path: &str,
) -> &'a PublishedFileIrArtifact {
    package
        .file_ir_units
        .iter()
        .find(|artifact| artifact.module_path == module_path)
        .unwrap_or_else(|| panic!("package should contain File IR module {module_path}"))
}

pub fn resource_blob<'a>(
    package: &'a PublishedPackageArtifact,
    artifact_path: &str,
) -> &'a PublishedResourceArtifact {
    package
        .resource_blobs
        .iter()
        .find(|artifact| artifact.artifact_path == artifact_path)
        .unwrap_or_else(|| panic!("package should contain resource blob {artifact_path}"))
}

pub fn write_resource_blobs(root: &Path, package: &PublishedPackageArtifact) {
    for resource in &package.resource_blobs {
        let path = root.join(&resource.artifact_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, &resource.bytes).unwrap();
    }
}

pub fn assert_file_ir_contains_package_symbol(
    package: &PublishedPackageArtifact,
    module_path: &str,
    dependency_ref: &str,
    symbol_path: &str,
) {
    let artifact_value = module_artifact(package, module_path).value();
    assert!(
        json_contains_package_symbol(&artifact_value, dependency_ref, symbol_path),
        "file IR unit {module_path} should contain package symbol {dependency_ref}:{symbol_path}: {artifact_value}",
    );
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
