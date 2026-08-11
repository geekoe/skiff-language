use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
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
    ContractOperationId, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, PackageArtifact, PackageCallableId,
    ServiceContract, ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, CompilerPlatformSources, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageSourceInput, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_runtime_bytecode_verifier::{verify, VerificationLimits, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{DeploymentImage, PinnedDeploymentEntry};
use skiff_runtime_linker::{link_deployment, LinkLimits};
use skiff_runtime_loader::{DeploymentBytecodeContentResolver, DeploymentBytecodeLoader};
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle},
};
use skiff_runtime_vm::{
    ChildInvocation, ChildTarget, PendingTicket, ResumeOutcome, StreamItem, Vm, VmBudget,
    VmControl, VmError, VmFiber, VmFiberState, VmLimits, VmOwnedValues, VmSemanticCharge,
};

fn compile_scalar_package() -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    compile_package_with_dependencies(
        "example.com/vm-scalar",
        "function helper(value: number) -> number { return value + 1 }\nfunction run(value: number) -> number { return helper(value) }\n",
        "skiff-vm-scalar",
        Vec::new(),
        &[],
    )
}

fn compile_package_text(
    package_id: &str,
    text: &str,
    temp_prefix: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    compile_package_with_dependencies(package_id, text, temp_prefix, Vec::new(), &[])
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

fn compile_stream_package(
    package_id: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    let text = "function helper(value: number) -> number { return value }\nfunction produce() -> Stream<number> {\n  emit(1)\n  emit(2)\n  return\n}\nfunction consume(values: Stream<number>) -> number {\n  final stream = values\n  for item in stream {\n    helper(item)\n  }\n  return 0\n}\nfunction value() -> number { return 42 }\n";
    compile_package_text(package_id, text, "skiff-vm-stream")
}

fn compile_producer_package(
    package_id: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    let text = "function produce() -> Stream<number> {\n  emit(1)\n  emit(2)\n  return\n}\nfunction consume(values: Stream<number>) -> number {\n  return 0\n}\nfunction value() -> number { return 42 }\n";
    compile_package_text(package_id, text, "skiff-vm-producer")
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

fn detached_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn operation_descriptor(
    operation_id: ContractOperationId,
    stable_key: &str,
    return_value: BoundaryReturn,
    stream: BoundaryStreamContract,
) -> BoundaryOperationDescriptor {
    BoundaryOperationDescriptor {
        operation_id,
        stable_key: stable_key.to_string(),
        contract: BoundaryOperationContract {
            parameters: Vec::new(),
            return_value,
            stream,
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
    }
}

fn stream_service_contract(
    package_id: &str,
) -> (Arc<ServiceContract>, BTreeMap<String, ContractOperationId>) {
    let produce =
        skiff_artifact_identity::contract_operation_id(package_id, "1.0.0", "produce").unwrap();
    let consume =
        skiff_artifact_identity::contract_operation_id(package_id, "1.0.0", "consume").unwrap();
    let value =
        skiff_artifact_identity::contract_operation_id(package_id, "1.0.0", "value").unwrap();
    let void_return = BoundaryReturn {
        ty: ContractTypeRef::builtin("void"),
        value_plan: detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call),
    };
    let number_return = BoundaryReturn {
        ty: ContractTypeRef::builtin("number"),
        value_plan: detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call),
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([
            (
                produce.clone(),
                operation_descriptor(
                    produce.clone(),
                    "produce",
                    void_return.clone(),
                    BoundaryStreamContract::ServerStream {
                        item_type: ContractTypeRef::builtin("number"),
                        item_value_plan: detached_plan(
                            BoundaryValueOwner::Provider,
                            BoundaryValueLifetime::Stream,
                        ),
                    },
                ),
            ),
            (
                consume.clone(),
                operation_descriptor(
                    consume.clone(),
                    "consume",
                    number_return.clone(),
                    BoundaryStreamContract::Unary,
                ),
            ),
            (
                value.clone(),
                operation_descriptor(
                    value.clone(),
                    "value",
                    number_return,
                    BoundaryStreamContract::Unary,
                ),
            ),
        ]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: skiff_artifact_model::ContractDiagnosticText {
            service: package_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (
        Arc::new(contract),
        BTreeMap::from([
            ("produce".to_string(), produce),
            ("consume".to_string(), consume),
            ("value".to_string(), value),
        ]),
    )
}

fn service_deployment_with_operations(
    package: &PackageArtifact,
    contract: &ServiceContract,
    operation_bindings: &[(ContractOperationId, PackageCallableId)],
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let package_ref = skiff_artifact_identity::package_artifact_ref(package).unwrap();
    let contract_ref = skiff_artifact_identity::service_contract_ref(contract).unwrap();
    let bindings = operation_bindings
        .iter()
        .map(|(operation, callable)| DeploymentOperationBinding {
            contract_operation_id: operation.clone(),
            package_callable_id: callable.clone(),
        })
        .collect();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref,
        deployment_revision: DeploymentRevision::new("revision:vm-vertical-stream"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref,
        operation_bindings: bindings,
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "vm vertical stream".to_string(),
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

    struct StreamTestImage {
        verified: Arc<VerifiedLinkedBytecodeImage>,
        image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        operations: BTreeMap<String, ContractOperationId>,
    }

    impl StreamTestImage {
        fn new(package_id: &str) -> Self {
            Self::from_compiled(package_id, compile_stream_package(package_id))
        }

        fn producer_new(package_id: &str) -> Self {
            Self::from_compiled(package_id, compile_producer_package(package_id))
        }

        fn from_compiled(
            package_id: &str,
            (package, bytecode): (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>),
        ) -> Self {
            let (contract, operations) = stream_service_contract(package_id);
            let callable = |symbol: &str| {
                skiff_compiler_core::implementation_package_callable_id(
                    package_id,
                    "main",
                    symbol,
                    skiff_compiler_core::ImplementationCallableKind::Function,
                )
                .unwrap()
            };
            let produce = callable("main.produce");
            let consume = callable("main.consume");
            let value = callable("main.value");
            for callable in [&produce, &consume, &value] {
                assert!(
                    package.callable_links.contains_key(callable),
                    "stream package is missing callable {callable}"
                );
            }
            let bindings = [
                (operations.get("produce").unwrap().clone(), produce),
                (operations.get("consume").unwrap().clone(), consume),
                (operations.get("value").unwrap().clone(), value),
            ];
            let (deployment, deployment_reference) =
                service_deployment_with_operations(&package, &contract, &bindings);
            let resolver = TestResolver {
                deployment,
                contract,
                package,
                bytecode,
            };
            let hydrated = DeploymentBytecodeLoader::new(&resolver)
                .load(&deployment_reference)
                .unwrap();
            let candidate = link_deployment(&hydrated, &generous_link_limits()).unwrap();
            let verified =
                Arc::new(verify(hydrated, candidate, &generous_verification_limits()).unwrap());
            let image = Arc::new(DeploymentImage::try_new(Arc::clone(&verified)).unwrap());
            Self {
                verified,
                image,
                operations,
            }
        }

        fn start(&self, stable_key: &str, arguments: Box<[ValueSlot]>) -> VmFiber {
            let operation = self.operations.get(stable_key).unwrap().clone();
            let entry = self.verified.operation_entry(&operation).unwrap();
            let pinned = PinnedDeploymentEntry::try_new(Arc::clone(&self.image), entry).unwrap();
            Vm::start(pinned, arguments, vm_limits()).unwrap()
        }
    }

    fn stream_endpoint() -> ValueSlot {
        ValueSlot::resource_ref(VmHandle::new(7), CompactTypeTag::new(0), ValueFlags::new(0))
    }

    fn next_emit(
        fiber: &mut VmFiber,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> StreamItem {
        loop {
            match fiber.run_segment(heap, budget) {
                VmControl::EmitStream(item) => return item,
                VmControl::Continue => {}
                _ => panic!("expected stream emission, got unsupported control"),
            }
        }
    }

    fn next_child(
        fiber: &mut VmFiber,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> ChildInvocation {
        loop {
            match fiber.run_segment(heap, budget) {
                VmControl::EnterChild(invocation) => return invocation,
                VmControl::Continue => {}
                other => panic!(
                    "expected stream child handoff, got {}",
                    match other {
                        VmControl::Continue => "continue".to_string(),
                        VmControl::Complete(Ok(_)) => "complete(ok)".to_string(),
                        VmControl::Complete(Err(error)) => format!("complete(err): {error}"),
                        VmControl::EnterChild(_) => "enter_child".to_string(),
                        VmControl::EnterAdapter(_) => "enter_adapter".to_string(),
                        VmControl::EmitStream(_) => "emit_stream".to_string(),
                        VmControl::Park(_) => "park".to_string(),
                    }
                ),
            }
        }
    }

    fn run_to_complete(
        fiber: &mut VmFiber,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> VmOwnedValues {
        loop {
            match fiber.run_segment(heap, budget) {
                VmControl::Continue => {}
                VmControl::Complete(result) => return result.unwrap(),
                _ => panic!("expected completion, got unsupported control"),
            }
        }
    }

    fn resume_outcome_tag(outcome: &ResumeOutcome) -> &'static str {
        match outcome {
            ResumeOutcome::Values(_) => "values",
            ResumeOutcome::Empty => "empty",
            ResumeOutcome::StreamEnd => "stream_end",
            ResumeOutcome::Throw(_) => "throw",
            ResumeOutcome::Failure(_) => "failure",
            ResumeOutcome::InternalTerminal(_) => "internal_terminal",
        }
    }

    #[test]
    fn stream_end_is_distinct_from_values_empty_and_failure() {
        let image = StreamTestImage::producer_new("example.com/vm-stream-end-outcome");
        let values = ResumeOutcome::Values(VmOwnedValues::empty(Arc::clone(&image.verified)));
        let empty = ResumeOutcome::Empty;
        let stream_end = ResumeOutcome::StreamEnd;
        let failure = ResumeOutcome::Failure(VmError::ResumeNotExpected);

        assert_eq!(resume_outcome_tag(&values), "values");
        assert_eq!(resume_outcome_tag(&empty), "empty");
        assert_eq!(resume_outcome_tag(&stream_end), "stream_end");
        assert_eq!(resume_outcome_tag(&failure), "failure");
    }

    #[test]
    fn stream_end_resume_fails_closed_when_end_pc_is_missing() {
        let image = StreamTestImage::producer_new("example.com/vm-stream-end-missing");
        let mut heap = TestHeap;
        let mut budget = TestBudget::new();
        let mut fiber = image.start("produce", Box::new([]));
        let item = next_emit(&mut fiber, &mut heap, &mut budget);
        let (_item_values, resume) = item.into_parts();

        assert_eq!(resume.end_resume_pc(), None);
        assert_eq!(
            fiber.resume(resume, ResumeOutcome::StreamEnd),
            Err(VmError::StreamEndResumeUnavailable)
        );
        assert_eq!(fiber.state(), VmFiberState::Terminal);
    }

    #[test]
    fn source_to_vm_scalar_tail_call_executes_through_the_verified_entry() {
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
        let candidate = link_deployment(&hydrated, &generous_link_limits()).unwrap();
        let verified =
            Arc::new(verify(hydrated, candidate, &generous_verification_limits()).unwrap());
        let image = Arc::new(DeploymentImage::try_new(Arc::clone(&verified)).unwrap());
        let operation =
            skiff_artifact_identity::contract_operation_id("example.com/vm-scalar", "1.0.0", "run")
                .unwrap();
        let entry = verified.operation_entry(&operation).unwrap();
        let pinned = PinnedDeploymentEntry::try_new(Arc::clone(&image), entry).unwrap();
        let mut fiber =
            Vm::start(pinned, Box::new([ValueSlot::number(41.0)]), vm_limits()).unwrap();
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

    #[test]
    fn stream_producer_natural_end_resumes_and_clears_continuation() {
        let image = StreamTestImage::producer_new("example.com/vm-stream");
        let mut heap = TestHeap;
        let mut budget = TestBudget::new();
        let mut fiber = image.start("produce", Box::new([]));
        let mut emitted = Vec::new();

        loop {
            match fiber.run_segment(&mut heap, &mut budget) {
                VmControl::EmitStream(item) => {
                    let (item_values, resume) = item.into_parts();
                    emitted.push(
                        item_values.values()[0]
                            .as_number()
                            .expect("stream producer item must be a number"),
                    );
                    let outcome = if emitted.len() == 1 {
                        ResumeOutcome::Values(VmOwnedValues::empty(Arc::clone(&image.verified)))
                    } else {
                        ResumeOutcome::Empty
                    };
                    fiber.resume(resume, outcome).unwrap();
                }
                VmControl::Complete(Ok(values)) => {
                    assert!(
                        values.is_empty(),
                        "stream producer has no call return value"
                    );
                    assert_eq!(emitted, vec![1.0, 2.0]);
                    assert_eq!(fiber.state(), VmFiberState::Terminal);

                    let mut foreign = image.start("produce", Box::new([]));
                    let foreign_item = next_emit(&mut foreign, &mut heap, &mut budget);
                    let (_foreign_values, foreign_resume) = foreign_item.into_parts();
                    assert_eq!(
                        fiber.resume(foreign_resume, ResumeOutcome::Empty),
                        Err(VmError::ResumeNotExpected)
                    );

                    fiber.discard_terminal_roots(&mut heap).unwrap();
                    break;
                }
                VmControl::Continue => {}
                _ => panic!("stream producer produced an unsupported control"),
            }
        }
    }

    #[test]
    fn stream_backpressure_handoff_parks_and_resumes_zero_result() {
        let image = StreamTestImage::producer_new("example.com/vm-stream");
        let mut heap = TestHeap;
        let mut budget = TestBudget::new();
        let mut fiber = image.start("produce", Box::new([]));
        let ticket = PendingTicket::new(NonZeroU64::new(7).unwrap());

        let item = next_emit(&mut fiber, &mut heap, &mut budget);
        let (_first_item, resume) = item.into_parts();
        fiber.resume(resume, ResumeOutcome::Empty).unwrap();

        let item = next_emit(&mut fiber, &mut heap, &mut budget);
        let (item_values, pending) = item.into_pending(ticket);
        assert_eq!(item_values.values()[0].as_number(), Some(2.0));
        assert_eq!(pending.ticket(), ticket);
        let (_ticket, resume) = pending.into_parts();
        let _supervisor_item = item_values;
        fiber.resume(resume, ResumeOutcome::Empty).unwrap();

        let values = run_to_complete(&mut fiber, &mut heap, &mut budget);
        assert!(values.is_empty());
    }

    #[test]
    fn stream_next_hands_off_affine_endpoint_and_resumes_item_then_end() {
        let image = StreamTestImage::new("example.com/vm-stream");
        let mut heap = TestHeap;
        let mut budget = TestBudget::new();
        let mut consumer = image.start("consume", Box::new([stream_endpoint()]));

        let invocation = next_child(&mut consumer, &mut heap, &mut budget);
        assert_eq!(invocation.target(), ChildTarget::StreamNext);
        assert!(invocation.arguments().is_empty());
        let (target, endpoint_escrow, resume) = invocation.into_parts();
        assert_eq!(target, ChildTarget::StreamNext);
        assert!(resume.end_resume_pc().is_some());
        let _endpoint_escrow = endpoint_escrow;

        let mut item_fiber = image.start("value", Box::new([]));
        let item_values = run_to_complete(&mut item_fiber, &mut heap, &mut budget);
        assert_eq!(item_values.len(), 1);
        assert_eq!(item_values.values()[0].as_number(), Some(42.0));
        consumer
            .resume(resume, ResumeOutcome::Values(item_values))
            .unwrap();

        let invocation = next_child(&mut consumer, &mut heap, &mut budget);
        let (target, endpoint_escrow, resume) = invocation.into_parts();
        assert_eq!(target, ChildTarget::StreamNext);
        assert!(resume.end_resume_pc().is_some());
        let _endpoint_escrow = endpoint_escrow;
        consumer
            .resume(resume, ResumeOutcome::StreamEnd)
            .unwrap();

        let values = run_to_complete(&mut consumer, &mut heap, &mut budget);
        assert_eq!(values.len(), 1);
        assert_eq!(values.values()[0].as_number(), Some(0.0));
    }

    #[test]
    fn stream_resume_tokens_reject_authority_and_image_reuse() {
        let image_a = StreamTestImage::new("example.com/vm-stream");
        let image_b = StreamTestImage::new("example.com/vm-stream-other");
        let mut heap = TestHeap;
        let mut budget = TestBudget::new();

        let mut producer_a = image_a.start("produce", Box::new([]));
        let item_a = next_emit(&mut producer_a, &mut heap, &mut budget);
        let (_item_a_values, token_a) = item_a.into_parts();

        let mut producer_b = image_b.start("produce", Box::new([]));
        let item_b = next_emit(&mut producer_b, &mut heap, &mut budget);
        let (_item_b_values, token_b) = item_b.into_parts();

        let mut consumer = image_a.start("consume", Box::new([stream_endpoint()]));
        let invocation = next_child(&mut consumer, &mut heap, &mut budget);
        let (_target, endpoint_escrow, token_child) = invocation.into_parts();
        let _endpoint_escrow = endpoint_escrow;

        assert_eq!(
            producer_a.resume(token_child, ResumeOutcome::Empty),
            Err(VmError::ResumeTokenMismatch)
        );
        assert_eq!(
            producer_a.resume(token_b, ResumeOutcome::Empty),
            Err(VmError::ResumeTokenMismatch)
        );

        producer_a.resume(token_a, ResumeOutcome::Empty).unwrap();
        let item = next_emit(&mut producer_a, &mut heap, &mut budget);
        let (_item_values, resume) = item.into_parts();
        producer_a.resume(resume, ResumeOutcome::Empty).unwrap();
        let values = run_to_complete(&mut producer_a, &mut heap, &mut budget);
        assert!(values.is_empty());
    }
}
