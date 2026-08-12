use std::collections::BTreeMap;

use crate::{
    derive_bytecode_statement_manifest_identity, CallableEffectSummary, CallableProvenanceSummary,
    CallableProvenanceUnknownReason, CallableSemanticFacts, FileIrRef, OperationCallableKind,
    OperationTargetRef, PackageArtifact, PackageBuildId, PackageCallableId,
    PackageCallableLinkFact, PackageCallableSignature, PackageExecutableCoordinate,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef, PackageTypeRef,
    TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

mod schema_records;
mod synthetic;

const PACKAGE_ID: &str = "example.pkg";
const IMPLEMENTATION_CALLABLE_ID: &str = "impl:canonical";

fn facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::analysis_pending(),
        provenance: CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::AnalysisPending,
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn signature() -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: Vec::new(),
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    }
}

fn owner_coordinate() -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: "file-ir:main".to_string(),
        module_path: "module".to_string(),
        executable_index: 7,
    }
}

fn authority_artifact() -> PackageArtifact {
    let callable_id = PackageCallableId::new(IMPLEMENTATION_CALLABLE_ID);
    let coordinate = owner_coordinate();
    let file = FileIrRef::new(
        coordinate.file_ir_identity.clone(),
        coordinate.module_path.clone(),
    );
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: PACKAGE_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        platform_error_projection_registry: crate::current_platform_error_projection_registry_ref()
            .clone(),
        files: vec![file.clone()],
        static_resources: Vec::new(),
        bytecode: None,
        bytecode_statement_manifest_identity: derive_bytecode_statement_manifest_identity(
            PACKAGE_ID,
            &[],
        )
        .unwrap(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unchanged"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::from([(
                "module.main".to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: signature(),
                },
            )]),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: PACKAGE_ID.to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("index"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::from([(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target: OperationTargetRef {
                    file_ref: file,
                    executable_index: coordinate.executable_index,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::InternalFunction,
                },
            },
        )]),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([(callable_id, facts())]),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

#[test]
fn package_without_bytecode_requires_its_own_canonical_empty_statement_manifest() {
    let mut artifact = authority_artifact();
    artifact.bytecode_statement_manifest_identity =
        derive_bytecode_statement_manifest_identity("other.pkg", &[]).unwrap();
    let error = crate::validate_package_build_authority(&artifact).unwrap_err();
    assert!(error
        .message()
        .contains("canonical empty statement manifest"));
}
