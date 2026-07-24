use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    ConstExport, ContractOperationId, ContractRequirement, ExecutableExport, ExecutableSignatureIr,
    FileIrRef, PackageCallableParameter, PackageCallableSignature, PackageExportIndex,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRequirement, PackageResourceRequirement,
    PackageRuntimeCapabilityRequirement, PackageRuntimeRequirements, PackageTypeRef,
    ServiceCallRef, ServiceProtocolIdentity, ServiceRequirement, TypeRefIr, ValueProvenance,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionPackageCallableKey, ProjectionPackageCallableSignatureFacts,
};

use crate::package_artifact::{
    api_exports::{PackageExportPublicInstance, PackageExportSymbol, PackageExports},
    export_links::{
        PackagePublicInstanceExecutionLink, PackagePublicInstanceMethodExecutionLink,
        ProjectedPackageExportLinks,
    },
    projection::{project_package_artifact_facts, ProjectedPackageFacts},
};

#[derive(Clone, Copy)]
pub(super) enum SignatureSet {
    Complete,
    ExactTyped,
    Missing,
    Extra,
    TargetMismatch,
}

pub(super) fn project_fixture(
    signature_set: SignatureSet,
    runtime_capability: &str,
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    project_fixture_with_runtime_requirements(
        signature_set,
        runtime_requirements(runtime_capability),
    )
}

pub(super) fn project_fixture_with_runtime_requirements(
    signature_set: SignatureSet,
    runtime_requirements: PackageRuntimeRequirements,
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    let file_ref = file_ref();
    let export_index = PackageExportIndex {
        functions: BTreeMap::from([
            (
                "mutate".to_string(),
                executable_export(&file_ref, 1, "mutate"),
            ),
            ("run".to_string(), executable_export(&file_ref, 0, "run")),
        ]),
        impl_methods: BTreeMap::from([("Worker.handle".to_string(), receiver_export(&file_ref))]),
        ..PackageExportIndex::default()
    };
    let api_exports = PackageExports {
        entries: Vec::new(),
        symbols: BTreeMap::from([
            (
                "mutate".to_string(),
                PackageExportSymbol {
                    module: "api".to_string(),
                    symbol: "mutate".to_string(),
                },
            ),
            (
                "run".to_string(),
                PackageExportSymbol {
                    module: "api".to_string(),
                    symbol: "run".to_string(),
                },
            ),
        ]),
        public_instances: vec![PackageExportPublicInstance {
            public_path: "worker".to_string(),
            module: "api".to_string(),
            const_symbol: "worker".to_string(),
            receiver_module: "api".to_string(),
            receiver_symbol: "Worker".to_string(),
            interfaces: Vec::new(),
        }],
    };
    let mut signature_entries = vec![
        (
            callable_key("run", 0),
            signature(TypeRefIr::native("string")),
        ),
        (
            callable_key("mutate", 1),
            signature(TypeRefIr::native("string")),
        ),
        (
            callable_key("worker.handle", 2),
            signature(TypeRefIr::native("string")),
        ),
    ];
    match signature_set {
        SignatureSet::Complete => {}
        SignatureSet::ExactTyped => signature_entries[0].1 = exact_typed_signature(),
        SignatureSet::Missing => {
            signature_entries.retain(|(key, _)| key.public_path() != "mutate");
        }
        SignatureSet::Extra => signature_entries.push((
            callable_key("internal", 9),
            signature(TypeRefIr::native("string")),
        )),
        SignatureSet::TargetMismatch => {
            signature_entries[0].0 = callable_key("run", 9);
        }
    }
    let signatures =
        ProjectionPackageCallableSignatureFacts::try_from_entries(signature_entries).unwrap();
    let mut mutate = safe_facts();
    mutate.effects = CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            writes_caller_reachable: true,
            ..no_effects()
        },
    };
    let semantic_facts = BTreeMap::from([
        (ProjectionExecutableKey::new("api", 0), safe_facts()),
        (ProjectionExecutableKey::new("api", 1), mutate),
        (ProjectionExecutableKey::new("api", 2), safe_facts()),
    ]);
    let mut file = skiff_artifact_model::FileIrUnit::empty("api", "source-hash");
    file.file_ir_identity = file_ref.file_ir_identity.clone();
    let protocol_identity = ServiceProtocolIdentity::new("protocol:greeter:v1");
    let operation_id = ContractOperationId::new("operation:greet");
    let contract_requirement = ContractRequirement {
        alias: "greeter_contract".to_string(),
        service_id: "greeter".to_string(),
        contract_version: "1.0.0".to_string(),
        expected_protocol_identity: protocol_identity.clone(),
    };
    let schema_types = BTreeMap::new();
    let schema_index = skiff_artifact_model::PackageSchemaIndex {
        package_id: "example.pkg".to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            "example.pkg",
            &schema_types,
        )
        .unwrap(),
        types: schema_types,
    };
    Ok(project_package_artifact_facts(ProjectedPackageFacts {
        package_id: "example.pkg",
        package_version: "1.0.0",
        api_exports: &api_exports,
        export_links: ProjectedPackageExportLinks {
            exports: export_index,
            public_instances: vec![public_instance(&file_ref)],
        },
        file_ir_units: vec![file],
        resources: Vec::new(),
        package_requirements: vec![PackageRequirement {
            alias: "dependency".to_string(),
            package_id: "example.dependency".to_string(),
            exact_version: "2.0.0".to_string(),
            expected_local_abi: PackageLocalAbiIdentity::new("local-abi:dependency:v2"),
        }],
        contract_requirements: vec![contract_requirement.clone()],
        service_requirements: vec![ServiceRequirement {
            contract_requirement,
            service_binding_slot: 3,
            used_operations: BTreeSet::from([operation_id.clone()]),
        }],
        runtime_requirements,
        callable_semantic_facts: semantic_facts,
        callable_signatures: signatures,
        package_schema_index: schema_index,
        package_schema_type_records: BTreeMap::new(),
        resolved_package_schema_type_records: BTreeMap::new(),
        package_schema_refs_by_source: BTreeMap::new(),
        service_call_refs: vec![ServiceCallRef {
            service_requirement_slot: 3,
            contract_operation_id: operation_id,
            expected_protocol_identity: protocol_identity,
        }],
    })?
    .artifact)
}

fn public_instance(file: &FileIrRef) -> PackagePublicInstanceExecutionLink {
    let interface_type = TypeRefIr::native("WorkerInterface");
    PackagePublicInstanceExecutionLink {
        public_path: "worker".to_string(),
        declared_receiver_type: TypeRefIr::native("Worker"),
        interfaces: vec![interface_type],
        receiver: ConstExport {
            file: file.clone(),
            const_index: 0,
            symbol: "worker".to_string(),
            ty: TypeRefIr::native("Worker"),
        },
        methods: vec![PackagePublicInstanceMethodExecutionLink {
            name: "handle".to_string(),
            public_path: "worker.handle".to_string(),
            executable: receiver_export(file),
        }],
    }
}

fn receiver_export(file: &FileIrRef) -> ExecutableExport {
    let self_ty = TypeRefIr::native("Worker");
    ExecutableExport {
        file: file.clone(),
        executable_index: 2,
        symbol: "Worker.handle".to_string(),
        signature: ExecutableSignatureIr {
            params: vec![
                skiff_artifact_model::ParamIr {
                    name: "self".to_string(),
                    slot: 0,
                    ty: self_ty.clone(),
                },
                skiff_artifact_model::ParamIr {
                    name: "value".to_string(),
                    slot: 1,
                    ty: TypeRefIr::native("string"),
                },
            ],
            return_type: TypeRefIr::native("string"),
            self_type: Some(self_ty),
            may_suspend: false,
        },
    }
}

fn executable_export(file: &FileIrRef, index: u32, symbol: &str) -> ExecutableExport {
    ExecutableExport {
        file: file.clone(),
        executable_index: index,
        symbol: symbol.to_string(),
        signature: ExecutableSignatureIr {
            params: vec![skiff_artifact_model::ParamIr {
                name: "value".to_string(),
                slot: 0,
                ty: TypeRefIr::native("string"),
            }],
            return_type: TypeRefIr::native("string"),
            self_type: None,
            may_suspend: false,
        },
    }
}

pub(super) fn signature(ty: TypeRefIr) -> PackageCallableSignature {
    PackageCallableSignature {
        parameters: vec![PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Local { local_type: ty },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::native("string"),
        },
        throw_types: Vec::new(),
        may_suspend: false,
    }
}

pub(super) fn exact_typed_signature() -> PackageCallableSignature {
    let contract = PackageTypeRef::PackageSchema {
        package_id: "example.payments".to_string(),
        stable_schema_key: "User".to_string(),
        package_schema_type_id: "package-type:user".into(),
    };
    PackageCallableSignature {
        parameters: vec![PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Nullable {
                inner: Box::new(PackageTypeRef::Container {
                    name: "Array".to_string(),
                    arguments: vec![PackageTypeRef::Nullable {
                        inner: Box::new(contract.clone()),
                    }],
                }),
            },
        }],
        return_type: contract,
        throw_types: Vec::new(),
        may_suspend: true,
    }
}

pub(super) fn safe_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: no_effects(),
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}

pub(super) fn runtime_requirements(capability: &str) -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        config: vec![skiff_artifact_model::PackageConfigRequirement {
            path: "app.token".to_string(),
            value_type: "string".to_string(),
            required: true,
        }],
        resources: vec![PackageResourceRequirement {
            key: "database".to_string(),
            capability: "mongodb".to_string(),
        }],
        runtime_capabilities: vec![PackageRuntimeCapabilityRequirement {
            capability: capability.to_string(),
            required_version: "1".to_string(),
        }],
    }
}

fn callable_key(path: &str, executable_index: u32) -> ProjectionPackageCallableKey {
    ProjectionPackageCallableKey::new(path, "api", executable_index)
}

pub(super) fn callable_id(
    artifact: &skiff_artifact_model::PackageArtifact,
    path: &str,
) -> skiff_artifact_model::PackageCallableId {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &artifact.package_local_abi.public_symbols[path]
    else {
        panic!("{path} must be a callable");
    };
    callable_id.clone()
}

fn file_ref() -> FileIrRef {
    FileIrRef {
        file_ir_identity: "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        module_path: "api".to_string(),
        artifact_path: None,
        source_ast_hash: Some("source-hash".to_string()),
    }
}
