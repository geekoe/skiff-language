use std::collections::BTreeMap;

use serde_json::{json, Value};
use skiff_artifact_model::{
    CallIr, CallTargetIr, CallableEffectSummary, CallableMayEffects,
    CanonicalPublicCallableSignature, ConfigAndEffectMetadata, ContractOperationId,
    DbDeclarationIr, DbFieldStorageIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, ExprIr,
    FileIrRef, FileIrUnit, FunctionTypeParamIr, InterfaceInstantiationRef, MetadataValue,
    OperationAbiRef, OperationCallableKind, OperationTargetRef, PackageCallableId,
    PackageCallableRef, PackageDependencyConstraint, PackageDependencyPublicLinkScope,
    PackageOperationTarget, PackageProductionLinkScope, PackageRefIr, PackageTestAssembly,
    PackageTestAssemblyKind, PackageTestEntrypoint, PackageTestEntrypointKind,
    PackageTestExecutableRef, PackageTestFileIrRef, PackageTestFileLinkScope,
    PackageTestLinkPolicy, PackageTestPackageUnitRef, PackageTestRuntimeExpectedError, PackageUnit,
    PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, PublicationResourceRef, PublicationSchemaType,
    PublicationSchemaTypeNameability, ServiceCallRef, ServiceCallRefIndex, ServiceOperation,
    ServiceProtocolIdentity, ServiceUnit, SourceCallMethodIndexEntry,
    SourceCallOperationIndexEntry, SourceMapSource, TypeRefIr,
};

use super::*;

mod artifact_reference;
mod canonical_compile_contract;
mod file_ir;
mod framing;
mod golden;
mod legacy_service;
mod operation;
mod package;
mod package_mutation_matrix;
mod package_test;
mod publication_validation;
mod runtime_program;
mod semantic;

#[test]
fn service_protocol_identity_hash_accepts_only_canonical_v2_identity() {
    let hash = "a".repeat(64);
    let identity = format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{hash}");
    assert_eq!(
        service_protocol_identity_hash(&identity).expect("canonical identity"),
        hash
    );

    for invalid in [
        format!("skiff-protocol-v1:sha256:{hash}"),
        SERVICE_PROTOCOL_IDENTITY_PREFIX.to_string(),
        format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", "a".repeat(63)),
        format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", "A".repeat(64)),
        format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", "g".repeat(64)),
    ] {
        assert!(
            service_protocol_identity_hash(&invalid).is_err(),
            "{invalid} must be rejected"
        );
    }
}

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
        file_ir_identity: "skiff-file-ir-v5:sha256:testfile".to_string(),
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
            build_identity: "skiff-package-build-v2:sha256:prod".to_string(),
            unit_path: "units/packages/example.com/pkg/prod.json".to_string(),
            public_abi_identity: "skiff-package-local-abi-v2:sha256:prodabi".to_string(),
            implementation_links_identity: "sha256:prodlinks".to_string(),
        },
        test_files: vec![owner_test_file.clone()],
        dependency_package_units: vec![PackageTestPackageUnitRef {
            package_id: "example.com/dep".to_string(),
            version: "1.0.0".to_string(),
            build_identity: "skiff-package-build-v2:sha256:dep".to_string(),
            unit_path: "units/packages/example.com/dep/dep.json".to_string(),
            public_abi_identity: "skiff-package-local-abi-v2:sha256:depabi".to_string(),
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
                build_identity: "skiff-package-build-v2:sha256:prod".to_string(),
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
                build_identity: "skiff-package-build-v2:sha256:dep".to_string(),
                public_abi_identity: "skiff-package-local-abi-v2:sha256:depabi".to_string(),
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
    unit.publication_abi = publication_abi_fixture();
    let operation = unit.publication_abi.operation_exports[0].clone();
    unit.config_and_effect_metadata.effects.operations.insert(
        operation.operation_abi_id.clone(),
        CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                writes_caller_reachable: body_seed == "changed",
                returns_caller_alias: false,
                throws_caller_alias: false,
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_suspend: false,
            },
        },
    );
    let file = FileIrRef {
        file_ir_identity: "skiff-file-ir-v5:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        module_path: "pkg.main".to_string(),
        artifact_path: Some("units/files/pkg.json".into()),
        source_ast_hash: Some("source".into()),
    };
    unit.files.push(file.clone());
    unit.implementation_links.functions.insert(
        "run".to_string(),
        skiff_artifact_model::ExecutableExport {
            file: file.clone(),
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
    unit.implementation_links.operation_targets.insert(
        operation.operation_abi_id.clone(),
        PackageOperationTarget::LocalExecutable {
            operation,
            target: OperationTargetRef {
                file_ref: file,
                executable_index: 0,
                callable_abi_id: "callable:run".to_string(),
                callable_kind: OperationCallableKind::PublicFunction,
            },
        },
    );
    assign_package_unit_identities(&mut unit).expect("package fixture identities");
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
