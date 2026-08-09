use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorAbiInput, ActorCreateImplementationIr, ActorCreateSignatureIr,
    ActorDeclarationIr, ActorFieldIr, ActorImplementationIdentity, ActorMethodIdentity,
    ActorPublicMethodIr, CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
    CallableSemanticFacts, ConstDeclarationIr, ConstExport, ConstIr, ContractOperationId,
    ContractRequirement, ExecutableBody, ExecutableDeclarationIr, ExecutableExport, ExecutableIr,
    ExecutableKind, ExecutableSignatureIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
    InterfaceDeclIr, InterfaceOperationIr, PackageCallableParameter, PackageCallableSignature,
    PackageExportIndex, PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRefIr,
    PackageRequirement, PackageRuntimeRequirements, PackageSymbolRef, PackageTypeRef, ParamIr,
    ParamModeIr, ServiceCallRef, ServiceProtocolIdentity, ServiceRequirement, ServiceSymbolRef,
    SlotLayout, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr, TypeExport, TypeRefIr,
    ValueProvenance, ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionLocalInterfaceConformance,
    ProjectionLocalInterfaceConformanceFacts, ProjectionPackageCallableKey,
    ProjectionPackageCallableSignatureFacts, ProjectionSourceSymbolKey,
};

use crate::package_artifact::{
    api_exports::{PackageExportPublicInstance, PackageExportSymbol, PackageExports},
    export_links::{
        project_package_export_links, PackagePublicInstanceExecutionLink,
        PackagePublicInstanceMethodExecutionLink, ProjectedPackageExportLinks,
    },
    model::PackageExportLinkProjectionInput,
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
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    project_fixture_with_runtime_requirements(signature_set, runtime_requirements())
}

pub(super) fn project_fixture_with_runtime_requirements(
    signature_set: SignatureSet,
    runtime_requirements: PackageRuntimeRequirements,
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    project_fixture_with_conformance_facts(signature_set, runtime_requirements, true)
}

pub(super) fn project_fixture_without_local_conformance_facts(
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    project_fixture_with_conformance_facts(SignatureSet::Complete, runtime_requirements(), false)
}

fn project_fixture_with_conformance_facts(
    signature_set: SignatureSet,
    runtime_requirements: PackageRuntimeRequirements,
    include_local_conformance_facts: bool,
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    let file_ref = file_ref();
    let export_index = PackageExportIndex {
        types: BTreeMap::from([(
            "Worker".to_string(),
            TypeExport {
                file: file_ref.clone(),
                type_index: 0,
                symbol: "Worker".to_string(),
                is_interface: false,
                descriptor: Some(TypeDescriptorIr::Record {
                    fields: BTreeMap::new(),
                }),
                type_params: Vec::new(),
                interface_methods: Vec::new(),
                actor: None,
            },
        )]),
        constants: BTreeMap::from([(
            "VERSION".to_string(),
            ConstExport {
                file: file_ref.clone(),
                const_index: 1,
                symbol: "VERSION".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
        )]),
        functions: BTreeMap::from([
            (
                "mutate".to_string(),
                executable_export(&file_ref, 1, "mutate"),
            ),
            ("run".to_string(), executable_export(&file_ref, 0, "run")),
            (
                "runAlias".to_string(),
                executable_export(&file_ref, 0, "run"),
            ),
        ]),
        impl_methods: BTreeMap::from([("Worker.handle".to_string(), receiver_export(&file_ref))]),
        ..PackageExportIndex::default()
    };
    let api_exports = PackageExports {
        entries: Vec::new(),
        symbols: BTreeMap::from([
            (
                "Worker".to_string(),
                PackageExportSymbol {
                    module: "api".to_string(),
                    symbol: "Worker".to_string(),
                },
            ),
            (
                "VERSION".to_string(),
                PackageExportSymbol {
                    module: "api".to_string(),
                    symbol: "VERSION".to_string(),
                },
            ),
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
            (
                "runAlias".to_string(),
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
            signature(TypeRefIr::builtin("string")),
        ),
        (
            callable_key("mutate", 1),
            signature(TypeRefIr::builtin("string")),
        ),
        (
            callable_key("worker.handle", 2),
            signature(TypeRefIr::builtin("string")),
        ),
        (
            callable_key("runAlias", 0),
            signature(TypeRefIr::builtin("string")),
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
            signature(TypeRefIr::builtin("string")),
        )),
        SignatureSet::TargetMismatch => {
            signature_entries[0].0 = callable_key("run", 9);
        }
    }
    let signatures =
        ProjectionPackageCallableSignatureFacts::try_from_entries(signature_entries).unwrap();
    let mut mutate = safe_facts();
    mutate.effects = CallableEffectSummary::Analyzed {
        effects: CallableMayEffects { ..no_effects() },
    };
    let semantic_facts = BTreeMap::from([
        (ProjectionExecutableKey::new("api", 0), safe_facts()),
        (ProjectionExecutableKey::new("api", 1), mutate),
        (ProjectionExecutableKey::new("api", 2), safe_facts()),
    ]);
    let mut file = skiff_artifact_model::FileIrUnit::empty("api", "source-hash");
    file.file_ir_identity = file_ref.file_ir_identity.clone();
    file.type_table.extend([
        TypeDeclIr {
            name: "Worker".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: vec![TypeRefIr::AnyInterface {
                interface: skiff_artifact_identity::interface_instantiation_ref(
                    TypeRefIr::LocalType { type_index: 1 },
                    Vec::new(),
                ),
            }],
            source_span: None,
        },
        TypeDeclIr {
            name: "WorkerInterface".to_string(),
            descriptor: TypeDescriptorIr::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ]);
    file.declarations.types.extend([
        (
            "Worker".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "api.Worker".to_string(),
                source_span: None,
            },
        ),
        (
            "WorkerInterface".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "api.WorkerInterface".to_string(),
                source_span: None,
            },
        ),
    ]);
    file.declarations.interfaces.insert(
        "WorkerInterface".to_string(),
        InterfaceDeclIr {
            name: "WorkerInterface".to_string(),
            type_params: Vec::new(),
            operations: vec![InterfaceOperationIr {
                name: "handle".to_string(),
                type_params: Vec::new(),
                params: vec![FunctionTypeParamIr {
                    name: "value".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
                return_type: TypeRefIr::builtin("string"),
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: Some(TypeRefIr::builtin("Self")),
            }],
            source_span: None,
        },
    );
    file.executables.extend([
        fixture_executable(ExecutableKind::Function, "api.run", None),
        fixture_executable(ExecutableKind::Function, "api.mutate", None),
        fixture_executable(
            ExecutableKind::ImplMethod,
            "api.Worker.handle",
            Some(TypeRefIr::LocalType { type_index: 0 }),
        ),
    ]);
    file.declarations.executables.insert(
        "Worker.handle".to_string(),
        ExecutableDeclarationIr {
            executable_index: 2,
            symbol: "api.Worker.handle".to_string(),
            source_span: None,
        },
    );
    file.constants.push(ConstIr {
        name: "worker".to_string(),
        ty: TypeRefIr::LocalType { type_index: 0 },
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.declarations.constants.insert(
        "worker".to_string(),
        ConstDeclarationIr {
            const_index: 0,
            symbol: "api.worker".to_string(),
            ty: TypeRefIr::LocalType { type_index: 0 },
            source_span: None,
        },
    );
    file.constants.push(ConstIr {
        name: "VERSION".to_string(),
        ty: TypeRefIr::builtin("string"),
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.declarations.constants.insert(
        "VERSION".to_string(),
        ConstDeclarationIr {
            const_index: 1,
            symbol: "api.VERSION".to_string(),
            ty: TypeRefIr::builtin("string"),
            source_span: None,
        },
    );
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
            alias_types: BTreeSet::new(),
        },
        file_ir_units: vec![file],
        resources: Vec::new(),
        package_requirements: vec![PackageRequirement {
            alias: "dependency".to_string(),
            package_id: "example.dependency".to_string(),
            exact_version: "2.0.0".to_string(),
            expected_local_abi: PackageLocalAbiIdentity::new("local-abi:dependency:v2"),
            expected_package_build: None,
        }],
        contract_requirements: vec![contract_requirement.clone()],
        service_requirements: vec![ServiceRequirement {
            contract_requirement,
            service_binding_slot: 3,
            used_operations: BTreeSet::from([operation_id.clone()]),
        }],
        runtime_requirements,
        callable_semantic_facts: semantic_facts,
        local_interface_conformances: if include_local_conformance_facts {
            ProjectionLocalInterfaceConformanceFacts::try_from_entries([
                ProjectionLocalInterfaceConformance::try_new(
                    Vec::new(),
                    ProjectionSourceSymbolKey::new("api", "Worker"),
                    skiff_artifact_identity::interface_instantiation_ref(
                        TypeRefIr::ServiceSymbol {
                            symbol: ServiceSymbolRef {
                                module_path: "api".to_string(),
                                symbol: "WorkerInterface".to_string(),
                            },
                        },
                        Vec::new(),
                    ),
                    vec![ProjectionExecutableKey::new("api", 2)],
                )
                .unwrap(),
            ])
            .unwrap()
        } else {
            ProjectionLocalInterfaceConformanceFacts::default()
        },
        callable_signatures: signatures,
        package_schema_index: schema_index,
        package_schema_type_records: BTreeMap::new(),
        resolved_package_schema_type_records: BTreeMap::new(),
        package_schema_refs_by_source: BTreeMap::new(),
        resolved_package_schemas: &[],
        service_call_refs: vec![ServiceCallRef {
            service_requirement_slot: 3,
            contract_operation_id: operation_id,
            expected_protocol_identity: protocol_identity,
        }],
    })?
    .artifact)
}

fn public_instance(file: &FileIrRef) -> PackagePublicInstanceExecutionLink {
    let receiver_type = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "api".to_string(),
            symbol: "Worker".to_string(),
        },
    };
    let interface_type = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: "example.pkg".to_string(),
            },
            symbol_path: "api.WorkerInterface".to_string(),
            abi_expectation: None,
        },
    };
    PackagePublicInstanceExecutionLink {
        public_path: "worker".to_string(),
        declared_receiver_type: receiver_type.clone(),
        interfaces: vec![interface_type],
        receiver: ConstExport {
            file: file.clone(),
            const_index: 0,
            symbol: "worker".to_string(),
            ty: receiver_type,
        },
        methods: vec![PackagePublicInstanceMethodExecutionLink {
            name: "handle".to_string(),
            public_path: "worker.handle".to_string(),
            executable: receiver_export(file),
        }],
    }
}

fn receiver_export(file: &FileIrRef) -> ExecutableExport {
    let self_ty = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "api".to_string(),
            symbol: "Worker".to_string(),
        },
    };
    ExecutableExport {
        file: file.clone(),
        executable_index: 2,
        symbol: "Worker.handle".to_string(),
        signature: ExecutableSignatureIr {
            params: vec![skiff_artifact_model::ParamIr {
                name: "value".to_string(),
                slot: 1,
                ty: TypeRefIr::builtin("string"),
                mode: skiff_artifact_model::ParamModeIr::Value,
            }],
            return_type: TypeRefIr::builtin("string"),
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
                ty: TypeRefIr::builtin("string"),
                mode: skiff_artifact_model::ParamModeIr::Value,
            }],
            return_type: TypeRefIr::builtin("string"),
            self_type: None,
            may_suspend: false,
        },
    }
}

fn fixture_executable(
    kind: ExecutableKind,
    symbol: &str,
    self_type: Option<TypeRefIr>,
) -> ExecutableIr {
    ExecutableIr {
        kind,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: u32::from(self_type.is_some()),
            ty: TypeRefIr::builtin("string"),
            mode: ParamModeIr::Value,
        }],
        return_type: TypeRefIr::builtin("string"),
        self_type,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    }
}

pub(super) fn signature(ty: TypeRefIr) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "value".to_string(),
            ty: PackageTypeRef::Local { local_type: ty },
            mode: ParamModeIr::Value,
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: false,
    }
}

pub(super) fn exact_typed_signature() -> PackageCallableSignature {
    let contract = PackageTypeRef::PackageSchema {
        package_id: "example.dependency".to_string(),
        stable_schema_key: "User".to_string(),
        package_schema_type_id: "package-type:user".into(),
    };
    PackageCallableSignature {
        type_params: Vec::new(),
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
            mode: ParamModeIr::Value,
        }],
        return_type: contract,
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
            direct_return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

pub(super) fn runtime_requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        config: vec![skiff_artifact_model::PackageConfigRequirement {
            path: "app.token".to_string(),
            access: skiff_artifact_model::PackageConfigAccess::Required {
                value_type: "string".to_string(),
            },
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

pub(super) fn project_actor_fixture(
) -> Result<skiff_artifact_model::PackageArtifact, crate::error::ProjectionError> {
    let package_id = "example.actor.pkg";
    let read_identity = ActorMethodIdentity::new("skiff-actor-method-v1:sha256:read");
    let create_identity = ActorMethodIdentity::new("skiff-actor-method-v1:sha256:create");
    let mut unit = FileIrUnit::empty("thread_actor", "source-hash");
    unit.file_ir_identity = "file-ir:thread_actor".to_string();
    unit.type_table.push(TypeDeclIr {
        name: "ThreadActor".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("u64")),
                ("label".to_string(), TypeRefIr::builtin("string")),
            ]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    unit.declarations.types.insert(
        "ThreadActor".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "thread_actor.ThreadActor".to_string(),
            source_span: None,
        },
    );
    unit.link_targets.types.insert(
        "ThreadActor".to_string(),
        skiff_artifact_model::TypeLinkTargetIr { type_index: 0 },
    );
    unit.actor_declarations.push(ActorDeclarationIr {
        actor_abi_identity: ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:thread-actor"),
        actor_implementation_identity: ActorImplementationIdentity::new(
            "skiff-actor-implementation-v1:sha256:thread-actor",
        ),
        abi: ActorAbiInput {
            actor_name: "ThreadActor".to_string(),
            actor_id_type: TypeRefIr::builtin("u64"),
            key_field: "id".to_string(),
            fields: vec![
                ActorFieldIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::builtin("u64"),
                    encoding: skiff_artifact_model::ActorFieldEncodingIr::CanonicalValueV1,
                },
                ActorFieldIr {
                    name: "label".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    encoding: skiff_artifact_model::ActorFieldEncodingIr::CanonicalValueV1,
                },
            ],
            create: Some(ActorCreateSignatureIr {
                parameters: vec![FunctionTypeParamIr {
                    name: "label".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
            }),
            public_methods: vec![ActorPublicMethodIr {
                method_identity: read_identity.clone(),
                name: "read".to_string(),
                parameters: Vec::new(),
                return_type: TypeRefIr::builtin("string"),
                may_suspend: false,
            }],
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        },
        method_implementations: BTreeMap::from([(read_identity, 0)]),
        create_implementation: Some(ActorCreateImplementationIr {
            identity: create_identity,
            executable_index: 1,
        }),
    });
    let mut read = fixture_executable(
        ExecutableKind::ImplMethod,
        "thread_actor.ThreadActor.read",
        Some(TypeRefIr::LocalType { type_index: 0 }),
    );
    read.params.clear();
    unit.executables.extend([
        read,
        fixture_executable(
            ExecutableKind::ImplMethod,
            "thread_actor.ThreadActor.create",
            Some(TypeRefIr::LocalType { type_index: 0 }),
        ),
    ]);
    unit.declarations.executables.extend([
        (
            "ThreadActor.read".to_string(),
            ExecutableDeclarationIr {
                executable_index: 0,
                symbol: "thread_actor.ThreadActor.read".to_string(),
                source_span: None,
            },
        ),
        (
            "ThreadActor.create".to_string(),
            ExecutableDeclarationIr {
                executable_index: 1,
                symbol: "thread_actor.ThreadActor.create".to_string(),
                source_span: None,
            },
        ),
    ]);
    let api_exports = PackageExports {
        entries: Vec::new(),
        symbols: BTreeMap::from([(
            "ThreadActor".to_string(),
            PackageExportSymbol {
                module: "thread_actor".to_string(),
                symbol: "ThreadActor".to_string(),
            },
        )]),
        public_instances: Vec::new(),
    };
    let file_ir_units = vec![unit];
    let export_links = project_package_export_links(
        &PackageExportLinkProjectionInput {
            package_id,
            exports: &api_exports,
            file_ir_units: &file_ir_units,
        },
        &[],
    )?;
    let schema_types = BTreeMap::new();
    let schema_index = skiff_artifact_model::PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            package_id,
            &schema_types,
        )
        .unwrap(),
        types: schema_types,
    };
    let signatures = ProjectionPackageCallableSignatureFacts::try_from_entries(Vec::new()).unwrap();
    Ok(project_package_artifact_facts(ProjectedPackageFacts {
        package_id,
        package_version: "1.0.0",
        api_exports: &api_exports,
        export_links,
        file_ir_units,
        resources: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([
            (
                ProjectionExecutableKey::new("thread_actor", 0),
                safe_facts(),
            ),
            (
                ProjectionExecutableKey::new("thread_actor", 1),
                safe_facts(),
            ),
        ]),
        local_interface_conformances: ProjectionLocalInterfaceConformanceFacts::default(),
        callable_signatures: signatures,
        package_schema_index: schema_index,
        package_schema_type_records: BTreeMap::new(),
        resolved_package_schema_type_records: BTreeMap::new(),
        package_schema_refs_by_source: BTreeMap::new(),
        resolved_package_schemas: &[],
        service_call_refs: Vec::new(),
    })?
    .artifact)
}

fn file_ref() -> FileIrRef {
    FileIrRef {
        file_ir_identity: "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        module_path: "api".to_string(),
        artifact_path: None,
        source_ast_hash: Some("source-hash".to_string()),
    }
}
