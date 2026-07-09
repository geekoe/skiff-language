use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use skiff_artifact_identity::{file_ir_identity, package_abi_identity, package_build_identity};

use super::*;
use crate::{
    artifact_cache::RuntimeArtifactCaches,
    program::{FileAddr, LinkedTypeDescriptor, ResolvedSymbol, TypeAddr, UnitAddr},
};

thread_local! {
    static FILE_IDENTITY_ALIASES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static PACKAGE_BUILD_IDENTITY_ALIASES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static PACKAGE_ABI_IDENTITY_ALIASES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

fn load_test_layers_at_root(
    root: &Path,
    selection: RuntimeProgramArtifactSelection,
) -> anyhow::Result<Arc<RuntimeProgramLayers>> {
    load_runtime_program_layers(
        selection,
        LoadOptions {
            roots: &[root.to_path_buf()],
            caches: None,
        },
    )
}

fn load_test_layers_at_root_with_caches(
    root: &Path,
    selection: RuntimeProgramArtifactSelection,
    caches: &RuntimeArtifactCaches,
) -> anyhow::Result<Arc<RuntimeProgramLayers>> {
    load_runtime_program_layers(
        selection,
        LoadOptions {
            roots: &[root.to_path_buf()],
            caches: Some(caches),
        },
    )
}

fn load_test_layers_at_roots(
    roots: &[PathBuf],
    selection: RuntimeProgramArtifactSelection,
) -> anyhow::Result<Arc<RuntimeProgramLayers>> {
    load_runtime_program_layers(
        selection,
        LoadOptions {
            roots,
            caches: None,
        },
    )
}

fn load_test_layers_at_roots_with_caches(
    roots: &[PathBuf],
    selection: RuntimeProgramArtifactSelection,
    caches: &RuntimeArtifactCaches,
) -> anyhow::Result<Arc<RuntimeProgramLayers>> {
    load_runtime_program_layers(
        selection,
        LoadOptions {
            roots,
            caches: Some(caches),
        },
    )
}

#[test]
fn artifact_loader_rejects_service_file_ref_missing_module_path() {
    let temp = TempDir::new("runtime-program-file-ref-missing-module-path");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    let mut service = service_unit_json(
        "example.com/svc",
        "v1",
        "file:service",
        "units/files/service.json",
        Vec::new(),
        Vec::new(),
    );
    service["files"][0]
        .as_object_mut()
        .expect("test service file ref should be an object")
        .remove("modulePath");
    write_service_unit(&root, "units/services/svc-v1.json", service);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("service unit file refs must declare modulePath");

    assert!(
        error.to_string().contains("modulePath"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn artifact_loader_rejects_service_file_ref_missing_artifact_path() {
    let temp = TempDir::new("runtime-program-file-ref-missing-artifact-path");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    let mut service = service_unit_json(
        "example.com/svc",
        "v1",
        "file:service",
        "units/files/service.json",
        Vec::new(),
        Vec::new(),
    );
    service["files"][0]
        .as_object_mut()
        .expect("test service file ref should be an object")
        .remove("artifactPath");
    write_service_unit(&root, "units/services/svc-v1.json", service);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("artifact root file refs must declare artifactPath");

    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("requires artifactPath"),
        "unexpected error: {error_text}"
    );
}

#[test]
fn artifact_loader_rejects_package_file_ref_missing_artifact_path() {
    let temp = TempDir::new("runtime-program-package-file-ref-missing-artifact-path");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_file_ir(&root, "units/files/pkg.json", "file:pkg", "pkg.main");
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "alias": "pkg"
            })],
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "abiIdentity": "abi:pkg",
                "usedSymbols": []
            })],
        ),
    );
    write_package_index(
        &root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    let mut package = package_unit_json("example.com/pkg", "1.0.0", "pkg:build:1.0.0", "abi:pkg");
    package["files"] = json!([
        {
            "fileIrIdentity": "file:pkg",
            "modulePath": "pkg.main",
            "sourceAstHash": "source:file:pkg"
        }
    ]);
    write_json(&root, "units/packages/pkg-1.0.0.json", package);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("artifact root package file refs must declare artifactPath");

    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("requires artifactPath"),
        "unexpected error: {error_text}"
    );
}

#[test]
fn artifact_loader_rejects_unknown_top_level_service_fields() {
    let temp = TempDir::new("runtime-program-service-unknown-top-level");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    let mut service = service_unit_json(
        "example.com/svc",
        "v1",
        "file:service",
        "units/files/service.json",
        Vec::new(),
        Vec::new(),
    );
    service["runtimeOnly"] = json!(true);
    write_service_unit(&root, "units/services/svc-v1.json", service);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("canonical service unit must reject unknown top-level fields");

    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("runtimeOnly") || error_text.contains("unknown field"),
        "unexpected error: {error_text}"
    );
}

#[test]
fn artifact_loader_rejects_unknown_top_level_package_fields_and_db() {
    for (case, field, value) in [
        ("unknown-top-level", "runtimeOnly", json!(true)),
        ("package-db", "db", json!([])),
    ] {
        let temp = TempDir::new(&format!("runtime-program-package-{case}"));
        let root = temp.path().join("artifacts");
        write_file_ir(
            &root,
            "units/files/service.json",
            "file:service",
            "svc.main",
        );
        write_service_unit(
            &root,
            "units/services/svc-v1.json",
            service_unit_json(
                "example.com/svc",
                "v1",
                "file:service",
                "units/files/service.json",
                vec![json!({
                    "id": "example.com/pkg",
                    "version": "1.0.0",
                    "alias": "pkg"
                })],
                Vec::new(),
            ),
        );
        write_package_index(
            &root,
            "example.com/pkg",
            "1.0.0",
            "units/packages/pkg-1.0.0.json",
        );
        let mut package =
            package_unit_json("example.com/pkg", "1.0.0", "pkg:build:1.0.0", "abi:pkg");
        package[field] = value;
        write_json(&root, "units/packages/pkg-1.0.0.json", package);
        write_release_pointer(
            &root,
            "example.com/svc",
            "v1",
            &build_identity_for_version_pointer(),
            "units/services/svc-v1.json",
        );

        let error = load_test_layers_at_root(
            &root,
            RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
        )
        .expect_err("canonical package unit must reject runtime-only fields");

        let error_text = format!("{error:#}");
        assert!(
            error_text.contains(field) || error_text.contains("unknown field"),
            "unexpected error for {case}: {error_text}"
        );
    }
}

#[test]
fn artifact_loader_rejects_service_operation_direct_file_and_executable() {
    for (case, field, value) in [
        (
            "direct-file",
            "file",
            json!({ "kind": "fileIrIdentity", "value": "file:service" }),
        ),
        ("direct-executable", "executable", json!(0)),
    ] {
        let temp = TempDir::new(&format!("runtime-program-operation-{case}"));
        let root = temp.path().join("artifacts");
        write_file_ir_with_executable_and_const(
            &root,
            "units/files/service.json",
            "file:service",
            "svc.main",
            true,
            false,
        );
        let mut service = service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        );
        service["operations"] = json!([service_operation_json(
            "svc.main.run",
            "file:service",
            0,
            json!({
                "kind": "builtin",
                "name": "Json"
            })
        )]);
        service["publicationAbi"] = publication_abi_json(
            "example.com/svc",
            "v1",
            "protocol:svc",
            "svc.main.run",
            json!({ "kind": "builtin", "name": "Json" }),
        );
        service["operations"][0][field] = value;
        write_service_unit(&root, "units/services/svc-v1.json", service);
        write_release_pointer(
            &root,
            "example.com/svc",
            "v1",
            &build_identity_for_version_pointer(),
            "units/services/svc-v1.json",
        );

        let error = load_test_layers_at_root(
            &root,
            RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
        )
        .expect_err("canonical service operation must reject direct runtime addresses");

        let error_text = format!("{error:#}");
        let rejected_legacy_direct_field =
            error_text.contains(field) || error_text.contains("unknown field");
        let rejected_direct_executable_value = case == "direct-executable"
            && error_text.contains("invalid type")
            && error_text.contains("OperationTargetRef");
        assert!(
            rejected_legacy_direct_field || rejected_direct_executable_value,
            "unexpected error for {case}: {error_text}"
        );
    }
}

#[test]
fn artifact_loader_accepts_canonical_service_package_and_preserves_package_constants() {
    let temp = TempDir::new("runtime-program-canonical-package-constants");
    let root = temp.path().join("artifacts");
    write_file_ir_with_executable_and_const(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
        true,
        false,
    );
    write_file_ir_with_executable_and_const(
        &root,
        "units/files/pkg.json",
        "file:pkg",
        "pkg.main",
        false,
        true,
    );

    let mut service = service_unit_json(
        "example.com/svc",
        "v1",
        "file:service",
        "units/files/service.json",
        vec![json!({
            "id": "example.com/pkg",
            "version": "1.0.0",
            "alias": "pkg"
        })],
        Vec::new(),
    );
    service["operations"] = json!([service_operation_json(
        "svc.main.run",
        "file:service",
        0,
        json!({
            "kind": "builtin",
            "name": "Json"
        })
    )]);
    service["publicationAbi"] = publication_abi_json(
        "example.com/svc",
        "v1",
        "protocol:svc",
        "svc.main.run",
        json!({ "kind": "builtin", "name": "Json" }),
    );
    write_service_unit(&root, "units/services/svc-v1.json", service);
    write_package_index(
        &root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    let mut package = package_unit_json("example.com/pkg", "1.0.0", "pkg:build:1.0.0", "abi:pkg");
    package["files"] = json!([
        {
            "fileIrIdentity": "file:pkg",
            "modulePath": "pkg.main",
            "artifactPath": "units/files/pkg.json",
            "sourceAstHash": "source:file:pkg"
        }
    ]);
    package["implementationLinks"] = json!({
        "constants": {
            "defaultLimit": {
                "file": {
                    "fileIrIdentity": "file:pkg",
                    "modulePath": "pkg.main",
                    "artifactPath": "units/files/pkg.json",
                    "sourceAstHash": "source:file:pkg"
                },
                "constIndex": 0,
                "symbol": "defaultLimit",
                "ty": { "kind": "builtin", "name": "Number" }
            }
        }
    });
    write_json(&root, "units/packages/pkg-1.0.0.json", package);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("canonical service/package units should load through runtime conversion");

    let operation_addr = program
        .image
        .operations
        .get(&operation_abi_id_for_target("svc.main.run"))
        .expect("operation ABI id should register a runtime target");
    assert_eq!(operation_addr.unit, UnitAddr::Service);
    // Service operation routes register in loaded-file-index form so the address
    // compares equal to the HTTP raw-adapter handler address (resolved through the
    // symbol overlay, which also emits `FileAddr::LoadedFileIndex`). The service file
    // is loaded at index 0.
    assert!(matches!(operation_addr.file, FileAddr::LoadedFileIndex(0)));
    assert_eq!(operation_addr.executable, 0);
    assert_eq!(
        program
            .image
            .link_overlay
            .resolved_package_symbol(0, "defaultLimit"),
        Some(&ResolvedSymbol::Constant {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            const_index: 0,
        })
    );
}

#[test]
fn artifact_loader_accepts_url_like_service_id_path_projection() {
    let temp = TempDir::new("runtime-program-url-like-service-id");
    let root = temp.path().join("artifacts");
    let service_id = "skiff.run/account";
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/skiff~run~~account/v1.json",
        service_unit_json(
            service_id,
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        service_id,
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/skiff~run~~account/v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release(service_id, "v1"),
    )
    .expect("URL-like service ids should project to storage-safe artifact paths");

    assert_eq!(program.activation.service.id, service_id);
    assert!(root
        .join("versions/services/skiff~run~~account/v1.json")
        .is_file());
    assert!(!root.join("versions/services/skiff.run%2Faccount").exists());
}

#[test]
fn artifact_loader_accepts_function_type_refs_in_file_ir() {
    let temp = TempDir::new("runtime-program-function-type-ref");
    let root = temp.path().join("artifacts");
    write_json(
        &root,
        "units/files/service.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:service",
            "sourceAstHash": "source:file:service",
            "modulePath": "svc.main",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {},
            "typeTable": [
                {
                    "name": "CallbackBox",
                    "descriptor": {
                        "kind": "record",
                        "fields": {
                            "callback": {
                                "kind": "function",
                                "params": [
                                    {
                                        "name": "input",
                                        "ty": { "kind": "builtin", "name": "string" }
                                    }
                                ],
                                "returnType": { "kind": "builtin", "name": "number" }
                            }
                        }
                    }
                }
            ],
            "executables": [],
            "externalRefs": {}
        }),
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("function type refs should load and link");

    let descriptor = program
        .image
        .types
        .descriptor(&TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        })
        .expect("type descriptor should be linked");
    assert!(matches!(descriptor, LinkedTypeDescriptor::Record { .. }));
}

#[test]
fn artifact_loader_rejects_legacy_service_unit_pointer_shapes() {
    for (label, service_unit_pointer, expected) in [
        (
            "top-level serviceUnitPath",
            json!({
                "serviceUnitPath": "units/services/svc-v1.json"
            }),
            "serviceUnitPath",
        ),
        (
            "string serviceUnit",
            json!({
                "serviceUnit": "units/services/svc-v1.json"
            }),
            "serviceUnit must be an object with unitPath",
        ),
        (
            "serviceUnit artifactPath",
            json!({
                "serviceUnit": {
                    "artifactPath": "units/services/svc-v1.json"
                }
            }),
            "serviceUnit requires unitPath",
        ),
        (
            "serviceUnit path",
            json!({
                "serviceUnit": {
                    "path": "units/services/svc-v1.json"
                }
            }),
            "serviceUnit requires unitPath",
        ),
    ] {
        let temp = TempDir::new(&format!("runtime-program-legacy-service-pointer-{label}"));
        let root = temp.path().join("artifacts");
        write_file_ir(
            &root,
            "units/files/service.json",
            "file:service",
            "svc.main",
        );
        write_service_unit(
            &root,
            "units/services/svc-v1.json",
            service_unit_json(
                "example.com/svc",
                "v1",
                "file:service",
                "units/files/service.json",
                Vec::new(),
                Vec::new(),
            ),
        );
        write_release_pointer_with_service_unit_pointer(
            &root,
            "example.com/svc",
            "v1",
            &build_identity_for_version_pointer(),
            service_unit_pointer,
        );

        let error = load_test_layers_at_root(
            &root,
            RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
        )
        .expect_err("legacy service unit pointer must fail");

        let error_text = format!("{error:#}");
        assert!(
            error_text.contains(expected),
            "{label}: expected error containing {expected:?}, got {error_text}"
        );
    }
}

#[test]
fn artifact_loader_accepts_descriptor_union_variants() {
    let temp = TempDir::new("runtime-program-descriptor-union-variants");
    let root = temp.path().join("artifacts");
    write_file_ir_with_type_descriptor(
        &root,
        "units/files/service.json",
        "file:service",
        json!({
            "kind": "union",
            "variants": [
                { "kind": "builtin", "name": "string" },
                { "kind": "builtin", "name": "number" }
            ]
        }),
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("descriptor union variants should load and link");

    let descriptor = program
        .image
        .types
        .descriptor(&TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        })
        .expect("type descriptor should be linked");
    assert!(matches!(descriptor, LinkedTypeDescriptor::Union { variants } if variants.len() == 2));
}

#[test]
fn artifact_loader_rejects_descriptor_union_items() {
    let temp = TempDir::new("runtime-program-descriptor-union-items");
    let root = temp.path().join("artifacts");
    write_file_ir_with_type_descriptor(
        &root,
        "units/files/service.json",
        "file:service",
        json!({
            "kind": "union",
            "items": [
                { "kind": "builtin", "name": "string" },
                { "kind": "builtin", "name": "number" }
            ]
        }),
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("descriptor union must use variants, not items");

    assert!(
        error.to_string().contains("variants"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn artifact_loader_reuses_artifact_file_cache_across_service_versions() {
    let temp = TempDir::new("runtime-program-file-cache");
    let root = temp.path().join("artifacts");
    write_file_ir(&root, "units/files/shared.json", "file:shared", "svc.main");
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:shared",
            "units/files/shared.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_service_unit(
        &root,
        "units/services/svc-v2.json",
        service_unit_json(
            "example.com/svc",
            "v2",
            "file:shared",
            "units/files/shared.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('a'),
        "units/services/svc-v1.json",
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v2",
        &build_identity('b'),
        "units/services/svc-v2.json",
    );

    let caches = RuntimeArtifactCaches::new();
    let layers_v1 = load_test_layers_at_root_with_caches(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
        &caches,
    )
    .expect("v1 should load");
    let layers_v2 = load_test_layers_at_root_with_caches(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v2"),
        &caches,
    )
    .expect("v2 should load");

    assert_ne!(
        layers_v1.identity.dynamic_build_id,
        layers_v2.identity.dynamic_build_id
    );
    assert_eq!(
        layers_v1.identity.linked_image_identity,
        layers_v2.identity.linked_image_identity
    );
    assert!(Arc::ptr_eq(&layers_v1.image, &layers_v2.image));
    assert!(!Arc::ptr_eq(&layers_v1.activation, &layers_v2.activation));
    assert_eq!(layers_v1.activation.version, "v1");
    assert_eq!(layers_v2.activation.version, "v2");
    let cached_activation_v1 = caches
        .activation_cache
        .get_by_dynamic_build_id(&layers_v1.identity.dynamic_build_id)
        .expect("v1 activation should be cached")
        .activation();
    let cached_activation_v2 = caches
        .activation_cache
        .get_by_dynamic_build_id(&layers_v2.identity.dynamic_build_id)
        .expect("v2 activation should be cached")
        .activation();
    assert!(Arc::ptr_eq(&cached_activation_v1, &layers_v1.activation));
    assert!(Arc::ptr_eq(&cached_activation_v2, &layers_v2.activation));
    assert!(!Arc::ptr_eq(&cached_activation_v1, &cached_activation_v2));
    assert_eq!(
        layers_v1.image.service_files[0].file_ir_identity,
        layers_v2.image.service_files[0].file_ir_identity
    );
    assert_eq!(caches.files.len(), 1);
    assert_eq!(caches.images.len(), 1);
    assert_eq!(caches.activation_cache.len(), 2);
}

#[test]
fn artifact_loader_exposes_loaded_artifact_graph_before_linking() {
    let temp = TempDir::new("runtime-program-artifact-graph");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );

    let caches = RuntimeArtifactCaches::new();
    let loader = RuntimeProgramPartsLoader::new(&root, &caches);
    let service = Arc::new(
        loader
            .load_service_unit_at_path(Path::new("units/services/svc-v1.json"))
            .expect("service unit should load"),
    );
    let graph = loader
        .load_service_artifact_graph(service)
        .expect("artifact graph should load");

    assert_eq!(graph.service_unit.service.id, "example.com/svc");
    assert_eq!(graph.service_files.len(), 1);
    assert_eq!(
        graph.identities.service_file_ir_identities,
        vec![graph.service_files[0].file_ir_identity.clone()]
    );
    assert!(graph.package_units.is_empty());
    assert!(graph.package_files.is_empty());
    assert!(graph.identities.package_build_identities.is_empty());
    assert!(graph.identities.package_file_ir_identities.is_empty());
    assert_eq!(caches.files.len(), 1);
    assert!(caches.images.is_empty());
    assert!(caches.activation_cache.is_empty());

    let parts = loader
        .link_loaded_artifact_graph_parts(graph)
        .expect("artifact graph should link");
    let layers = runtime_program_layers_from_parts(parts);
    assert_eq!(layers.activation.service.id, "example.com/svc");
    assert!(layers
        .identity
        .linked_image_identity
        .starts_with("skiff-linked-program-image-v1:sha256:"));
    assert_eq!(caches.images.len(), 1);
    assert_eq!(caches.activation_cache.len(), 1);
}

#[test]
fn artifact_loader_rejects_activation_cache_linked_image_identity_mismatch() {
    let temp = TempDir::new("runtime-program-activation-cache-image-mismatch");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );

    let caches = RuntimeArtifactCaches::new();
    let loader = RuntimeProgramPartsLoader::new(&root, &caches);
    let service = Arc::new(
        loader
            .load_service_unit_at_path(Path::new("units/services/svc-v1.json"))
            .expect("service unit should load"),
    );
    let graph = loader
        .load_service_artifact_graph(service)
        .expect("artifact graph should load");
    let image_build =
        link_runtime_program_image(graph.clone()).expect("artifact graph should link");
    let mut cached_identity = image_build.identity.clone();
    let activation = Arc::new(
        build_runtime_activation_for_image(&image_build.image, image_build.activation_facts)
            .expect("linked image activation should build"),
    );
    cached_identity.linked_image_identity = "image:stale".to_string();
    caches
        .activation_cache
        .insert_arc(cached_identity, activation);

    let error = match loader.link_loaded_artifact_graph_parts(graph) {
        Ok(_) => panic!("mismatched activation cache entry should fail"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("cached runtime activation linked image identity mismatch"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn artifact_loader_evicts_lru_cache_entries_after_load_when_over_budget() {
    let temp = TempDir::new("runtime-program-cache-budget");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let caches = RuntimeArtifactCaches::with_artifact_budget_bytes(1);
    let layers = load_test_layers_at_root_with_caches(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
        &caches,
    )
    .expect("program should load even when cache entries are evicted");

    assert_eq!(layers.activation.service.id, "example.com/svc");
    assert!(caches.total_estimated_size_bytes() <= caches.memory_budgets().artifact_cache_bytes);
}

#[test]
fn artifact_loader_rejects_cached_file_ref_with_unsafe_artifact_path() {
    for unsafe_artifact_path in ["../escape.json", "/tmp/escape.json"] {
        let temp = TempDir::new("runtime-program-file-cache-unsafe-artifact-path");
        let root = temp.path().join("artifacts");
        write_file_ir(&root, "units/files/shared.json", "file:shared", "svc.main");
        write_service_unit(
            &root,
            "units/services/svc-v1.json",
            service_unit_json(
                "example.com/svc",
                "v1",
                "file:shared",
                "units/files/shared.json",
                Vec::new(),
                Vec::new(),
            ),
        );
        let mut unsafe_service = service_unit_json(
            "example.com/svc",
            "v2",
            "file:shared",
            "units/files/shared.json",
            Vec::new(),
            Vec::new(),
        );
        unsafe_service["files"][0]["artifactPath"] = json!(unsafe_artifact_path);
        write_service_unit(&root, "units/services/svc-v2.json", unsafe_service);
        write_release_pointer(
            &root,
            "example.com/svc",
            "v1",
            &build_identity('a'),
            "units/services/svc-v1.json",
        );
        write_release_pointer(
            &root,
            "example.com/svc",
            "v2",
            &build_identity('b'),
            "units/services/svc-v2.json",
        );

        let caches = RuntimeArtifactCaches::new();
        load_test_layers_at_root_with_caches(
            &root,
            RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
            &caches,
        )
        .expect("safe v1 should populate the file cache");

        let error = load_test_layers_at_root_with_caches(
            &root,
            RuntimeProgramArtifactSelection::release("example.com/svc", "v2"),
            &caches,
        )
        .expect_err("unsafe artifactPath must fail even when file identity is cached");

        let error_text = format!("{error:#}");
        assert!(
            error_text.contains("must be relative and stay inside artifacts root"),
            "expected unsafe artifactPath error for {unsafe_artifact_path}, got {error_text}"
        );
        assert_eq!(caches.files.len(), 1);
    }
}

#[test]
fn artifact_loader_rejects_mismatched_package_abi_expectation() {
    let temp = TempDir::new("runtime-program-package-abi-mismatch");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "alias": "pkg"
            })],
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "abiIdentity": "abi:expected",
                "usedSymbols": []
            })],
        ),
    );
    write_package_index(
        &root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    write_package_unit(
        &root,
        "units/packages/pkg-1.0.0.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:1.0.0",
        "abi:actual",
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('c'),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("mismatched package ABI expectation must gate activation");

    assert!(
        format!("{error:#}").contains("ABI identity mismatch"),
        "expected ABI mismatch error, got {error:#}"
    );
}

#[test]
fn artifact_loader_accepts_matching_package_abi_expectation() {
    let temp = TempDir::new("runtime-program-package-abi-match");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "alias": "pkg"
            })],
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "abiIdentity": "abi:actual",
                "usedSymbols": []
            })],
        ),
    );
    write_package_index(
        &root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    write_package_unit(
        &root,
        "units/packages/pkg-1.0.0.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:1.0.0",
        "abi:actual",
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('c'),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("matching package ABI expectation should allow activation");

    assert_eq!(
        program.image.packages[0].abi_identity,
        empty_package_abi_identity_for_test()
    );
}

#[test]
fn artifact_loader_uses_pinned_package_units_when_version_index_moves() {
    let temp = TempDir::new("runtime-program-pinned-package-units");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_package_unit(
        &root,
        "units/packages/pkg-old.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:old",
        "abi:actual",
    );
    write_package_unit(
        &root,
        "units/packages/pkg-new.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:new",
        "abi:new",
    );
    let pinned_package = read_json(&root, "units/packages/pkg-old.json");
    let pinned_build_identity = pinned_package["buildIdentity"]
        .as_str()
        .expect("pinned package build identity")
        .to_string();
    let pinned_abi_identity = pinned_package["abiIdentity"]
        .as_str()
        .expect("pinned package ABI identity")
        .to_string();
    let service_unit = service_unit_json(
        "example.com/svc",
        "v1",
        "file:service",
        "units/files/service.json",
        vec![json!({
            "id": "example.com/pkg",
            "version": "1.0.0",
            "alias": "pkg"
        })],
        vec![json!({
            "id": "example.com/pkg",
            "version": "1.0.0",
            "abiIdentity": pinned_abi_identity,
            "usedSymbols": []
        })],
    );
    write_service_unit(&root, "units/services/svc-v1.json", service_unit);
    write_package_index(
        &root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-new.json",
    );
    write_release_pointer_with_service_unit_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('c'),
        json!({
            "serviceUnit": {
                "unitPath": "units/services/svc-v1.json"
            },
            "packageUnits": [{
                "schemaVersion": "skiff-package-unit-v1",
                "packageId": "example.com/pkg",
                "version": "1.0.0",
                "buildIdentity": pinned_build_identity,
                "abiIdentity": pinned_package["abiIdentity"],
                "unitPath": "units/packages/pkg-old.json"
            }]
        }),
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("pinned package unit should load even after version index moves");

    assert_eq!(
        program.image.packages[0].build_identity,
        pinned_build_identity
    );
}

#[test]
fn artifact_loader_rejects_explicit_empty_package_unit_lock_for_dependencies() {
    let temp = TempDir::new("runtime-program-empty-package-unit-lock");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "alias": "pkg"
            })],
            vec![],
        ),
    );
    write_package_unit(
        &root,
        "units/packages/pkg.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build",
        "abi:actual",
    );
    write_package_index(&root, "example.com/pkg", "1.0.0", "units/packages/pkg.json");
    write_release_pointer_with_service_unit_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('c'),
        json!({
            "serviceUnit": {
                "unitPath": "units/services/svc-v1.json"
            },
            "packageUnits": []
        }),
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("explicit empty packageUnits must not fall back to package indexes");

    assert!(
        format!("{error:#}")
            .contains("pinned packageUnits missing dependency example.com/pkg@1.0.0"),
        "expected missing pinned dependency error, got {error:#}"
    );
}

#[test]
fn artifact_loader_resolves_packages_from_selected_artifact_root() {
    let temp = TempDir::new("runtime-program-multi-root-package-binding");
    let default_root = temp.path().join("default-artifacts");
    let override_root = temp.path().join("override-artifacts");
    let build_id = build_identity('f');

    write_package_index(
        &default_root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    write_package_unit(
        &default_root,
        "units/packages/pkg-1.0.0.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:wrong",
        "abi:wrong",
    );

    write_file_ir(
        &override_root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &override_root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "alias": "pkg"
            })],
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "abiIdentity": "abi:actual",
                "usedSymbols": []
            })],
        ),
    );
    write_package_index(
        &override_root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    write_package_unit(
        &override_root,
        "units/packages/pkg-1.0.0.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:actual",
        "abi:actual",
    );
    write_release_pointer(
        &override_root,
        "example.com/svc",
        "v1",
        &build_id,
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_roots(
        &[default_root, override_root],
        RuntimeProgramArtifactSelection::release_build("example.com/svc", build_id),
    )
    .expect("selected pointer root should own package resolution");

    assert_eq!(
        program.image.packages[0].abi_identity,
        empty_package_abi_identity_for_test()
    );
}

#[test]
fn artifact_loader_scopes_file_cache_by_artifact_root() {
    let temp = TempDir::new("runtime-program-multi-root-cache-scope");
    let default_root = temp.path().join("default-artifacts");
    let override_root = temp.path().join("override-artifacts");
    let default_build_id = build_identity('1');
    let override_build_id = build_identity('2');

    write_file_ir(
        &default_root,
        "units/files/service.json",
        "file:shared",
        "svc.default",
    );
    let mut default_service = service_unit_json(
        "example.com/default",
        "v1",
        "file:shared",
        "units/files/service.json",
        Vec::new(),
        Vec::new(),
    );
    default_service["files"][0]["modulePath"] = json!("svc.default");
    write_service_unit(
        &default_root,
        "units/services/default-v1.json",
        default_service,
    );
    write_release_pointer(
        &default_root,
        "example.com/default",
        "v1",
        &default_build_id,
        "units/services/default-v1.json",
    );

    write_file_ir(
        &override_root,
        "units/files/service.json",
        "file:shared",
        "svc.override",
    );
    let mut override_service = service_unit_json(
        "example.com/override",
        "v1",
        "file:shared",
        "units/files/service.json",
        Vec::new(),
        Vec::new(),
    );
    override_service["files"][0]["modulePath"] = json!("svc.override");
    write_service_unit(
        &override_root,
        "units/services/override-v1.json",
        override_service,
    );
    write_release_pointer(
        &override_root,
        "example.com/override",
        "v1",
        &override_build_id,
        "units/services/override-v1.json",
    );

    let caches = RuntimeArtifactCaches::new();
    let default_program = load_test_layers_at_roots_with_caches(
        &[default_root.clone(), override_root.clone()],
        RuntimeProgramArtifactSelection::release_build("example.com/default", default_build_id),
        &caches,
    )
    .expect("default root service should load");
    let override_program = load_test_layers_at_roots_with_caches(
        &[default_root, override_root],
        RuntimeProgramArtifactSelection::release_build("example.com/override", override_build_id),
        &caches,
    )
    .expect("override root service should not reuse default root FileIR");

    assert_eq!(
        default_program.image.service_files[0].module_path,
        "svc.default"
    );
    assert_eq!(
        override_program.image.service_files[0].module_path,
        "svc.override"
    );
    assert_eq!(caches.files.len(), 2);
}

#[test]
fn artifact_loader_rejects_legacy_package_unit_pointer_shapes() {
    for (label, package_index_pointer, expected) in [
        (
            "top-level packageUnitPath",
            json!({
                "packageUnitPath": "units/packages/pkg-1.0.0.json"
            }),
            "packageUnit.unitPath",
        ),
        (
            "string packageUnit",
            json!({
                "packageUnit": "units/packages/pkg-1.0.0.json"
            }),
            "packageUnit must be an object with unitPath",
        ),
        (
            "packageUnit artifactPath",
            json!({
                "packageUnit": {
                    "artifactPath": "units/packages/pkg-1.0.0.json"
                }
            }),
            "packageUnit requires unitPath",
        ),
        (
            "packageUnit path",
            json!({
                "packageUnit": {
                    "path": "units/packages/pkg-1.0.0.json"
                }
            }),
            "packageUnit requires unitPath",
        ),
    ] {
        let temp = TempDir::new(&format!("runtime-program-legacy-package-pointer-{label}"));
        let root = temp.path().join("artifacts");
        write_file_ir(
            &root,
            "units/files/service.json",
            "file:service",
            "svc.main",
        );
        write_service_unit(
            &root,
            "units/services/svc-v1.json",
            service_unit_json(
                "example.com/svc",
                "v1",
                "file:service",
                "units/files/service.json",
                vec![json!({
                    "id": "example.com/pkg",
                    "version": "1.0.0",
                    "alias": "pkg"
                })],
                vec![json!({
                    "id": "example.com/pkg",
                    "version": "1.0.0",
                    "abiIdentity": "abi:pkg",
                    "usedSymbols": []
                })],
            ),
        );
        write_package_index_with_pointer(&root, "example.com/pkg", "1.0.0", package_index_pointer);
        write_package_unit(
            &root,
            "units/packages/pkg-1.0.0.json",
            "example.com/pkg",
            "1.0.0",
            "pkg:build:1.0.0",
            "abi:pkg",
        );
        write_release_pointer(
            &root,
            "example.com/svc",
            "v1",
            &build_identity_for_version_pointer(),
            "units/services/svc-v1.json",
        );

        let error = load_test_layers_at_root(
            &root,
            RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
        )
        .expect_err("legacy package unit pointer must fail");

        let error_text = format!("{error:#}");
        assert!(
            error_text.contains(expected),
            "{label}: expected error containing {expected:?}, got {error_text}"
        );
    }
}

#[test]
fn artifact_loader_resolves_authority_path_package_id() {
    let temp = TempDir::new("runtime-program-authority-package");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "skiff.run/http-session",
                "version": "1.0.0",
                "alias": "httpSession"
            })],
            vec![json!({
                "id": "skiff.run/http-session",
                "version": "1.0.0",
                "abiIdentity": "abi:http-session",
                "usedSymbols": []
            })],
        ),
    );
    write_package_index(
        &root,
        "skiff.run/http-session",
        "1.0.0",
        "units/packages/http-session-1.0.0.json",
    );
    write_package_unit(
        &root,
        "units/packages/http-session-1.0.0.json",
        "skiff.run/http-session",
        "1.0.0",
        "pkg:build:http-session",
        "abi:http-session",
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('f'),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("authority/path package id should load");

    assert_eq!(
        program.image.packages[0].package_id,
        "skiff.run/http-session"
    );
}

#[test]
fn artifact_loader_loads_transitive_package_dependencies() {
    let temp = TempDir::new("runtime-program-transitive-package");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg-a",
                "version": "1.0.0",
                "alias": "pkgA"
            })],
            Vec::new(),
        ),
    );
    write_package_index(
        &root,
        "example.com/pkg-a",
        "1.0.0",
        "units/packages/pkg-a-1.0.0.json",
    );
    write_json(
        &root,
        "units/packages/pkg-a-1.0.0.json",
        json!({
            "schemaVersion": "skiff-package-unit-v1",
            "packageId": "example.com/pkg-a",
            "version": "1.0.0",
            "buildIdentity": "pkg-a:build",
            "abiIdentity": "pkg-a:abi",
            "publicationAbi": empty_publication_abi_json("example.com/pkg-a", "1.0.0", "pkg-a:abi"),
            "files": [],
            "implementationLinks": {},
            "dependencies": [
                {
                    "id": "example.com/pkg-b",
                    "version": "1.0.0",
                    "alias": "db"
                }
            ],
            "configAndEffectMetadata": {}
        }),
    );
    write_package_index(
        &root,
        "example.com/pkg-b",
        "1.0.0",
        "units/packages/pkg-b-1.0.0.json",
    );
    write_package_unit(
        &root,
        "units/packages/pkg-b-1.0.0.json",
        "example.com/pkg-b",
        "1.0.0",
        "pkg-b:build",
        "pkg-b:abi",
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('7'),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("program with transitive package dependency should load");

    assert_eq!(
        program
            .image
            .packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        vec!["example.com/pkg-a", "example.com/pkg-b"]
    );
    let db_slot = program
        .image
        .link_overlay
        .package_slot_for_dependency_ref("db")
        .expect("package dependency ref should be linked");
    assert_eq!(
        program.image.packages[db_slot].package_id,
        "example.com/pkg-b"
    );
}

#[test]
fn artifact_loader_rejects_caret_package_version_without_fallback() {
    let temp = TempDir::new("runtime-program-reject-caret-package-version");
    let root = temp.path().join("artifacts");
    write_file_ir(
        &root,
        "units/files/service.json",
        "file:service",
        "svc.main",
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "^1",
                "alias": "pkg"
            })],
            Vec::new(),
        ),
    );
    write_package_index(
        &root,
        "example.com/pkg",
        "1.0.0",
        "units/packages/pkg-1.0.0.json",
    );
    write_package_unit(
        &root,
        "units/packages/pkg-1.0.0.json",
        "example.com/pkg",
        "1.0.0",
        "pkg:build:1.0.0",
        "abi:pkg",
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity('8'),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("caret versions must not fall back to the latest matching index");

    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("version ^1"),
        "unexpected error: {error_text}"
    );
}

#[test]
fn artifact_loader_package_bugfix_changes_dynamic_build_id_for_same_service_version() {
    let temp_a = TempDir::new("runtime-program-bugfix-a");
    let temp_b = TempDir::new("runtime-program-bugfix-b");
    let root_a = temp_a.path().join("artifacts");
    let root_b = temp_b.path().join("artifacts");
    write_package_bugfix_root(&root_a, "pkg:build:old");
    write_package_bugfix_root(&root_b, "pkg:build:new");

    let program_a = load_test_layers_at_root(
        &root_a,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("program with old package build should load");
    let program_b = load_test_layers_at_root(
        &root_b,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("program with new package build should load");

    assert_eq!(program_a.activation.version, "v1");
    assert_eq!(program_b.activation.version, "v1");
    assert_eq!(program_a.image.packages[0].version, "1.0.0");
    assert_eq!(program_b.image.packages[0].version, "1.0.0");
    assert_eq!(
        program_a.image.packages[0].abi_identity,
        empty_package_abi_identity_for_test()
    );
    assert_eq!(
        program_b.image.packages[0].abi_identity,
        empty_package_abi_identity_for_test()
    );
    assert_ne!(
        program_a.image.packages[0].build_identity,
        program_b.image.packages[0].build_identity
    );
    assert_ne!(
        program_a.identity.dynamic_build_id,
        program_b.identity.dynamic_build_id
    );
    assert!(program_a
        .identity
        .dynamic_build_id
        .starts_with("skiff-service-build-v1:sha256:"));
    assert!(program_b
        .identity
        .dynamic_build_id
        .starts_with("skiff-service-build-v1:sha256:"));
}

#[test]
fn artifact_loader_dynamic_build_id_matches_cross_system_fixture() {
    let fixture = dynamic_build_id_fixture();
    assert!(fixture.applies_to.iter().any(|system| system == "runtime"));

    let temp = TempDir::new("runtime-dynamic-build-id-fixture");
    write_fixture_artifact_root(temp.path(), &fixture);

    let program = load_test_layers_at_root(
        temp.path(),
        RuntimeProgramArtifactSelection::release(&fixture.service_id, &fixture.service_version),
    )
    .expect("cross-system dynamic build id fixture should load");

    assert_eq!(
        program.identity.dynamic_build_id,
        fixture.expected_dynamic_build_id
    );
    assert_eq!(
        program
            .image
            .packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "example.com/pkg-alpha",
            "skiff.run/std",
            "example.com/pkg-shared",
            "example.com/pkg-leaf",
            "example.com/pkg-beta"
        ]
    );
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicBuildIdFixture {
    applies_to: Vec<String>,
    service_id: String,
    service_version: String,
    expected_dynamic_build_id: String,
    artifact_root: BTreeMap<String, Value>,
}

fn dynamic_build_id_fixture() -> DynamicBuildIdFixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime crate should live under the skiff repository root")
        .join("cross-system-fixtures/dynamic-build-id-parity/case.json");
    let text = fs::read_to_string(&path).expect("dynamic build id fixture should be readable");
    serde_json::from_str(&text).expect("dynamic build id fixture should parse")
}

fn write_fixture_artifact_root(root: &Path, fixture: &DynamicBuildIdFixture) {
    for (relative_path, value) in &fixture.artifact_root {
        write_json_raw(root, relative_path, value.clone());
    }
}

fn write_package_bugfix_root(root: &Path, build_identity: &str) {
    let version = "1.0.0";
    write_file_ir(root, "units/files/service.json", "file:service", "svc.main");
    write_service_unit(
        root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "alias": "pkg"
            })],
            vec![json!({
                "id": "example.com/pkg",
                "version": "1.0.0",
                "abiIdentity": "abi:pkg",
                "usedSymbols": []
            })],
        ),
    );
    write_package_index(
        root,
        "example.com/pkg",
        version,
        &format!("units/packages/pkg-{version}.json"),
    );
    write_package_unit(
        root,
        &format!("units/packages/pkg-{version}.json"),
        "example.com/pkg",
        version,
        build_identity,
        "abi:pkg",
    );
    write_release_pointer(
        root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );
}

fn service_unit_json(
    service_id: &str,
    version: &str,
    file_identity: &str,
    file_path: &str,
    package_dependencies: Vec<Value>,
    package_abi_expectations: Vec<Value>,
) -> Value {
    json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": {
            "id": service_id,
            "displayName": "Service"
        },
        "version": version,
        "protocolIdentity": "protocol:svc",
        "publicationAbi": empty_publication_abi_json(service_id, version, "protocol:svc"),
        "files": [
            {
                "fileIrIdentity": file_identity,
                "modulePath": "svc.main",
                "artifactPath": file_path,
                "sourceAstHash": format!("source:{file_identity}")
            }
        ],
        "packageDependencies": package_dependencies,
        "packageAbiExpectations": package_abi_expectations,
        "operations": [],
        "gateway": {},
        "config": {}
    })
}

fn empty_publication_abi_json(publication_id: &str, version: &str, abi_identity: &str) -> Value {
    json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": publication_id,
        "version": version,
        "abiIdentity": abi_identity
    })
}

fn publication_abi_json(
    publication_id: &str,
    version: &str,
    abi_identity: &str,
    target: &str,
    return_type: Value,
) -> Value {
    let operation = operation_ref_json(target);
    let public_path = operation_public_path(target);
    json!({
        "schemaVersion": "skiff-publication-abi-unit-v1",
        "publicationId": publication_id,
        "version": version,
        "abiIdentity": abi_identity,
        "operationExports": [operation.clone()],
        "operationAbi": [
            {
                "operation": operation.clone(),
                "publicSignature": {
                    "params": [],
                    "returnType": return_type,
                    "maySuspend": false
                }
            }
        ],
        "sourceCallOperationIndex": [
            {
                "sourceCallPath": public_path,
                "operation": operation
            }
        ]
    })
}

fn operation_ref_json(target: &str) -> Value {
    json!({
        "operationAbiId": operation_abi_id_for_target(target),
        "kind": "publicFunction",
        "publicPath": operation_public_path(target),
        "displayName": target
    })
}

fn operation_public_path(target: &str) -> String {
    target
        .rsplit_once('.')
        .map(|(_, symbol)| symbol.to_string())
        .unwrap_or_else(|| target.to_string())
}

fn operation_abi_id_for_target(target: &str) -> String {
    format!("operation:{target}")
}

fn operation_target_ref_json(
    file_identity: &str,
    module_path: &str,
    symbol: &str,
    executable_index: u32,
    callable_kind: &str,
) -> Value {
    json!({
        "fileRef": {
            "fileIrIdentity": file_identity,
            "modulePath": module_path
        },
        "executableIndex": executable_index,
        "callableAbiId": format!("callable:{module_path}.{symbol}"),
        "callableKind": callable_kind
    })
}

fn service_operation_json(
    target: &str,
    file_identity: &str,
    executable_index: u32,
    _return_type: Value,
) -> Value {
    let (module_path, symbol) = target
        .rsplit_once('.')
        .expect("test operation target should include module path and symbol");
    json!({
        "kind": "localExecutable",
        "operation": operation_ref_json(target),
        "executable": operation_target_ref_json(
            file_identity,
            module_path,
            symbol,
            executable_index,
            "publicFunction",
        ),
    })
}

fn write_file_ir(root: &Path, relative_path: &str, identity: &str, module_path: &str) {
    write_json(
        root,
        relative_path,
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": identity,
            "sourceAstHash": format!("source:{identity}"),
            "modulePath": module_path,
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {},
            "typeTable": [],
            "executables": [],
            "externalRefs": {}
        }),
    );
}

fn write_file_ir_with_executable_and_const(
    root: &Path,
    relative_path: &str,
    identity: &str,
    module_path: &str,
    include_executable: bool,
    include_const: bool,
) {
    let mut link_targets = json!({});
    let mut executables = Vec::new();
    if include_executable {
        link_targets["executables"] = json!({
            "run": { "executableIndex": 0 }
        });
        executables.push(json!({
            "kind": "function",
            "symbol": "run",
            "returnType": { "kind": "builtin", "name": "Json" },
            "slots": { "slots": [], "frameSize": 0 },
            "maySuspend": false,
            "body": {}
        }));
    }

    let mut constants = Vec::new();
    if include_const {
        link_targets["constants"] = json!({
            "defaultLimit": { "constIndex": 0 }
        });
        constants.push(json!({
            "name": "defaultLimit",
            "ty": { "kind": "builtin", "name": "Number" },
            "body": {}
        }));
    }

    write_json(
        root,
        relative_path,
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": identity,
            "sourceAstHash": format!("source:{identity}"),
            "modulePath": module_path,
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": link_targets,
            "typeTable": [],
            "constants": constants,
            "executables": executables,
            "externalRefs": {}
        }),
    );
}

fn write_file_ir_with_type_descriptor(
    root: &Path,
    relative_path: &str,
    identity: &str,
    descriptor: Value,
) {
    write_json(
        root,
        relative_path,
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": identity,
            "sourceAstHash": format!("source:{identity}"),
            "modulePath": "svc.main",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {},
            "typeTable": [
                {
                    "name": "TestType",
                    "descriptor": descriptor
                }
            ],
            "executables": [],
            "externalRefs": {}
        }),
    );
}

fn write_service_unit(root: &Path, relative_path: &str, value: Value) {
    write_json(root, relative_path, value);
}

fn write_package_index(root: &Path, package_id: &str, version: &str, package_unit_path: &str) {
    write_package_index_with_pointer(
        root,
        package_id,
        version,
        json!({
            "packageUnit": {
                "unitPath": package_unit_path
            }
        }),
    );
}

fn write_package_index_with_pointer(
    root: &Path,
    package_id: &str,
    version: &str,
    package_unit_pointer: Value,
) {
    let mut index = json!({
        "schemaVersion": "skiff-package-unit-index-v1",
        "packageId": package_id,
        "version": version
    });
    index
        .as_object_mut()
        .expect("test package index should be an object")
        .extend(
            package_unit_pointer
                .as_object()
                .expect("test package unit pointer should be an object")
                .clone(),
        );
    write_json(
        root,
        &format!(
            "indexes/packages/{}/versions/{version}.json",
            storage_segment_for_test(package_id)
        ),
        index,
    );
}

fn write_package_unit(
    root: &Path,
    relative_path: &str,
    package_id: &str,
    version: &str,
    build_identity: &str,
    abi_identity: &str,
) {
    write_json(
        root,
        relative_path,
        package_unit_json(package_id, version, build_identity, abi_identity),
    );
}

fn package_unit_json(
    package_id: &str,
    version: &str,
    build_identity: &str,
    abi_identity: &str,
) -> Value {
    json!({
        "schemaVersion": "skiff-package-unit-v1",
        "packageId": package_id,
        "version": version,
        "buildIdentity": build_identity,
        "abiIdentity": abi_identity,
        "publicationAbi": empty_publication_abi_json(package_id, version, abi_identity),
        "files": [],
        "implementationLinks": {},
        "dependencies": [],
        "configAndEffectMetadata": {
            "effects": {
                "__testBuildSeed": {
                    "metadata": {
                        "value": build_identity
                    }
                }
            }
        }
    })
}

fn write_release_pointer(
    root: &Path,
    service_id: &str,
    version: &str,
    build_id: &str,
    service_unit_path: &str,
) {
    write_release_pointer_with_service_unit_pointer(
        root,
        service_id,
        version,
        build_id,
        json!({
            "serviceUnit": {
                "unitPath": service_unit_path
            }
        }),
    );
}

fn write_release_pointer_with_service_unit_pointer(
    root: &Path,
    service_id: &str,
    version: &str,
    build_id: &str,
    service_unit_pointer: Value,
) {
    let service_id_path = service_id_path_for_test(service_id);
    write_json(
        root,
        &format!("versions/services/{service_id_path}/{version}.json"),
        json!({
            "schemaVersion": "skiff-service-version-pointer-v1",
            "serviceId": service_id,
            "version": version,
            "buildId": build_id
        }),
    );
    let build_hash = build_id
        .rsplit_once(":sha256:")
        .expect("test build id should include hash")
        .1;
    let mut build_record = json!({
        "schemaVersion": "skiff-service-build-v1",
        "serviceId": service_id,
        "serviceVersion": version,
        "buildId": build_id,
        "serviceAssembly": {
            "assemblyIdentity": build_identity('d'),
            "assemblyPath": "assemblies/services/unused.json"
        }
    });
    build_record
        .as_object_mut()
        .expect("test service build record should be an object")
        .extend(
            service_unit_pointer
                .as_object()
                .expect("test service unit pointer should be an object")
                .clone(),
        );
    write_json(
        root,
        &format!("builds/services/{service_id_path}/{build_hash}.json"),
        build_record,
    );
}

fn service_id_path_for_test(service_id: &str) -> String {
    storage_segment_for_test(service_id)
}

fn storage_segment_for_test(publication_id: &str) -> String {
    publication_id.replace('.', "~").replace('/', "~~")
}

fn build_identity(character: char) -> String {
    format!(
        "skiff-service-build-v1:sha256:{}",
        character.to_string().repeat(64)
    )
}

fn build_identity_for_version_pointer() -> String {
    build_identity('e')
}

fn write_json(root: &Path, relative_path: &str, value: Value) {
    let path = root.join(relative_path);
    let value = canonicalize_test_artifact_json(value);
    fs::create_dir_all(
        path.parent()
            .expect("test artifact path should have parent"),
    )
    .expect("artifact directory should be created");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("test JSON should serialize"),
    )
    .expect("test artifact should be written");
}

fn read_json(root: &Path, relative_path: &str) -> Value {
    let path = root.join(relative_path);
    serde_json::from_slice(&fs::read(path).expect("test artifact should be readable"))
        .expect("test artifact should be valid JSON")
}

fn canonicalize_test_artifact_json(mut value: Value) -> Value {
    match value.get("schemaVersion").and_then(Value::as_str) {
        Some("skiff-file-ir-v3") => canonicalize_test_file_ir_json(value),
        Some("skiff-package-unit-v1") => {
            resolve_file_identity_aliases(&mut value);
            canonicalize_test_package_unit_json(value)
        }
        Some("skiff-service-unit-v1") => {
            resolve_file_identity_aliases(&mut value);
            resolve_package_abi_identity_aliases(&mut value);
            value
        }
        _ => value,
    }
}

fn canonicalize_test_file_ir_json(mut value: Value) -> Value {
    let declared = value
        .get("fileIrIdentity")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let Ok(unit) = serde_json::from_value::<skiff_artifact_model::FileIrUnit>(value.clone()) else {
        return value;
    };
    let identity = file_ir_identity(&unit).expect("test File IR identity should compute");
    value["fileIrIdentity"] = json!(identity.clone());
    value["sourceAstHash"] = json!(format!("source:{identity}"));
    if !declared.is_empty() {
        FILE_IDENTITY_ALIASES.with(|aliases| {
            aliases.borrow_mut().insert(declared, identity);
        });
    }
    value
}

fn canonicalize_test_package_unit_json(mut value: Value) -> Value {
    let declared_build = value
        .get("buildIdentity")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let declared_abi = value
        .get("abiIdentity")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let Ok(unit) = serde_json::from_value::<skiff_artifact_model::PackageUnit>(value.clone())
    else {
        return value;
    };
    let build_identity =
        package_build_identity(&unit).expect("test package build identity should compute");
    let abi_identity =
        package_abi_identity(&unit).expect("test package ABI identity should compute");
    value["buildIdentity"] = json!(build_identity.clone());
    value["abiIdentity"] = json!(abi_identity.clone());
    if !declared_build.is_empty() {
        PACKAGE_BUILD_IDENTITY_ALIASES.with(|aliases| {
            aliases.borrow_mut().insert(declared_build, build_identity);
        });
    }
    if !declared_abi.is_empty() {
        PACKAGE_ABI_IDENTITY_ALIASES.with(|aliases| {
            aliases.borrow_mut().insert(declared_abi, abi_identity);
        });
    }
    value
}

fn resolve_file_identity_aliases(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(identity) = object
                .get("fileIrIdentity")
                .and_then(Value::as_str)
                .and_then(resolve_file_identity_alias)
            {
                object.insert("fileIrIdentity".to_string(), json!(identity.clone()));
                if object.contains_key("sourceAstHash") {
                    object.insert(
                        "sourceAstHash".to_string(),
                        json!(format!("source:{identity}")),
                    );
                }
            }
            for child in object.values_mut() {
                resolve_file_identity_aliases(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_file_identity_aliases(item);
            }
        }
        _ => {}
    }
}

fn resolve_file_identity_alias(identity: &str) -> Option<String> {
    FILE_IDENTITY_ALIASES.with(|aliases| aliases.borrow().get(identity).cloned())
}

fn resolve_package_abi_identity_aliases(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(identity) = object
                .get("abiIdentity")
                .and_then(Value::as_str)
                .and_then(resolve_package_abi_identity_alias)
            {
                object.insert("abiIdentity".to_string(), json!(identity));
            }
            for child in object.values_mut() {
                resolve_package_abi_identity_aliases(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_package_abi_identity_aliases(item);
            }
        }
        _ => {}
    }
}

fn resolve_package_abi_identity_alias(identity: &str) -> Option<String> {
    PACKAGE_ABI_IDENTITY_ALIASES
        .with(|aliases| aliases.borrow().get(identity).cloned())
        .or_else(|| match identity {
            "abi:pkg" | "abi:actual" | "abi:http-session" => {
                Some(empty_package_abi_identity_for_test())
            }
            _ => None,
        })
}

fn empty_package_abi_identity_for_test() -> String {
    let unit = skiff_artifact_model::PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    package_abi_identity(&unit).expect("empty package ABI identity should compute")
}

fn write_json_raw(root: &Path, relative_path: &str, value: Value) {
    let path = root.join(relative_path);
    fs::create_dir_all(
        path.parent()
            .expect("test artifact path should have parent"),
    )
    .expect("artifact directory should be created");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("test JSON should serialize"),
    )
    .expect("test artifact should be written");
}

// ── Verification case #23 ─────────────────────────────────────────────────────
// Architecture invariant: File IR `type_index` cannot be used outside its
// owning file without owner context.  The linker converts every `LocalType
// { type_index }` to `Address { addr: TypeAddr }`, which carries the full
// `UnitAddr + FileAddr + type_index`.  A bare `type_index` that falls outside
// the owning file's type table must be rejected at link time, not silently
// accepted or interpreted as a cross-file index.

#[test]
fn linker_rejects_local_type_ref_with_out_of_bounds_type_index() {
    // Build a file IR that has exactly one entry in typeTable (index 0).
    // The type descriptor references `localType { typeIndex: 1 }` — which is
    // out of bounds for this file.  The linker must detect this and reject it,
    // because `type_index` is file-local: there is no global index 1.
    let temp = TempDir::new("runtime-case23-local-type-oob");
    let root = temp.path().join("artifacts");
    write_json(
        &root,
        "units/files/service.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:service",
            "sourceAstHash": "source:file:service",
            "modulePath": "svc.main",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {},
            // typeTable has ONE entry (index 0 = "Owner").
            // Its descriptor points at localType index 1, which does not exist
            // in this file — demonstrating that type_index is file-local.
            "typeTable": [
                {
                    "name": "Owner",
                    "descriptor": {
                        "kind": "record",
                        "fields": {
                            "child": {
                                "kind": "localType",
                                "typeIndex": 1   // out of bounds: only index 0 exists
                            }
                        }
                    }
                }
            ],
            "executables": [],
            "externalRefs": {}
        }),
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("localType ref with out-of-bounds typeIndex must be rejected at link time");

    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("out of bounds") || error_text.contains("type index"),
        "expected type-index-out-of-bounds error, got: {error_text}"
    );
}

#[test]
fn linker_accepts_local_type_ref_within_bounds_and_resolves_to_owner_context() {
    // Counterpart: a `localType { typeIndex: 0 }` that IS in bounds for the
    // owning file must succeed, and the resulting `Address` must carry the
    // correct owner context (UnitAddr::Service, file index 0, type_index 0).
    // This confirms the invariant from the positive side.
    let temp = TempDir::new("runtime-case23-local-type-in-bounds");
    let root = temp.path().join("artifacts");
    write_json(
        &root,
        "units/files/service.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:service",
            "sourceAstHash": "source:file:service",
            "modulePath": "svc.main",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {},
            "typeTable": [
                {
                    "name": "Inner",
                    "descriptor": { "kind": "record", "fields": {} }
                },
                {
                    "name": "Wrapper",
                    "descriptor": {
                        "kind": "record",
                        "fields": {
                            "inner": {
                                "kind": "localType",
                                "typeIndex": 0  // refers to "Inner" in this file — valid
                            }
                        }
                    }
                }
            ],
            "executables": [],
            "externalRefs": {}
        }),
    );
    write_service_unit(
        &root,
        "units/services/svc-v1.json",
        service_unit_json(
            "example.com/svc",
            "v1",
            "file:service",
            "units/files/service.json",
            Vec::new(),
            Vec::new(),
        ),
    );
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    let program = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("localType ref within bounds should link successfully");

    // After linking, the descriptor for Wrapper (index 1) must have its
    // localType field resolved to Address { UnitAddr::Service, file 0,
    // type_index 0 } — confirming that the type_index is scoped to its
    // owning file (service, file index 0), not to a global namespace.
    let wrapper_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index: 1,
    };
    let inner_expected = TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let descriptor = program
        .image
        .types
        .descriptor(&wrapper_addr)
        .expect("Wrapper descriptor should be linked");
    match descriptor {
        LinkedTypeDescriptor::Record { fields } => {
            let inner_ref = fields
                .get("inner")
                .expect("Wrapper should have 'inner' field");
            assert_eq!(
                inner_ref,
                &crate::program::LinkedTypeRef::Address {
                    addr: inner_expected
                },
                "localType in bounds must resolve to Address with owning file context"
            );
        }
        other => panic!("expected Record descriptor, got {other:?}"),
    }
}

// ── Verification case #25 ─────────────────────────────────────────────────────
// Architecture invariant: runtime call execution does not parse source display
// paths to find symbols.  The linker resolves `ExternalServiceSymbol` calls
// using structured `ServiceSymbolKey { module_path, symbol }` keys, NOT by
// splitting dotted display strings.

#[test]
fn linker_resolves_cross_file_call_using_structured_module_path_and_symbol() {
    // Case #25: runtime call execution does not parse source display paths
    // to find symbols.
    //
    // Two service files: "svc.caller" calls "svc.callee"/"compute" via
    // ExternalServiceSymbol { modulePath: "svc.callee", symbol: "compute" }.
    // The linker must find the target using the structured key pair, not by
    // splitting a dotted display string.
    let temp = TempDir::new("runtime-case25-structured-symbol-key");
    let root = temp.path().join("artifacts");

    // File for "svc.callee": provides "compute" as a link target.
    write_json(
        &root,
        "units/files/file_callee.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:callee",
            "sourceAstHash": "source:file:callee",
            "modulePath": "svc.callee",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {
                "executables": { "compute": { "executableIndex": 0 } }
            },
            "typeTable": [],
            "executables": [
                {
                    "kind": "function",
                    "symbol": "compute",
                    "returnType": { "kind": "builtin", "name": "number" },
                    "slots": { "slots": [], "frameSize": 0 },
                    "maySuspend": false,
                    "body": {}
                }
            ],
            "externalRefs": {}
        }),
    );

    // File for "svc.caller": calls compute via structured ExternalServiceSymbol.
    write_json(
        &root,
        "units/files/file_caller.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:caller",
            "sourceAstHash": "source:file:caller",
            "modulePath": "svc.caller",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {
                "executables": { "run": { "executableIndex": 0 } }
            },
            "typeTable": [],
            "executables": [
                {
                    "kind": "function",
                    "symbol": "run",
                    "returnType": { "kind": "builtin", "name": "number" },
                    "slots": { "slots": [], "frameSize": 0 },
                    "maySuspend": false,
                    "body": {
                        "expressions": [
                            {
                                "kind": "call",
                                "call": {
                                    // Structured key: modulePath + symbol separately.
                                    // The linker resolves ("svc.callee", "compute") —
                                    // it never parses the display path "svc.callee.compute".
                                    "target": {
                                        "kind": "externalServiceSymbol",
                                        "symbol": {
                                            "modulePath": "svc.callee",
                                            "symbol": "compute"
                                        }
                                    },
                                    "args": [],
                                    "typeArgs": {}
                                }
                            }
                        ]
                    }
                }
            ],
            "externalRefs": {
                "serviceSymbols": [
                    { "modulePath": "svc.callee", "symbol": "compute" }
                ]
            }
        }),
    );

    // Build the service unit JSON manually so we can set module paths precisely.
    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc", "displayName": "Service" },
        "version": "v1",
        "protocolIdentity": "protocol:svc",
        "publicationAbi": publication_abi_json(
            "example.com/svc",
            "v1",
            "protocol:svc",
            "svc.caller.run",
            json!({ "kind": "builtin", "name": "number" }),
        ),
        "files": [
            {
                "fileIrIdentity": "file:caller",
                "modulePath": "svc.caller",
                "artifactPath": "units/files/file_caller.json",
                "sourceAstHash": "source:file:caller"
            },
            {
                "fileIrIdentity": "file:callee",
                "modulePath": "svc.callee",
                "artifactPath": "units/files/file_callee.json",
                "sourceAstHash": "source:file:callee"
            }
        ],
        "packageDependencies": [],
        "packageAbiExpectations": [],
        "operations": [
            service_operation_json("svc.caller.run", "file:caller", 0, json!({
                "kind": "builtin",
                "name": "number"
            }))
        ],
        "gateway": {},
        "config": {}
    });
    write_service_unit(&root, "units/services/svc-v1.json", service);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    // Structured ExternalServiceSymbol must resolve successfully (case #25).
    load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect("ExternalServiceSymbol with structured modulePath+symbol must link (case #25)");
}

#[test]
fn linker_rejects_external_service_symbol_with_display_path_as_symbol_name() {
    // Companion test to case #25: the symbol field contains the full dotted
    // display path "svc.callee.compute" with an empty modulePath.
    // The linker uses ServiceSymbolKey { module_path, symbol } equality —
    // ("", "svc.callee.compute") ≠ ("svc.callee", "compute") — so the
    // call target must not resolve via display-path parsing.
    let temp = TempDir::new("runtime-case25-display-path-rejected");
    let root = temp.path().join("artifacts");

    write_json(
        &root,
        "units/files/file_callee.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:callee",
            "sourceAstHash": "source:file:callee",
            "modulePath": "svc.callee",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {
                "executables": { "compute": { "executableIndex": 0 } }
            },
            "typeTable": [],
            "executables": [
                {
                    "kind": "function",
                    "symbol": "compute",
                    "returnType": { "kind": "builtin", "name": "number" },
                    "slots": { "slots": [], "frameSize": 0 },
                    "maySuspend": false,
                    "body": {}
                }
            ],
            "externalRefs": {}
        }),
    );

    write_json(
        &root,
        "units/files/file_caller.json",
        json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:caller",
            "sourceAstHash": "source:file:caller",
            "modulePath": "svc.caller",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {
                "executables": { "run": { "executableIndex": 0 } }
            },
            "typeTable": [],
            "executables": [
                {
                    "kind": "function",
                    "symbol": "run",
                    "returnType": { "kind": "builtin", "name": "number" },
                    "slots": { "slots": [], "frameSize": 0 },
                    "maySuspend": false,
                    "body": {
                        "expressions": [
                            {
                                "kind": "call",
                                "call": {
                                    // BAD: using the dotted display path as the symbol
                                    // with an empty modulePath.
                                    // ("", "svc.callee.compute") ≠ ("svc.callee", "compute")
                                    // so this must NOT resolve.
                                    "target": {
                                        "kind": "externalServiceSymbol",
                                        "symbol": {
                                            "modulePath": "",
                                            "symbol": "svc.callee.compute"
                                        }
                                    },
                                    "args": [],
                                    "typeArgs": {}
                                }
                            }
                        ]
                    }
                }
            ],
            "externalRefs": {
                "serviceSymbols": [
                    { "modulePath": "", "symbol": "svc.callee.compute" }
                ]
            }
        }),
    );

    let service = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "service": { "id": "example.com/svc", "displayName": "Service" },
        "version": "v1",
        "protocolIdentity": "protocol:svc",
        "publicationAbi": publication_abi_json(
            "example.com/svc",
            "v1",
            "protocol:svc",
            "svc.caller.run",
            json!({ "kind": "builtin", "name": "number" }),
        ),
        "files": [
            {
                "fileIrIdentity": "file:caller",
                "modulePath": "svc.caller",
                "artifactPath": "units/files/file_caller.json",
                "sourceAstHash": "source:file:caller"
            },
            {
                "fileIrIdentity": "file:callee",
                "modulePath": "svc.callee",
                "artifactPath": "units/files/file_callee.json",
                "sourceAstHash": "source:file:callee"
            }
        ],
        "packageDependencies": [],
        "packageAbiExpectations": [],
        "operations": [
            service_operation_json("svc.caller.run", "file:caller", 0, json!({
                "kind": "builtin",
                "name": "number"
            }))
        ],
        "gateway": {},
        "config": {}
    });
    write_service_unit(&root, "units/services/svc-v1.json", service);
    write_release_pointer(
        &root,
        "example.com/svc",
        "v1",
        &build_identity_for_version_pointer(),
        "units/services/svc-v1.json",
    );

    // ("", "svc.callee.compute") must not match the ("svc.callee", "compute")
    // link target.  The linker must not fall back to display-path parsing.
    let error = load_test_layers_at_root(
        &root,
        RuntimeProgramArtifactSelection::release("example.com/svc", "v1"),
    )
    .expect_err("ExternalServiceSymbol with display-path-as-symbol must not resolve (case #25)");

    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("unresolved")
            || error_text.contains("not found")
            || error_text.contains("LinkSymbol"),
        "expected link-symbol-unresolved error, got: {error_text}"
    );
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
