use std::collections::BTreeMap;

use serde_json::{json, Value};
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, ConfigAndEffectMetadata, DbDeclarationIr, DbFieldStorageIr,
    DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
    InterfaceInstantiationRef, MetadataValue, OperationAbiRef, OperationCallableKind,
    OperationTargetRef, PackageDependencyConstraint, PackageDependencyPublicLinkScope,
    PackageProductionLinkScope, PackageTestAssembly, PackageTestAssemblyKind,
    PackageTestEntrypoint, PackageTestEntrypointKind, PackageTestExecutableRef,
    PackageTestFileIrRef, PackageTestFileLinkScope, PackageTestLinkPolicy,
    PackageTestPackageUnitRef, PackageTestRuntimeExpectedError, PackageUnit, PublicationAbiUnit,
    PublicationOperationAbi, PublicationOperationKind, PublicationPublicInstanceExport,
    PublicationResourceRef, PublicationSchemaType, PublicationSchemaTypeNameability,
    ServiceOperation, ServiceUnit, SourceCallMethodIndexEntry, SourceCallOperationIndexEntry,
    SourceMapSource, TypeRefIr,
};

use super::*;

mod file_ir;
mod golden;
mod legacy_service;
mod operation;
mod package;
mod package_test;
mod publication_validation;
mod runtime_program;
mod semantic;

fn resource_ref(path: &str, sha256: &str) -> PublicationResourceRef {
    PublicationResourceRef {
        path: path.to_string(),
        sha256: sha256.to_string(),
        byte_len: sha256.len() as u64,
        content_type: None,
        artifact_path: Some(format!("resources/sha256/{sha256}")),
    }
}

fn package_test_assembly_fixture() -> PackageTestAssembly {
    let owner_test_file = PackageTestFileIrRef {
        file_ir_identity: "skiff-file-ir-v3:sha256:testfile".to_string(),
        file_ir_path: "units/files/test.json".to_string(),
        source_path: "tests/pkg.test.skiff".to_string(),
        module_path: "pkg.test".to_string(),
    };
    let entrypoint_local_id = package_test_entrypoint_local_id(
        "example.com/pkg",
        "1.0.0",
        "tests/pkg.test.skiff",
        0,
        "runs internal helper",
    )
    .expect("entrypoint local id");

    PackageTestAssembly {
        schema_version: "skiff-package-test-assembly-v1".to_string(),
        kind: PackageTestAssemblyKind::PackageTest,
        package_id: "example.com/pkg".to_string(),
        package_version: "1.0.0".to_string(),
        test_build_identity: "skiff-package-test-build-v1:sha256:stale".to_string(),
        production_package_unit: PackageTestPackageUnitRef {
            package_id: "example.com/pkg".to_string(),
            version: "1.0.0".to_string(),
            build_identity: "skiff-package-build-v1:sha256:prod".to_string(),
            unit_path: "units/packages/example.com/pkg/prod.json".to_string(),
            public_abi_identity: "skiff-package-abi-v1:sha256:prodabi".to_string(),
            implementation_links_identity: "sha256:prodlinks".to_string(),
        },
        test_files: vec![owner_test_file.clone()],
        dependency_package_units: vec![PackageTestPackageUnitRef {
            package_id: "example.com/dep".to_string(),
            version: "1.0.0".to_string(),
            build_identity: "skiff-package-build-v1:sha256:dep".to_string(),
            unit_path: "units/packages/example.com/dep/dep.json".to_string(),
            public_abi_identity: "skiff-package-abi-v1:sha256:depabi".to_string(),
            implementation_links_identity: "sha256:deplinks".to_string(),
        }],
        test_entrypoints: vec![PackageTestEntrypoint {
            kind: PackageTestEntrypointKind::TestOnly,
            entrypoint_local_id: entrypoint_local_id.clone(),
            entrypoint_id: "skiff-package-test-entrypoint-v1:sha256:stale".to_string(),
            display_name: "runs internal helper".to_string(),
            source_path: "tests/pkg.test.skiff".to_string(),
            module_path: "pkg.test".to_string(),
            owner_test_file: owner_test_file.clone(),
            executable_ref: PackageTestExecutableRef {
                file_ir_identity: owner_test_file.file_ir_identity.clone(),
                executable_index: 0,
                executable_local_id: "test-entrypoint-0".to_string(),
                symbol: Some("__skiff_package_test_0".to_string()),
            },
            default_run: true,
            config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            runtime_expected_error: Some(PackageTestRuntimeExpectedError {
                code: "ProviderUnavailableError".to_string(),
                message_contains: Some("offline".to_string()),
            }),
        }],
        link_policy: PackageTestLinkPolicy {
            current_package_production: PackageProductionLinkScope {
                package_id: "example.com/pkg".to_string(),
                version: "1.0.0".to_string(),
                build_identity: "skiff-package-build-v1:sha256:prod".to_string(),
                files_digest: "sha256:prodfiles".to_string(),
                implementation_links_digest: "sha256:prodlinks".to_string(),
                allow_private: true,
            },
            test_file_scopes: vec![PackageTestFileLinkScope {
                owner_test_file_identity: owner_test_file.file_ir_identity.clone(),
                source_path: owner_test_file.source_path.clone(),
                module_path: owner_test_file.module_path.clone(),
                allowed_local_link_digest: "sha256:testlinks".to_string(),
                entrypoint_local_ids: vec![entrypoint_local_id],
            }],
            dependency_public_scopes: vec![PackageDependencyPublicLinkScope {
                package_id: "example.com/dep".to_string(),
                version: "1.0.0".to_string(),
                build_identity: "skiff-package-build-v1:sha256:dep".to_string(),
                public_abi_identity: "skiff-package-abi-v1:sha256:depabi".to_string(),
                public_export_digest: "sha256:depexports".to_string(),
                implementation_links_digest: "sha256:deplinks".to_string(),
                allow_private: false,
            }],
        },
        config_and_effect_metadata: ConfigAndEffectMetadata::default(),
        source_map: json!({ "sources": [] }),
    }
}

fn package_fixture(body_seed: &str) -> PackageUnit {
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    unit.config_and_effect_metadata
        .effects
        .entry("bodySeed".to_string())
        .or_default()
        .metadata
        .insert(
            "value".to_string(),
            MetadataValue::String(body_seed.to_string()),
        );
    unit.implementation_links.functions.insert(
            "run".to_string(),
            skiff_artifact_model::ExecutableExport {
                file: FileIrRef {
                    file_ir_identity: "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    module_path: "pkg.main".to_string(),
                    artifact_path: Some("units/files/pkg.json".into()),
                    source_ast_hash: Some("source".into()),
                },
                executable_index: 0,
                symbol: "run".to_string(),
                signature: skiff_artifact_model::ExecutableSignatureIr {
                    params: Vec::new(),
                    return_type: TypeRefIr::native("string"),
                    self_type: None,
                    may_suspend: false,
                },
            },
        );
    unit.publication_abi = publication_abi_fixture();
    unit.publication_abi.abi_identity =
        publication_abi_identity(&unit.publication_abi).expect("publication abi identity");
    unit
}

fn publication_abi_fixture() -> PublicationAbiUnit {
    let public_signature = CanonicalPublicCallableSignature {
        params: Vec::new(),
        return_type: TypeRefIr::native("string"),
        may_suspend: false,
    };
    let operation = OperationAbiRef {
        operation_abi_id: public_function_operation_abi_id(
            "run",
            &public_signature,
            &[],
            &BTreeMap::new(),
        )
        .expect("operation ABI identity"),
        kind: PublicationOperationKind::PublicFunction,
        public_path: "run".to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: "run".to_string(),
    };
    let mut unit = PublicationAbiUnit::empty("example.com/pkg", "1.0.0", "");
    unit.operation_exports.push(operation.clone());
    unit.operation_abi.push(PublicationOperationAbi {
        operation: operation.clone(),
        public_signature,
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    });
    unit.source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path: "run".to_string(),
            operation,
        });
    unit
}
