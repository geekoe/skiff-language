//! Official regenerator for the dynamic-build-id-parity cross-system fixture.
//!
//! Recomputes every content-derived identity bottom-up via the
//! `skiff_artifact_identity` library (the same code the runtime, compiler and
//! identity CLI use) and rewrites case.json values in place. Never hand-edit
//! fixture hashes; run this instead:
//!
//! ```bash
//! cargo test -p skiff-artifact-identity --test regenerate_dynamic_build_id_fixture -- --ignored
//! ```
//!
//! Remember to update value pins in consumers afterwards (e.g.
//! `router/tests/dynamic-build-id-parity.test.ts` EXPECTED_DYNAMIC_BUILD_ID).

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use skiff_artifact_identity::{
    assign_package_unit_identities, assign_publication_abi_identity, file_ir_identity,
    runtime_program_dynamic_build_id_from_artifact_root, service_unit_identity,
};
use skiff_artifact_model::{FileIrUnit, PackageUnit, ServiceUnit};

#[test]
#[ignore]
fn regenerate_dynamic_build_id_fixture() {
    let case_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("cross-system-fixtures/dynamic-build-id-parity/case.json");
    let mut case: Value =
        serde_json::from_str(&fs::read_to_string(&case_path).expect("read case.json"))
            .expect("parse case.json");

    let artifact_paths: Vec<String> = case["artifactRoot"]
        .as_object()
        .expect("artifactRoot object")
        .keys()
        .cloned()
        .collect();

    // 1. File IR units: recompute content identities.
    let mut new_file_identity_by_artifact_path = BTreeMap::<String, String>::new();
    for path in &artifact_paths {
        if !path.starts_with("units/files/") {
            continue;
        }
        let unit: FileIrUnit = serde_json::from_value(case["artifactRoot"][path].clone())
            .unwrap_or_else(|error| panic!("parse {path}: {error}"));
        let identity = file_ir_identity(&unit).expect("file ir identity");
        case["artifactRoot"][path]["fileIrIdentity"] = Value::String(identity.clone());
        case["artifactRoot"][path]["sourceAstHash"] = Value::String(format!("source:{identity}"));
        new_file_identity_by_artifact_path.insert(path.clone(), identity);
    }

    // 2. Update every file reference (files[] arrays, implementationLinks,
    //    and any other object carrying artifactPath + fileIrIdentity).
    fn update_file_refs(value: &mut Value, identities: &BTreeMap<String, String>) {
        match value {
            Value::Object(object) => {
                let referenced = object
                    .get("artifactPath")
                    .and_then(Value::as_str)
                    .and_then(|path| identities.get(path))
                    .cloned();
                if let Some(identity) = referenced {
                    if object.contains_key("fileIrIdentity") {
                        object.insert(
                            "fileIrIdentity".to_string(),
                            Value::String(identity.clone()),
                        );
                    }
                    if object.contains_key("sourceAstHash") {
                        object.insert(
                            "sourceAstHash".to_string(),
                            Value::String(format!("source:{identity}")),
                        );
                    }
                }
                for child in object.values_mut() {
                    update_file_refs(child, identities);
                }
            }
            Value::Array(items) => {
                for item in items {
                    update_file_refs(item, identities);
                }
            }
            _ => {}
        }
    }
    for path in &artifact_paths {
        if path.starts_with("units/files/") {
            continue;
        }
        update_file_refs(
            &mut case["artifactRoot"][path],
            &new_file_identity_by_artifact_path,
        );
    }

    // 3. Package units: recompute publication ABI + build/abi identities.
    let mut package_abi_by_id_version = BTreeMap::<(String, String), String>::new();
    for path in &artifact_paths {
        if !path.starts_with("units/packages/") {
            continue;
        }
        let mut unit: PackageUnit = serde_json::from_value(case["artifactRoot"][path].clone())
            .unwrap_or_else(|error| panic!("parse {path}: {error}"));
        let (build_identity, abi_identity) =
            assign_package_unit_identities(&mut unit).expect("package identities");
        case["artifactRoot"][path]["buildIdentity"] = Value::String(build_identity);
        case["artifactRoot"][path]["abiIdentity"] = Value::String(abi_identity.clone());
        case["artifactRoot"][path]["publicationAbi"]["abiIdentity"] =
            Value::String(unit.publication_abi.abi_identity.clone());
        package_abi_by_id_version.insert(
            (unit.package_id.clone(), unit.version.clone()),
            abi_identity,
        );
    }

    // 4. Service unit: package ABI expectations + publication ABI identity.
    let service_unit_path = case["serviceUnitPath"]
        .as_str()
        .expect("serviceUnitPath")
        .to_string();
    if let Some(expectations) = case["artifactRoot"][&service_unit_path]
        .get_mut("packageAbiExpectations")
        .and_then(Value::as_array_mut)
    {
        for expectation in expectations {
            let key = (
                expectation["id"]
                    .as_str()
                    .expect("expectation id")
                    .to_string(),
                expectation["version"]
                    .as_str()
                    .expect("expectation version")
                    .to_string(),
            );
            let abi = package_abi_by_id_version
                .get(&key)
                .unwrap_or_else(|| panic!("missing package abi for {key:?}"));
            expectation["abiIdentity"] = Value::String(abi.clone());
        }
    }
    let mut service: ServiceUnit =
        serde_json::from_value(case["artifactRoot"][&service_unit_path].clone())
            .expect("parse service unit");
    let publication_abi_identity =
        assign_publication_abi_identity(&mut service.publication_abi).expect("service pub abi");
    case["artifactRoot"][&service_unit_path]["publicationAbi"]["abiIdentity"] =
        Value::String(publication_abi_identity);

    // 5. Final identities from the fully updated tree.
    let service: ServiceUnit =
        serde_json::from_value(case["artifactRoot"][&service_unit_path].clone())
            .expect("parse final service unit");
    let unit_identity = service_unit_identity(&service).expect("service unit identity");
    case["expectedServiceUnitIdentity"] = Value::String(unit_identity);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("skiff-fixture-regen-{nanos}"));
    for (path, value) in case["artifactRoot"].as_object().expect("artifactRoot") {
        let file_path = root.join(path);
        fs::create_dir_all(file_path.parent().expect("parent")).expect("mkdir");
        fs::write(&file_path, serde_json::to_vec_pretty(value).expect("json")).expect("write");
    }
    let dynamic_build_id = runtime_program_dynamic_build_id_from_artifact_root(&root, &service)
        .expect("dynamic build id");
    case["expectedDynamicBuildId"] = Value::String(dynamic_build_id);
    fs::remove_dir_all(&root).ok();

    let mut text = serde_json::to_string_pretty(&case).expect("serialize case");
    text.push('\n');
    fs::write(&case_path, text).expect("write case.json");
}
