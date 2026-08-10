use std::{collections::HashMap, sync::Arc};

use super::*;
use crate::{
    ExecutableKind, ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay,
    LinkedExecutableBody, PackageCodeSlotIndex, ParamIr, RuntimeExecutionPackage,
    RuntimeTypeContext, SlotLayoutIr, SourceMapDto,
};

#[test]
fn resolve_executable_borrows_file_body_without_cloning() {
    let file = Arc::new(file_unit("file:service", "service.entry"));
    let body_ptr = &file.executables[0].body as *const LinkedExecutableBody;
    let executable_ptr = &file.executables[0] as *const LinkedExecutable;
    let image = image(vec![Arc::clone(&file)], Vec::new());

    let resolved = image
        .resolve_executable(&ExecutableAddr::service(0, 0))
        .expect("expected executable to resolve");

    assert!(Arc::ptr_eq(resolved.file_arc, &file));
    assert_eq!(
        resolved.executable as *const LinkedExecutable,
        executable_ptr
    );
    assert_eq!(
        &resolved.executable.body as *const LinkedExecutableBody,
        body_ptr
    );
}

#[test]
fn package_slot_and_file_identity_resolve_expected_file() {
    let package_file_a = Arc::new(file_unit("file:pkg:a", "pkg.a"));
    let package_file_b = Arc::new(file_unit("file:pkg:b", "pkg.b"));
    let image = image(
        Vec::new(),
        vec![vec![
            Arc::clone(&package_file_a),
            Arc::clone(&package_file_b),
        ]],
    );

    let resolved_by_index = image
        .resolve_executable(&ExecutableAddr::package(0, 1, 0))
        .expect("expected package executable to resolve by loaded file index");
    let resolved_by_identity = image
        .resolve_executable(&ExecutableAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::file_ir_identity("file:pkg:b"),
            executable: 0,
        })
        .expect("expected package executable to resolve by file identity");

    assert!(Arc::ptr_eq(resolved_by_index.file_arc, &package_file_b));
    assert!(Arc::ptr_eq(resolved_by_identity.file_arc, &package_file_b));
    assert_eq!(resolved_by_index.executable.symbol, "pkg.b");
}

#[test]
fn out_of_bounds_resolution_returns_clear_errors() {
    let file = Arc::new(file_unit("file:service", "service.entry"));
    let image = image(vec![file], Vec::new());

    assert_eq!(
        image
            .resolve_executable(&ExecutableAddr::package(1, 0, 0))
            .expect_err("expected package slot error"),
        LinkedProgramResolveError::PackageSlotOutOfBounds {
            slot: 1,
            package_count: 0,
        }
    );
    assert_eq!(
        image
            .resolve_executable(&ExecutableAddr::service(2, 0))
            .expect_err("expected file index error"),
        LinkedProgramResolveError::FileIndexOutOfBounds {
            unit: UnitAddr::Service,
            index: 2,
            file_count: 1,
        }
    );
    assert_eq!(
        image
            .resolve_executable(&ExecutableAddr::service(0, 2))
            .expect_err("expected executable index error"),
        LinkedProgramResolveError::ExecutableIndexOutOfBounds {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            index: 2,
            executable_count: 1,
        }
    );
}

fn image(
    service_files: Vec<Arc<LinkedFileUnit>>,
    package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
) -> LinkedProgramImage {
    let packages = package_files
        .into_iter()
        .enumerate()
        .map(|(slot, files)| {
            let file_refs = files
                .iter()
                .map(|file| {
                    serde_json::json!({
                        "fileIrIdentity": file.file_ir_identity,
                        "modulePath": file.module_path,
                        "sourceAstHash": file.source_ast_hash,
                    })
                })
                .collect::<Vec<_>>();
            let package_id = format!("test.package.{slot}");
            let bytecode_statement_manifest_identity =
                skiff_artifact_model::derive_bytecode_statement_manifest_identity(&package_id, &[])
                    .expect("empty package statement manifest should be canonical");
            let artifact: skiff_artifact_model::PackageArtifact =
                serde_json::from_value(serde_json::json!({
                "schemaVersion": skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION,
                "packageId": package_id,
                "packageVersion": "1.0.0",
                "packageBuildId": format!("test-build:{slot}"),
                "files": file_refs,
                "staticResources": [],
                "bytecodeStatementManifestIdentity": bytecode_statement_manifest_identity,
                "packageLocalAbi": {
                    "localAbiIdentity": format!("test-abi:{slot}"),
                    "publicSymbols": {}
                },
                "packageSchemaIndex": {
                    "packageId": package_id,
                    "packageSchemaIndexIdentity": format!("test-schema:{slot}")
                },
                "packageSchemaTypeRecords": {},
                "implementationLinks": {},
                "callableLinks": {},
                "syntheticCallbackOwners": [],
                "bytecodeSchemaRecords": {},
                "actorImplementations": [],
                "localInterfaceConformances": [],
                "packageRequirements": [],
                "contractRequirements": [],
                "serviceRequirements": [],
                "runtimeRequirements": {
                    "config": []
                },
                "callableSemanticFacts": {},
                "boundaryProjections": {},
                "serviceCallRefs": []
                }))
                .unwrap();
            assert!(artifact.actor_implementations.is_empty());
            assert!(artifact.local_interface_conformances.is_empty());
            RuntimeExecutionPackage::try_new(
                PackageCodeSlotIndex::new(slot),
                Arc::new(artifact),
                files,
                Default::default(),
            )
            .map(Arc::new)
            .unwrap()
        })
        .collect();
    LinkedProgramImage {
        service_files,
        packages,
        service_resources: Default::default(),
        routes: HashMap::new(),
        task_routes: HashMap::new(),
        operations: HashMap::new(),
        operation_receivers: HashMap::new(),
        link_overlay: LinkOverlay::default(),
        types: RuntimeTypeContext::default(),
    }
}

fn file_unit(identity: &str, symbol: &str) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: identity.to_string(),
        source_ast_hash: format!("source:{identity}"),
        module_path: if symbol.starts_with("pkg.") {
            "pkg.main".to_string()
        } else {
            "svc.main".to_string()
        },
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![executable(symbol)],
        external_refs: ExternalRefTable::default(),
    }
}

fn executable(symbol: &str) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::<ParamIr>::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: LinkedExecutableBody::default(),
    }
}
