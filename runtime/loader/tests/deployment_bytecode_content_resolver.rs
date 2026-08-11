use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeArtifactRef,
    BytecodeConstantRef, BytecodeImage, BytecodePoolEntry, BytecodePools, ContractDiagnosticText,
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentRevision, FrozenConstantGraph,
    FrozenConstantNode, LiteralIr, PackageArtifact, PackageArtifactRef, PackageBuildId,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexRef, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity, TypeRefIr, ValueDropPlan,
    ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_loader::{
    load_deployment_bytecode_from_store, DeploymentBytecodeHydrationError,
    DeploymentBytecodeReference, FilesystemDeploymentBytecodeContentResolver,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-loader-deployment-bytecode-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bytecode(seed: &str) -> BytecodeArtifact {
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::new(),
            pools: BytecodePools {
                constants: vec![BytecodePoolEntry::ConstantRef {
                    reference: BytecodeConstantRef::LocalNode { node_index: 0 },
                    type_ref: 0,
                    plan: ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::Trivial,
                    },
                }],
                types: vec![BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                }],
                shapes: Vec::new(),
                effects: Vec::new(),
                resume: Vec::new(),
                callback_capture: Vec::new(),
                writable_paths: Vec::new(),
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph {
                nodes: vec![FrozenConstantNode::Literal {
                    literal: LiteralIr::String {
                        value: seed.to_string(),
                    },
                }],
            },
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    artifact
}

fn package(
    package_id: &str,
    bytecode: &BytecodeArtifact,
) -> (PackageArtifact, PackageArtifactRef, BytecodeArtifactRef) {
    let bytecode_ref = BytecodeArtifactRef::new(bytecode.bytecode_identity.clone());
    let schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(package_id, &BTreeMap::new())
            .unwrap();
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode: Some(bytecode_ref.clone()),
        bytecode_statement_manifest_identity:
            skiff_artifact_model::derive_bytecode_statement_manifest_identity(package_id, &[])
                .unwrap(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: schema_index_identity,
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
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
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    let reference = skiff_artifact_identity::package_artifact_ref(&artifact).unwrap();
    (artifact, reference, bytecode_ref)
}

fn contract(service_id: &str) -> (ServiceContract, ServiceContractRef) {
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::new(),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    let reference = skiff_artifact_identity::service_contract_ref(&contract).unwrap();
    (contract, reference)
}

fn deployment(
    contract: ServiceContractRef,
    implementation: PackageArtifactRef,
) -> (ServiceDeployment, ServiceDeploymentRef) {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation,
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "filesystem resolver".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (deployment, reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_resolver_hydrates_exact_deployment_from_canonical_store() {
        let temp = TestRoot::new();
        let store = CanonicalArtifactStore::create(temp.path()).unwrap();
        let canonical_root = store.root().to_path_buf();

        let bytecode = bytecode("filesystem-resolver");
        let (package, package_ref, bytecode_ref) = package("example.filesystem", &bytecode);
        let (contract, contract_ref) = contract("example.filesystem");
        let (deployment, deployment_ref) = deployment(contract_ref, package_ref.clone());
        let own_contract = deployment.contract.clone();

        store.write_service_contract(&contract).unwrap();
        store
            .write_package_bytecode(&package_ref, &bytecode)
            .unwrap();
        store.write_package_artifact(&package).unwrap();
        store.write_service_deployment(&deployment).unwrap();

        let hydrated = load_deployment_bytecode_from_store(store, &deployment_ref).unwrap();
        assert_eq!(hydrated.reference(), &deployment_ref);
        assert_eq!(hydrated.deployment().implementation, package_ref);
        assert!(hydrated.contract_store().contains_key(&own_contract));
        let hydrated_package = hydrated
            .packages()
            .get(&package_ref.package_build_id)
            .expect("implementation package must hydrate");
        assert_eq!(
            hydrated_package.artifact().bytecode.as_ref(),
            Some(&bytecode_ref)
        );
        assert_eq!(hydrated_package.bytecode().reference(), &bytecode_ref);
        assert_eq!(
            hydrated_package.platform_error_projection_registry(),
            skiff_artifact_model::current_platform_error_projection_registry_ref()
        );
        assert_eq!(
            hydrated.platform_error_projection_registry(),
            hydrated_package.platform_error_projection_registry()
        );

        let resolver = FilesystemDeploymentBytecodeContentResolver::open(&canonical_root).unwrap();
        assert_eq!(resolver.store().root(), canonical_root.as_path());
        let hydrated = resolver.load_deployment_bytecode(&deployment_ref).unwrap();
        assert_eq!(hydrated.reference(), &deployment_ref);
    }

    #[test]
    fn missing_deployment_reference_fails_closed_through_filesystem_resolver() {
        let temp = TestRoot::new();
        let store = CanonicalArtifactStore::create(temp.path()).unwrap();
        let resolver = FilesystemDeploymentBytecodeContentResolver::from_store(store);
        let reference = ServiceDeploymentRef {
            service_id: "example.missing".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("revision:missing"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("missing"),
        };

        let error = resolver.load_deployment_bytecode(&reference).unwrap_err();
        assert!(matches!(
            error,
            DeploymentBytecodeHydrationError::ContentResolution {
                reference,
                ..
            } if matches!(
                reference.as_ref(),
                DeploymentBytecodeReference::ServiceDeployment(_)
            )
        ));
    }
}
