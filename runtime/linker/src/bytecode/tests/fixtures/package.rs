use std::collections::BTreeMap;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, BoundaryCallableProjection,
    BoundaryImplementationRequirements, BytecodeFunctionStatementManifest, ExecutableExport,
    ExecutableSignatureIr, FileIrRef, InterfaceMethodSignature, OperationCallableKind,
    OperationTargetRef, PackageArtifact, PackageBuildId, PackageCallableId,
    PackageCallableLinkFact, PackageCallableSignature, PackageExecutableCoordinate,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    PackageSyntheticCallbackOwner, PackageTypeRef, TypeDescriptorIr, TypeExport, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::{
    analyzed_facts, constants, no_effects, schema_record, synthetic_callback_callable_for,
    DependencyTypeSurfaceConflict, RootProgram, DEPENDENCY_PACKAGE_ID, HELPER_CALLABLE,
    OWNER_IMPLEMENTATION_PATH, OWNER_PUBLIC_PATH, PRIVATE_IMPLEMENTATION_PATH, ROOT_CALLABLE,
};

pub(super) fn package(
    bytecode: &ValidatedBytecodeArtifact,
    program: RootProgram,
    entry_alias: Option<&PackageCallableId>,
    include_normalization_surface: bool,
    conflicting_type_surfaces: bool,
) -> PackageArtifact {
    let package_id = "example.bytecode-link";
    let file = file_ref();
    let root_callable = PackageCallableId::new(ROOT_CALLABLE);
    let helper_callable = PackageCallableId::new(HELPER_CALLABLE);
    let root_signature = callable_signature(program.root_has_parameter());
    let mut callable_links = BTreeMap::from([
        (
            root_callable.clone(),
            callable_link(
                root_callable.clone(),
                0,
                OperationCallableKind::InternalFunction,
            ),
        ),
        (
            helper_callable.clone(),
            callable_link(
                helper_callable.clone(),
                1,
                OperationCallableKind::InternalFunction,
            ),
        ),
    ]);
    let descriptor = empty_record_descriptor();
    let mut implementation_symbols = BTreeMap::from([
        (
            "fixture.root".to_string(),
            callable_symbol(root_callable.clone(), root_signature.clone()),
        ),
        (
            "fixture.helper".to_string(),
            callable_symbol(
                helper_callable.clone(),
                callable_signature(program == RootProgram::UnreachableInterface),
            ),
        ),
    ]);
    implementation_symbols.extend(constants::implementation_symbols(bytecode, package_id));
    let mut public_symbols = BTreeMap::new();
    if let Some((symbol_path, symbol)) = constants::representation_type_symbol(program) {
        public_symbols.insert(symbol_path, symbol);
    }
    if matches!(
        program,
        RootProgram::Interface | RootProgram::UnreachableInterface
    ) {
        implementation_symbols.insert(
            "fixture.Reader".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:example.bytecode-link:top-level:fixture.Reader".to_string(),
                descriptor: TypeDescriptorIr::Interface,
                is_alias: false,
                is_interface: true,
                type_params: Vec::new(),
                interface_methods: vec![interface_method("read")],
                actor: None,
            },
        );
        public_symbols.insert(
            "Reader".to_string(),
            PackageLocalAbiSymbol::Type {
                local_type_id: "type:Reader".to_string(),
                descriptor: TypeDescriptorIr::Interface,
                is_alias: false,
                is_interface: true,
                type_params: Vec::new(),
                interface_methods: vec![interface_method("read")],
                actor: None,
            },
        );
    }
    if include_normalization_surface {
        implementation_symbols.insert(
            OWNER_IMPLEMENTATION_PATH.to_string(),
            implementation_type_symbol(package_id, OWNER_IMPLEMENTATION_PATH, descriptor.clone()),
        );
        implementation_symbols.insert(
            PRIVATE_IMPLEMENTATION_PATH.to_string(),
            implementation_type_symbol(package_id, PRIVATE_IMPLEMENTATION_PATH, descriptor.clone()),
        );
        public_symbols.insert(
            OWNER_PUBLIC_PATH.to_string(),
            public_type_symbol(OWNER_PUBLIC_PATH, descriptor.clone()),
        );
    }
    if conflicting_type_surfaces {
        public_symbols.insert(
            OWNER_IMPLEMENTATION_PATH.to_string(),
            public_type_symbol(
                OWNER_IMPLEMENTATION_PATH,
                TypeDescriptorIr::Alias {
                    target: TypeRefIr::builtin("string"),
                },
            ),
        );
    }
    let mut callable_semantic_facts = BTreeMap::from([
        (root_callable, analyzed_facts()),
        (helper_callable, analyzed_facts()),
    ]);
    let synthetic_callback_owner = if program == RootProgram::UnreachableCallback {
        HELPER_CALLABLE
    } else {
        ROOT_CALLABLE
    };
    let synthetic_callback = matches!(
        program,
        RootProgram::SyntheticTarget | RootProgram::UnreachableCallback
    )
    .then(|| synthetic_callback_callable_for(synthetic_callback_owner));
    if let Some(callback) = &synthetic_callback {
        callable_semantic_facts.insert(callback.clone(), analyzed_facts());
    }
    let mut boundary_projections = BTreeMap::new();
    if let Some(alias) = entry_alias {
        callable_links.insert(
            alias.clone(),
            callable_link(alias.clone(), 0, OperationCallableKind::PublicFunction),
        );
        public_symbols.insert(
            "fixture.public_root".to_string(),
            callable_symbol(alias.clone(), root_signature),
        );
        let facts = analyzed_facts();
        boundary_projections.insert(
            alias.clone(),
            BoundaryCallableProjection::Available {
                operation_contract: super::records::operation_contract(
                    program.root_has_parameter(),
                ),
                implementation_requirements: BoundaryImplementationRequirements {
                    config: Vec::new(),
                    state: Vec::new(),
                    native_capabilities: Vec::new(),
                    complete_may_effects: no_effects(),
                    provenance: facts.provenance.clone(),
                },
            },
        );
        callable_semantic_facts.insert(alias.clone(), facts);
    }

    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        files: vec![file],
        static_resources: Vec::new(),
        bytecode: Some(bytecode.reference().clone()),
        bytecode_statement_manifest_identity: statement_manifest_identity(package_id, bytecode),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols,
            implementation_symbols,
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.bytecode-link".to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("unassigned"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: {
                let mut types = if include_normalization_surface {
                    type_links(descriptor)
                } else {
                    BTreeMap::new()
                };
                if matches!(
                    program,
                    RootProgram::Interface | RootProgram::UnreachableInterface
                ) {
                    types.insert(
                        "fixture.Reader".to_string(),
                        TypeExport {
                            file: file_ref(),
                            type_index: 0,
                            symbol: "Reader".to_string(),
                            is_interface: true,
                            descriptor: Some(TypeDescriptorIr::Interface),
                            type_params: Vec::new(),
                            interface_methods: vec![interface_method("read")],
                            actor: None,
                        },
                    );
                }
                types
            },
            functions: entry_alias
                .is_some()
                .then(|| {
                    (
                        "fixture.root".to_string(),
                        ExecutableExport {
                            file: file_ref(),
                            executable_index: 0,
                            symbol: "fixture.root".to_string(),
                            signature: executable_signature(program.root_has_parameter()),
                        },
                    )
                })
                .into_iter()
                .collect(),
            constants: constants::implementation_links(bytecode),
            ..PackageImplementationLinks::default()
        },
        callable_links,
        synthetic_callback_owners: synthetic_callback
            .map(|package_callable_id| PackageSyntheticCallbackOwner {
                owner: PackageExecutableCoordinate {
                    file_ir_identity: "file-ir:fixture".to_string(),
                    module_path: "fixture".to_string(),
                    executable_index: u32::from(program == RootProgram::UnreachableCallback),
                },
                site_ordinal: 0,
                package_callable_id,
            })
            .into_iter()
            .collect(),
        bytecode_schema_records: include_normalization_surface
            .then(schema_record)
            .map(|record| (record.package_schema_type_id.clone(), record))
            .into_iter()
            .collect(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts,
        boundary_projections,
        service_call_refs: Vec::new(),
    };
    artifact.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            &artifact.package_id,
            &BTreeMap::new(),
        )
        .unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

pub(super) fn dependency_type_owner_package(
    bytecode: &ValidatedBytecodeArtifact,
    conflict: Option<DependencyTypeSurfaceConflict>,
) -> PackageArtifact {
    let descriptor = empty_record_descriptor();
    let mut shared_public = public_type_symbol(OWNER_IMPLEMENTATION_PATH, descriptor.clone());
    if let PackageLocalAbiSymbol::Type {
        descriptor: public_descriptor,
        is_interface,
        type_params,
        ..
    } = &mut shared_public
    {
        match conflict {
            Some(DependencyTypeSurfaceConflict::Descriptor) => {
                *public_descriptor = TypeDescriptorIr::Alias {
                    target: TypeRefIr::builtin("string"),
                };
            }
            Some(DependencyTypeSurfaceConflict::TypeParameters) => {
                type_params.push("T".to_string());
            }
            Some(DependencyTypeSurfaceConflict::InterfaceFlag) => {
                *is_interface = true;
            }
            None => {}
        }
    }
    let mut implementation_symbols = BTreeMap::from([
        (
            OWNER_IMPLEMENTATION_PATH.to_string(),
            implementation_type_symbol(
                DEPENDENCY_PACKAGE_ID,
                OWNER_IMPLEMENTATION_PATH,
                descriptor.clone(),
            ),
        ),
        (
            PRIVATE_IMPLEMENTATION_PATH.to_string(),
            implementation_type_symbol(
                DEPENDENCY_PACKAGE_ID,
                PRIVATE_IMPLEMENTATION_PATH,
                descriptor.clone(),
            ),
        ),
    ]);
    implementation_symbols.extend(constants::implementation_symbols(
        bytecode,
        DEPENDENCY_PACKAGE_ID,
    ));
    let public_symbols = BTreeMap::from([
        (
            OWNER_PUBLIC_PATH.to_string(),
            public_type_symbol(OWNER_PUBLIC_PATH, descriptor.clone()),
        ),
        (OWNER_IMPLEMENTATION_PATH.to_string(), shared_public),
    ]);
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: DEPENDENCY_PACKAGE_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        files: vec![file_ref()],
        static_resources: Vec::new(),
        bytecode: Some(bytecode.reference().clone()),
        bytecode_statement_manifest_identity: statement_manifest_identity(
            DEPENDENCY_PACKAGE_ID,
            bytecode,
        ),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols,
            implementation_symbols,
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: DEPENDENCY_PACKAGE_ID.to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("unassigned"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: type_links(descriptor),
            constants: constants::implementation_links(bytecode),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    artifact.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            &artifact.package_id,
            &BTreeMap::new(),
        )
        .unwrap();
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn interface_method(name: &str) -> InterfaceMethodSignature {
    InterfaceMethodSignature {
        name: name.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }
}

fn statement_manifest_identity(
    package_id: &str,
    bytecode: &ValidatedBytecodeArtifact,
) -> skiff_artifact_model::BytecodeStatementManifestIdentity {
    let mut functions = bytecode
        .view()
        .functions()
        .iter()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    functions.sort_by(|left, right| left.origin.cmp(&right.origin));
    derive_bytecode_statement_manifest_identity(package_id, &functions).unwrap()
}

fn empty_record_descriptor() -> TypeDescriptorIr {
    TypeDescriptorIr::Record {
        fields: BTreeMap::new(),
    }
}

fn implementation_type_symbol(
    package_id: &str,
    source_path: &str,
    descriptor: TypeDescriptorIr,
) -> PackageLocalAbiSymbol {
    type_symbol(
        format!("type:{package_id}:top-level:{source_path}"),
        descriptor,
    )
}

fn public_type_symbol(public_path: &str, descriptor: TypeDescriptorIr) -> PackageLocalAbiSymbol {
    type_symbol(format!("type:{public_path}"), descriptor)
}

fn type_symbol(local_type_id: String, descriptor: TypeDescriptorIr) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Type {
        local_type_id,
        descriptor,
        is_alias: false,
        is_interface: false,
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: None,
    }
}

fn type_links(descriptor: TypeDescriptorIr) -> BTreeMap<String, TypeExport> {
    let owner = TypeExport {
        file: file_ref(),
        type_index: 0,
        symbol: "Owner".to_string(),
        is_interface: false,
        descriptor: Some(descriptor.clone()),
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: None,
    };
    let private = TypeExport {
        file: file_ref(),
        type_index: 1,
        symbol: PRIVATE_IMPLEMENTATION_PATH.to_string(),
        is_interface: false,
        descriptor: Some(descriptor),
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: None,
    };
    BTreeMap::from([
        (OWNER_PUBLIC_PATH.to_string(), owner.clone()),
        (OWNER_IMPLEMENTATION_PATH.to_string(), owner),
        (PRIVATE_IMPLEMENTATION_PATH.to_string(), private),
    ])
}

fn file_ref() -> FileIrRef {
    FileIrRef::new("file-ir:fixture", "fixture")
}

fn callable_link(
    callable_id: PackageCallableId,
    executable_index: u32,
    callable_kind: OperationCallableKind,
) -> PackageCallableLinkFact {
    PackageCallableLinkFact {
        callable_id: callable_id.clone(),
        target: OperationTargetRef {
            file_ref: file_ref(),
            executable_index,
            callable_abi_id: callable_id.as_str().to_string(),
            callable_kind,
        },
    }
}

fn callable_symbol(
    callable_id: PackageCallableId,
    signature: PackageCallableSignature,
) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    }
}

fn callable_signature(has_parameter: bool) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: has_parameter
            .then(|| skiff_artifact_model::PackageCallableParameter {
                name: "carrier".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
                mode: skiff_artifact_model::ParamModeIr::Value,
            })
            .into_iter()
            .collect(),
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    }
}

fn executable_signature(has_parameter: bool) -> ExecutableSignatureIr {
    ExecutableSignatureIr {
        params: has_parameter
            .then(|| skiff_artifact_model::ParamIr {
                name: "carrier".to_string(),
                slot: 0,
                ty: TypeRefIr::builtin("string"),
                mode: skiff_artifact_model::ParamModeIr::Value,
            })
            .into_iter()
            .collect(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        may_suspend: false,
    }
}
