use std::collections::BTreeMap;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    FileIrRef, OperationCallableKind, OperationTargetRef, PackageArtifact, PackageBuildId,
    PackageCallableId, PackageCallableLinkFact, PackageCallableSignature,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::{analyzed_facts, RootProgram, HELPER_CALLABLE, ROOT_CALLABLE};

pub(super) fn package(
    bytecode: &ValidatedBytecodeArtifact,
    program: RootProgram,
    entry_alias: Option<&PackageCallableId>,
) -> PackageArtifact {
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
    let implementation_symbols = BTreeMap::from([
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
    let mut callable_semantic_facts = BTreeMap::from([
        (root_callable, analyzed_facts()),
        (helper_callable, analyzed_facts()),
    ]);
    if let Some(alias) = entry_alias {
        callable_links.insert(
            alias.clone(),
            callable_link(alias.clone(), 0, OperationCallableKind::PublicFunction),
        );
        public_symbols.insert(
            "fixture.public_root".to_string(),
            callable_symbol(alias.clone(), root_signature),
        );
        callable_semantic_facts.insert(alias.clone(), analyzed_facts());
    }

    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.bytecode-link".to_string(),
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
        implementation_links: PackageImplementationLinks::default(),
        callable_links,
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts,
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
