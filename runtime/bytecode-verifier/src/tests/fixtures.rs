use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeArtifactRef,
    BytecodeImage, BytecodePoolEntry, BytecodePools, ContractDiagnosticText,
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentRevision, FrozenConstantGraph,
    PackageArtifact, PackageArtifactRef, PackageBuildId, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceProtocolIdentity, ShapeDeclaration, TypeRefIr, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    BytecodePackageIndex, LinkedBytecodeAuthorityPins, LinkedBytecodeCandidate,
    LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts, LinkedPackageBytecodeProvenance,
    LinkedShapeEntry, LinkedTypeEntry,
};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader, HydratedDeploymentBytecode,
};

use crate::VerificationLimits;

mod local_calls;

pub(super) use local_calls::{
    loader_backed_local_call, LocalCallCandidateCorruption, TARGET_FUNCTION_INDEX,
};

#[derive(Debug)]
struct ExactResolver {
    deployment: Arc<ServiceDeployment>,
    contract: Arc<ServiceContract>,
    package: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AuthorityPinCorruption {
    NativeValueLifecycleRegistry,
    ValueLifecyclePolicy,
    HostEffectRegistry,
    IntrinsicRegistry,
}

impl DeploymentBytecodeContentResolver for ExactResolver {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        Ok(Arc::clone(&self.package))
    }

    fn resolve_package_bytecode(
        &self,
        _package: &PackageArtifactRef,
        _reference: &BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        Ok(Arc::clone(&self.bytecode))
    }
}

pub(super) fn exact_hydration() -> HydratedDeploymentBytecode {
    exact_hydration_with_types(Vec::new())
}

pub(super) fn exact_hydration_with_types(types: Vec<TypeRefIr>) -> HydratedDeploymentBytecode {
    exact_hydration_with_types_and_shapes(types, Vec::new())
}

pub(super) fn exact_hydration_with_types_and_shapes(
    types: Vec<TypeRefIr>,
    shapes: Vec<ShapeDeclaration>,
) -> HydratedDeploymentBytecode {
    let bytecode = bytecode(types, shapes);
    let package = package(bytecode.reference().clone());
    let contract = contract();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(&contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:verifier-test"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(&package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "verifier test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(contract),
        package: Arc::new(package),
        bytecode,
    };
    DeploymentBytecodeLoader::new(&resolver)
        .load(&reference)
        .unwrap()
}

fn bytecode(
    types: Vec<TypeRefIr>,
    shapes: Vec<ShapeDeclaration>,
) -> Arc<ValidatedBytecodeArtifact> {
    let pools = BytecodePools {
        types: types
            .into_iter()
            .map(|ty| BytecodePoolEntry::TypeRef { ty })
            .collect(),
        shapes: shapes
            .into_iter()
            .map(|shape| BytecodePoolEntry::ShapeRef { shape })
            .collect(),
        ..BytecodePools::default()
    };
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
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::new(),
            pools,
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn package(bytecode: BytecodeArtifactRef) -> PackageArtifact {
    let package_id = "example.verifier";
    let schema_identity =
        skiff_artifact_identity::package_schema_index_identity(package_id, &BTreeMap::new())
            .unwrap();
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode: Some(bytecode),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: schema_identity,
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
    artifact
}

fn contract() -> ServiceContract {
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: "example.verifier".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::new(),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "example.verifier".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    contract
}

pub(super) fn candidate_for(
    hydrated: &HydratedDeploymentBytecode,
    schema_override: Option<&str>,
) -> LinkedBytecodeCandidate {
    candidate_for_with_authority_corruption(hydrated, schema_override, None)
}

pub(super) fn candidate_for_with_authority_corruption(
    hydrated: &HydratedDeploymentBytecode,
    schema_override: Option<&str>,
    authority_corruption: Option<AuthorityPinCorruption>,
) -> LinkedBytecodeCandidate {
    LinkedBytecodeCandidate::try_from_parts(candidate_parts(
        hydrated,
        schema_override,
        authority_corruption,
    ))
    .unwrap()
}

pub(super) fn candidate_for_concrete_types(
    hydrated: &HydratedDeploymentBytecode,
    types: Vec<LinkedTypeEntry>,
    shapes: Vec<LinkedShapeEntry>,
) -> Result<LinkedBytecodeCandidate, LinkedBytecodeCandidateError> {
    let mut parts = candidate_parts(hydrated, None, None);
    parts.types = types;
    parts.shapes = shapes;
    LinkedBytecodeCandidate::try_from_parts(parts)
}

fn candidate_parts(
    hydrated: &HydratedDeploymentBytecode,
    schema_override: Option<&str>,
    authority_corruption: Option<AuthorityPinCorruption>,
) -> LinkedBytecodeCandidateParts {
    let packages = hydrated
        .packages()
        .values()
        .enumerate()
        .map(|(index, package)| {
            let index = u32::try_from(index).unwrap();
            let artifact = package.bytecode().artifact();
            let view = package.bytecode().view();
            let mut native_registry = view.native_value_lifecycle_registry().clone();
            let mut policy = view.value_lifecycle_policy().clone();
            let mut host_registry = view.host_effect_registry().clone();
            let mut intrinsic_registry = view.intrinsic_registry().clone();
            match authority_corruption {
                Some(AuthorityPinCorruption::NativeValueLifecycleRegistry) => {
                    native_registry.fingerprint.push_str(":corrupt");
                }
                Some(AuthorityPinCorruption::ValueLifecyclePolicy) => {
                    policy.fingerprint.push_str(":corrupt");
                }
                Some(AuthorityPinCorruption::HostEffectRegistry) => {
                    host_registry.fingerprint.push_str(":corrupt");
                }
                Some(AuthorityPinCorruption::IntrinsicRegistry) => {
                    intrinsic_registry.fingerprint.push_str(":corrupt");
                }
                None => {}
            }
            LinkedPackageBytecodeProvenance::new(
                BytecodePackageIndex::new(index),
                package.reference().package_build_id.clone(),
                package.bytecode().reference().clone(),
                view.bytecode_identity(),
                artifact.magic.clone(),
                schema_override.unwrap_or_else(|| view.schema_version()),
                view.isa_version(),
                view.opcode_table_fingerprint(),
                LinkedBytecodeAuthorityPins::new(
                    native_registry,
                    policy,
                    host_registry,
                    intrinsic_registry,
                )
                .unwrap(),
            )
            .unwrap()
        })
        .collect();
    LinkedBytecodeCandidateParts {
        packages,
        functions: Vec::new(),
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: Vec::new(),
        service_operations: Vec::new(),
        actor_creates: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        callback_capture_layouts: Vec::new(),
        host_effect_adapters: Vec::new(),
        intrinsics: Vec::new(),
        types: Vec::new(),
        shapes: Vec::new(),
        constants: Vec::new(),
        constant_roots: Vec::new(),
        frozen_constant_nodes: Vec::new(),
        resume_sites: Vec::new(),
        writable_paths: Vec::new(),
    }
}

pub(super) fn generous_limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: u64::MAX,
        max_total_instructions: u64::MAX,
        max_instructions_per_function: u64::MAX,
        max_frame_slots_per_function: u64::MAX,
        max_operand_depth: u64::MAX,
        max_control_flow_edges_per_function: u64::MAX,
        max_exception_regions_per_function: u64::MAX,
        max_switch_targets_per_function: u64::MAX,
        max_debug_entries_per_function: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_arity: u64::MAX,
        max_callback_captures_per_callback: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_value_lifecycle_nodes: u64::MAX,
        max_value_lifecycle_canonical_bytes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}
