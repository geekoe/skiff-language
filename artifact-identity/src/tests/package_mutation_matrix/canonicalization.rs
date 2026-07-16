use serde_json::{json, Map, Value};
use skiff_artifact_model::PackageDependencyConstraint;

use super::*;

#[test]
fn storage_paths_ref_provenance_and_diagnostic_wording_are_excluded() {
    let base = package_fixture("hello");
    let mut changed = base.clone();

    rewrite_file_ref_storage(&mut changed, "relocated", "new-source-provenance");
    changed
        .resources
        .push(resource_ref("asset.txt", "same-content"));
    let mut same_resource = resource_ref("asset.txt", "same-content");
    same_resource.artifact_path = Some("elsewhere/resource.bin".to_string());
    let mut baseline_with_resource = base.clone();
    baseline_with_resource.resources.push(same_resource);
    rewrite_operation_display_names(&mut changed, "new diagnostic wording");

    assert_eq!(
        identities(&baseline_with_resource),
        identities(&changed),
        "storage locations, repeated provenance and display diagnostics are not semantic"
    );
}

#[test]
fn collection_insertion_order_does_not_change_either_identity() {
    let mut forward = package_fixture("hello");
    forward
        .files
        .push(FileIrRef::new(file_identity('b'), "pkg.extra"));
    forward
        .resources
        .extend([resource_ref("b.txt", "bbb"), resource_ref("a.txt", "aaa")]);
    forward.dependencies.extend([
        dependency("example.com/b", "b", json!({ "z": 1, "a": 2 })),
        dependency("example.com/a", "a", json!({ "y": 3, "b": 4 })),
    ]);

    let mut reverse = forward.clone();
    reverse.files.reverse();
    reverse.resources.reverse();
    reverse.dependencies.reverse();
    for dependency in &mut reverse.dependencies {
        if let Value::Object(entries) = &dependency.config {
            let mut reversed = Map::new();
            for (key, value) in entries.iter().rev() {
                reversed.insert(key.clone(), value.clone());
            }
            dependency.config = Value::Object(reversed);
        }
    }

    assert_eq!(identities(&forward), identities(&reverse));
}

#[test]
fn typed_preimages_expose_only_the_declared_inclusion_matrix() {
    let unit = package_fixture("hello");
    let local = serde_json::to_value(
        package_local_abi_identity_projection(&unit).expect("local ABI projection"),
    )
    .expect("serialize local ABI projection");
    let build =
        serde_json::to_value(package_build_identity_projection(&unit).expect("build projection"))
            .expect("serialize build projection");

    assert_eq!(
        sorted_object_keys(&local),
        [
            "abiIdentityFacts",
            "packageId",
            "packageVersion",
            "publicSurfaceIdentity",
            "schema",
        ]
    );
    assert_eq!(
        sorted_object_keys(&build),
        [
            "callableEffects",
            "configRequirements",
            "fileIrUnits",
            "implementationLinks",
            "localAbiIdentity",
            "packageDependencies",
            "recoverableMetadata",
            "resources",
            "schema",
        ]
    );
    let text = build.to_string();
    assert!(!text.contains("artifactPath"));
    assert!(!text.contains("sourceAstHash"));
    assert!(!text.contains("displayName"));
}

fn rewrite_file_ref_storage(unit: &mut PackageUnit, path: &str, source_hash: &str) {
    for file in &mut unit.files {
        file.artifact_path = Some(format!("{path}/top-level.json"));
        file.source_ast_hash = Some(source_hash.to_string());
    }
    for export in unit
        .implementation_links
        .types
        .values_mut()
        .map(|export| &mut export.file)
        .chain(
            unit.implementation_links
                .constants
                .values_mut()
                .map(|export| &mut export.file),
        )
        .chain(
            unit.implementation_links
                .functions
                .values_mut()
                .map(|export| &mut export.file),
        )
        .chain(
            unit.implementation_links
                .impl_methods
                .values_mut()
                .map(|export| &mut export.file),
        )
    {
        export.artifact_path = Some(format!("{path}/link.json"));
        export.source_ast_hash = Some(source_hash.to_string());
    }
    for target in unit.implementation_links.operation_targets.values_mut() {
        match target {
            PackageOperationTarget::LocalExecutable { target, .. } => {
                target.file_ref.artifact_path = Some(format!("{path}/target.json"));
                target.file_ref.source_ast_hash = Some(source_hash.to_string());
            }
            PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => {
                target.receiver.file_ref.artifact_path = Some(format!("{path}/receiver.json"));
                target.receiver.file_ref.source_ast_hash = Some(source_hash.to_string());
                target.executable_target.file_ref.artifact_path =
                    Some(format!("{path}/target.json"));
                target.executable_target.file_ref.source_ast_hash = Some(source_hash.to_string());
            }
        }
    }
}

fn rewrite_operation_display_names(unit: &mut PackageUnit, display_name: &str) {
    for operation in &mut unit.publication_abi.operation_exports {
        operation.display_name = display_name.to_string();
    }
    for operation in &mut unit.publication_abi.operation_abi {
        operation.operation.display_name = display_name.to_string();
    }
    for operation in &mut unit.publication_abi.source_call_operation_index {
        operation.operation.display_name = display_name.to_string();
    }
    for target in unit.implementation_links.operation_targets.values_mut() {
        match target {
            PackageOperationTarget::LocalExecutable { operation, .. }
            | PackageOperationTarget::LocalConstReceiverExecutable { operation, .. } => {
                operation.display_name = display_name.to_string();
            }
        }
    }
}

fn dependency(id: &str, alias: &str, config: Value) -> PackageDependencyConstraint {
    PackageDependencyConstraint {
        id: id.to_string(),
        version: "1.0.0".to_string(),
        alias: alias.to_string(),
        config,
    }
}

fn sorted_object_keys(value: &Value) -> Vec<String> {
    let mut keys = value
        .as_object()
        .expect("identity projection must be an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    keys
}
