use std::collections::BTreeMap;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryImplementationRequirements, ExecutableExport,
    ExecutableSignatureIr, FileIrRef, OperationCallableKind, OperationTargetRef, PackageArtifact,
    PackageBuildId, PackageCallableId, PackageCallableLinkFact, PackageCallableSignature,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    TypeDescriptorIr, TypeExport, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::{
    analyzed_facts, no_effects, schema_record, RootProgram, HELPER_CALLABLE,
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
            callable_symbol(helper_callable.clone(), callable_signature(false)),
        ),
    ]);
    let mut public_symbols = BTreeMap::new();
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
        files: vec![file],
        static_resources: Vec::new(),
        bytecode: Some(bytecode.reference().clone()),
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
            types: if include_normalization_surface {
                type_links(descriptor)
            } else {
                BTreeMap::new()
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
            ..PackageImplementationLinks::default()
        },
        callable_links,
        synthetic_callback_owners: Vec::new(),
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
        symbol: "Private".to_string(),
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
