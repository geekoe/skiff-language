use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, PackageArtifact, ServiceContract,
    ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, CompilerPlatformSources, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageSourceInput, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_runtime_bytecode_verifier::VerificationLimits;
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionLimits, LinkLimits,
};
use skiff_runtime_loader::{DeploymentBytecodeContentResolver, DeploymentBytecodeLoader};
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    vm_heap::{VmHeap, VmHeapError},
    vm_value::ValueSlot,
};
use skiff_runtime_vm::{
    Vm, VmBudget, VmControl, VmLimits, VmSemanticCharge,
};

fn compile_scalar_package() -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    compile_package_with_dependencies(
        "example.com/vm-scalar",
        "function helper(value: number) -> number { return value + 1 }\nfunction run(value: number) -> number { final result = helper(value)\n return result }\n",
        "skiff-vm-scalar",
        Vec::new(),
        &[],
    )
}

fn compile_package_with_dependencies(
    package_id: &str,
    text: &str,
    temp_prefix: &str,
    dependencies: Vec<skiff_compiler_input::PackageDependency>,
    dependency_packages: &[Arc<PackageArtifact>],
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let package_id = skiff_compiler_core::id::PublicationId::parse(package_id).unwrap();
    let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!(
        "{temp_prefix}-{}-{}-{}",
        std::process::id(),
        unique,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).unwrap();
    let has_dependencies = !dependencies.is_empty() || !dependency_packages.is_empty();
    let source_tree = SourceTree {
        root: temp.clone(),
        sources: vec![SourceTreeFile {
            module_path: "main".to_string(),
            file_path: PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .unwrap();
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            package_id.clone(),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            dependencies,
            ManifestProvenance {
                owner: ManifestOwner::UserOrBuiltinPackage,
                path: PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let base_input = PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        package_id.as_str(),
        true,
    );
    let owned_dependency_packages = dependency_packages
        .iter()
        .map(|package| package.as_ref().clone())
        .collect::<Vec<_>>();
    let input = if has_dependencies {
        base_input.with_canonical_dependencies(&owned_dependency_packages, &[])
    } else {
        base_input
    };
    let compiled = compile_package(input).unwrap();
    let handoff = compiled.bytecode_handoff().unwrap();
    let package_artifact = Arc::new(compiled.package().artifact.clone());
    let bytecode = Arc::new(ValidatedBytecodeArtifact::admit(handoff.artifact().clone()).unwrap());
    std::fs::remove_dir_all(temp).unwrap();
    (package_artifact, bytecode)
}

fn service_contract(
    package_id: &str,
) -> (
    Arc<ServiceContract>,
    skiff_artifact_model::ContractOperationId,
) {
    let operation_id =
        skiff_artifact_identity::contract_operation_id(package_id, "1.0.0", "run").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: "run".to_string(),
                contract: BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: BoundaryReturn {
                        ty: ContractTypeRef::builtin("number"),
                        value_plan: BoundaryValuePlan::Linkable {
                            carrier: BoundaryValueCarrier::DetachedValueGraph,
                            encoding: BoundaryValueEncoding::CanonicalValue,
                            owner: BoundaryValueOwner::Provider,
                            lifetime: BoundaryValueLifetime::Call,
                        },
                    },
                    stream: BoundaryStreamContract::Unary,
                    callbacks: BoundaryCallbackContract::None,
                    effect_guarantee: BoundaryEffectGuarantee {
                        detached_parameters: true,
                        detached_return: true,
                        detached_error: true,
                        no_caller_reachable_mutation: true,
                        no_caller_value_escape: true,
                        no_same_heap_identity: true,
                    },
                },
            },
        )]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: skiff_artifact_model::ContractDiagnosticText {
            service: package_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (Arc::new(contract), operation_id)
}

fn service_deployment(
    package: &PackageArtifact,
    contract: &ServiceContract,
    operation_id: skiff_artifact_model::ContractOperationId,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let package_ref = skiff_artifact_identity::package_artifact_ref(package).unwrap();
    let contract_ref = skiff_artifact_identity::service_contract_ref(contract).unwrap();
    let callable_id = package
        .callable_links
        .keys()
        .next()
        .expect("compiled scalar package has a callable")
        .clone();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref,
        deployment_revision: DeploymentRevision::new("revision:vm-vertical"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref,
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation_id,
            package_callable_id: callable_id,
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "vm vertical".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

struct TestResolver {
    deployment: Arc<ServiceDeployment>,
    contract: Arc<ServiceContract>,
    package: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
}

impl DeploymentBytecodeContentResolver for TestResolver {
    fn resolve_deployment(
        &self,
        reference: &skiff_artifact_model::ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        let actual = skiff_artifact_identity::service_deployment_ref(&self.deployment);
        anyhow::ensure!(&actual == reference, "deployment reference mismatch");
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &skiff_artifact_model::ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        let actual = skiff_artifact_identity::service_contract_ref(&self.contract).unwrap();
        anyhow::ensure!(&actual == reference, "contract reference mismatch");
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &skiff_artifact_model::PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        let actual = skiff_artifact_identity::package_artifact_ref(&self.package).unwrap();
        anyhow::ensure!(&actual == reference, "package reference mismatch");
        Ok(Arc::clone(&self.package))
    }

    fn resolve_package_bytecode(
        &self,
        package: &skiff_artifact_model::PackageArtifactRef,
        reference: &skiff_artifact_model::BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        let actual_package = skiff_artifact_identity::package_artifact_ref(&self.package).unwrap();
        anyhow::ensure!(&actual_package == package, "bytecode package mismatch");
        anyhow::ensure!(
            self.bytecode.reference() == reference,
            "bytecode reference mismatch"
        );
        Ok(Arc::clone(&self.bytecode))
    }
}

fn generous_link_limits() -> LinkLimits {
    LinkLimits {
        max_packages: u64::MAX,
        max_root_specializations: u64::MAX,
        max_specializations: u64::MAX,
        max_code_words_per_function: u64::MAX,
        max_total_code_words: u64::MAX,
        max_relocations_per_function: u64::MAX,
        max_total_relocations: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_total_image_table_entries: u64::MAX,
        max_total_function_table_entries: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_expanded_type_nodes: u64::MAX,
        max_expanded_type_bytes: u64::MAX,
        max_constant_graph_nodes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}

fn generous_verification_limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: u64::MAX,
        max_total_instructions: u64::MAX,
        max_instructions_per_function: u64::MAX,
        max_frame_slots_per_function: u64::MAX,
        max_operand_depth: u64::MAX,
        max_control_flow_edges_per_function: u64::MAX,
        max_exception_regions_per_function: u64::MAX,
        max_switch_targets_per_function: u64::MAX,
        max_statement_events_per_pc: u64::MAX,
        max_statement_events_per_function: u64::MAX,
        max_total_statement_events: u64::MAX,
        max_source_map_entries_per_function: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_arity: u64::MAX,
        max_callback_captures_per_callback: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_value_lifecycle_nodes: u64::MAX,
        max_value_lifecycle_canonical_bytes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}

struct TestHeap;

impl VmHeap for TestHeap {
    fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        Ok(*source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        Ok(*source)
    }

    fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }

    fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }
}

struct TestBudget {
    fuel: u64,
}

impl TestBudget {
    fn new() -> Self {
        Self { fuel: 1_000_000 }
    }
}

impl VmBudget for TestBudget {
    fn replenish_raw_fuel(
        &mut self,
        maximum: NonZeroU32,
    ) -> Result<NonZeroU32, skiff_runtime_vm::VmBudgetError> {
        if self.fuel == 0 {
            return Err(skiff_runtime_vm::VmBudgetError::InstructionLimitExceeded);
        }
        let grant = self.fuel.min(u64::from(maximum.get()));
        self.fuel -= grant;
        NonZeroU32::new(grant as u32).ok_or(skiff_runtime_vm::VmBudgetError::AccountingFailure)
    }

    fn poll_interrupt(&mut self) -> Result<(), skiff_runtime_vm::VmBudgetError> {
        Ok(())
    }

    fn charge_semantic(
        &mut self,
        _charge: VmSemanticCharge<'_>,
    ) -> Result<(), skiff_runtime_vm::VmBudgetError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm_limits() -> VmLimits {
        VmLimits::new(
            NonZeroUsize::new(128).unwrap(),
            NonZeroUsize::new(4096).unwrap(),
            NonZeroU32::new(1024).unwrap(),
            NonZeroU32::new(1024).unwrap(),
        )
    }

    fn noop_observer() -> BytecodeExecutionObserver {
        BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
            router_session_id: "vertical-test-session".to_string(),
            request_id: "vertical-test".to_string(),
        })
    }

    #[test]
    fn source_to_vm_scalar_local_call_executes_through_the_execution_entry() {
        let (package, bytecode) = compile_scalar_package();
        let (contract, operation_id) = service_contract(package.package_id.as_str());
        let (deployment, deployment_reference) =
            service_deployment(&package, &contract, operation_id);
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .unwrap();
        let image = Arc::new(
            link_deployment_execution_image(
                hydrated,
                &DeploymentExecutionLimits::new(
                    generous_link_limits(),
                    generous_verification_limits(),
                ),
            )
            .unwrap(),
        );
        let operation =
            skiff_artifact_identity::contract_operation_id("example.com/vm-scalar", "1.0.0", "run")
                .unwrap();
        let entry = image.operation_entry(&operation).unwrap();
        let mut fiber = Vm::start(
            entry,
            Box::new([ValueSlot::number(41.0)]),
            vm_limits(),
            noop_observer(),
        )
        .unwrap();
        let mut heap = TestHeap;
        let mut budget = TestBudget::new();
        loop {
            match fiber.run_segment(&mut heap, &mut budget) {
                VmControl::Continue => continue,
                VmControl::Complete(result) => {
                    let values = result.unwrap();
                    assert_eq!(values.len(), 1);
                    assert_eq!(values.values()[0].as_number(), Some(42.0));
                    break;
                }
                _ => panic!("scalar VM test produced an unsupported control handoff"),
            }
        }
    }
}
