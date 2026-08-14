// These integration tests keep the request boundary on the Phase 1 synchronous,
// immediate-scalar surface. Disabled value shapes and pending effects are covered
// as typed compiler-containment negatives below.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};

use skiff_artifact_identity::{gateway_entry_identity, ValidatedBytecodeArtifact};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentOperationBinding, DeploymentRevision, GatewayAdapterArg,
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, IngressProtocol, IngressSelector, PackageArtifact, PackageCallableId,
    PackageLocalAbiSymbol, ServiceContract, ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, BytecodeEmissionError, CompilerPlatformSources, ManifestOwner,
    ManifestProvenance, PackageCompileError, PackageCompileInput, PackageCompileOutput,
    PackageSourceInput, Phase1UnsupportedCapability, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_runtime_bytecode_verifier::VerificationLimits;
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionEntry, DeploymentExecutionImage,
    DeploymentExecutionLimits, LinkLimits,
};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader, HydratedDeploymentBytecode,
};
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
};
use skiff_runtime_request::{
    drive_runtime_bytecode_request, BinaryHttpRequest, BinaryHttpRequestMetadata, BoundaryResponse,
    BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput,
    DrivenBytecodeRequestOwnerInventory, ExecutionBudget,
    GatewayAdapterArg as RequestGatewayAdapterArg,
    GatewayAdapterSource as RequestGatewayAdapterSource, HttpAdapter, HttpAdapterCallable,
    HttpAdapterKind, RequestEnvelope, RequestError, RequestExecutionOwnerInventorySnapshot,
    ResponseEnd, ResponseEvent,
};

fn compile_scalar_package() -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    compile_scalar_package_with_source(
        "function run() -> number {
  return 2.0
}
",
    )
}

fn compile_scalar_package_with_source(
    text: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    let compiled = compile_test_package_with_source(text).unwrap();
    let handoff = compiled.bytecode_handoff().unwrap();
    let package_artifact = Arc::new(compiled.package().artifact.clone());
    let bytecode = Arc::new(ValidatedBytecodeArtifact::admit(handoff.artifact().clone()).unwrap());
    (package_artifact, bytecode)
}

fn compile_test_package_with_source(
    text: &str,
) -> Result<PackageCompileOutput, PackageCompileError> {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("request manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let package_id =
        skiff_compiler_core::id::PublicationId::parse("example.com/vm-scalar").unwrap();
    static NEXT_TEST_TEMP: AtomicU64 = AtomicU64::new(0);
    let temp = std::env::temp_dir().join(format!(
        "skiff-request-vm-scalar-{}-{}-{}",
        std::process::id(),
        NEXT_TEST_TEMP.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).unwrap();
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
            package_id,
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
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
    let input = PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        "example.com/vm-scalar",
        true,
    );
    let compiled = compile_package(input);
    std::fs::remove_dir_all(temp).unwrap();
    compiled
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
        deployment_revision: DeploymentRevision::new("revision:request-vm-scalar"),
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
            display_name: "request vm scalar".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

fn implementation_callable_id(package: &PackageArtifact, selector: &str) -> PackageCallableId {
    let symbol = package
        .package_local_abi
        .implementation_symbols
        .get(selector)
        .unwrap_or_else(|| panic!("compiled scalar package has no callable {selector}"));
    match symbol {
        PackageLocalAbiSymbol::Callable { callable_id, .. } => callable_id.clone(),
        _ => panic!("compiled scalar symbol {selector} is not callable"),
    }
}

fn scalar_gateway_contract(package_id: &str) -> Arc<ServiceContract> {
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::new(),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: skiff_artifact_model::ContractDiagnosticText {
            service: package_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    Arc::new(contract)
}

fn scalar_gateway_entry(
    key: &str,
    handler: PackageCallableId,
    schema: GatewayExternalSchema,
) -> (GatewayEntryKey, DeploymentGatewayEntry) {
    let key = GatewayEntryKey::parse(key).unwrap();
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(schema.clone()),
            response_schema: Some(schema),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity = gateway_entry_identity(&protocol_surface).unwrap();
    let entry = DeploymentGatewayEntry {
        gateway_entry_identity: identity,
        protocol_surface,
        handler: Some(handler),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![GatewayAdapterArg {
                param: "value".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        },
        close_handler: None,
        close_adapter_plan: None,
    };
    (key, entry)
}

fn scalar_gateway_deployment(
    package: &PackageArtifact,
    contract: &ServiceContract,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
    BTreeMap<&'static str, (IngressSelector, GatewayEntryIdentity)>,
) {
    let definitions = [
        ("number", "main.numberBody", GatewayExternalSchema::Number),
        ("bool", "main.boolBody", GatewayExternalSchema::Boolean),
        ("null", "main.nullBody", GatewayExternalSchema::Null),
    ];
    let mut gateway_entries = BTreeMap::new();
    let mut ingress = Vec::new();
    let mut keys = BTreeMap::new();
    for (name, selector, schema) in definitions {
        let (key, entry) =
            scalar_gateway_entry(name, implementation_callable_id(package, selector), schema);
        let selector = IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: format!("/{name}"),
        };
        let identity = entry.gateway_entry_identity.clone();
        ingress.push(DeploymentIngressBinding {
            selector: selector.clone(),
            gateway_entry_key: key.clone(),
        });
        keys.insert(name, (selector, identity));
        gateway_entries.insert(key, entry);
    }

    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:request-vm-scalar-gateways"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries,
        ingress,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "request vm scalar gateways".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference, keys)
}

struct ScalarGatewayFixture {
    image: Arc<DeploymentExecutionImage>,
    keys: BTreeMap<&'static str, (IngressSelector, GatewayEntryIdentity)>,
}

impl ScalarGatewayFixture {
    fn build() -> Self {
        let (package, bytecode) = compile_scalar_package_with_source(
            "function numberBody(value: number) -> number {
  return value + 1.0
}

function boolBody(value: bool) -> bool {
  return value
}

function nullBody(value: null) -> null {
  return value
}
",
        );
        let contract = scalar_gateway_contract(package.package_id.as_str());
        let (deployment, deployment_reference, keys) =
            scalar_gateway_deployment(&package, &contract);
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .unwrap();
        let image = execution_image(hydrated);
        Self { image, keys }
    }

    fn target(&self, name: &str) -> DeploymentExecutionEntry {
        let (selector, identity) = self
            .keys
            .get(name)
            .unwrap_or_else(|| panic!("unknown scalar gateway {name}"));
        self.image.http_gateway_entry(selector, identity).unwrap()
    }
}

fn scalar_gateway_fixture() -> &'static ScalarGatewayFixture {
    static FIXTURE: OnceLock<ScalarGatewayFixture> = OnceLock::new();
    FIXTURE.get_or_init(ScalarGatewayFixture::build)
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

fn execution_image(hydrated: HydratedDeploymentBytecode) -> Arc<DeploymentExecutionImage> {
    Arc::new(
        link_deployment_execution_image(
            hydrated,
            &DeploymentExecutionLimits::new(generous_link_limits(), generous_verification_limits()),
        )
        .unwrap(),
    )
}

fn request_envelope() -> RequestEnvelope {
    RequestEnvelope {
        request_id: "scalar-bytecode-request".to_string(),
        mode: "unary".to_string(),
        target: "display-only".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some("example.com/vm-scalar".to_string()),
        build_id: "legacy-build".to_string(),
        service_protocol_identity: "legacy-protocol".to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/entry".to_string(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn scalar_gateway_request(
    name: &str,
    kind: HttpAdapterKind,
    body: &[u8],
    adapter_args: Vec<RequestGatewayAdapterArg>,
) -> RequestEnvelope {
    let mut request = request_envelope();
    let path = format!("/{name}");
    request.ingress_selector = Some(IngressSelector {
        protocol: IngressProtocol::Http,
        method: Some("POST".to_string()),
        path: path.clone(),
    });
    request.binary_http = Some(BinaryHttpRequest {
        metadata: BinaryHttpRequestMetadata {
            method: "POST".to_string(),
            url: format!("https://example.test{path}"),
            path,
            query: Vec::new(),
            headers: Vec::new(),
        },
        body: body.to_vec(),
    });
    request.http_adapter = Some(HttpAdapter {
        kind,
        handler: HttpAdapterCallable::PackageFunction {
            package_id: "example.com/vm-scalar".to_string(),
            symbol_path: name.to_string(),
        },
        guard: None,
        pre: None,
        adapter_args,
    });
    request
}

fn http_body_argument() -> RequestGatewayAdapterArg {
    RequestGatewayAdapterArg {
        param: "value".to_string(),
        source: RequestGatewayAdapterSource::HttpBody,
    }
}

fn noop_observer() -> BytecodeExecutionObserver {
    BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
        router_session_id: "request-test-session".to_string(),
        request_id: "request-test".to_string(),
    })
}

fn run_synchronous_request(
    input: BytecodeRequestExecutionInput,
) -> Result<BoundaryResponse, RequestError> {
    let driven = drive_runtime_bytecode_request(input);
    let snapshot = driven.owner_inventory.into_snapshot();
    assert_synchronous_owner_inventory(&snapshot);
    drop(driven.retention);
    driven.result
}

fn assert_synchronous_owner_inventory(snapshot: &RequestExecutionOwnerInventorySnapshot) {
    for (domain_name, domain) in [
        ("pending", snapshot.pending),
        ("resource", snapshot.resource),
        ("child", snapshot.child),
    ] {
        assert_eq!(
            domain.current, 0,
            "synchronous Phase 1 request created a live {domain_name} owner"
        );
        assert!(
            !domain.ever_created,
            "synchronous Phase 1 request ever created a {domain_name} owner"
        );
    }
}

fn execute_scalar_gateway(
    name: &str,
    kind: HttpAdapterKind,
    body: &[u8],
    adapter_args: Vec<RequestGatewayAdapterArg>,
) -> Result<BoundaryResponse, RequestError> {
    run_synchronous_request(BytecodeRequestExecutionInput {
        target: scalar_gateway_fixture().target(name),
        request: scalar_gateway_request(name, kind, body, adapter_args),
        observer: noop_observer(),
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
        },
        http_client: None,
        heap: None,
    })
}

fn response_payload(response: BoundaryResponse) -> Vec<u8> {
    let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload))) = response
    else {
        panic!("scalar gateway returned a non-payload response: {response:?}");
    };
    payload
}

fn assert_phase_1_compiler_rejection(
    error: PackageCompileError,
    expected_capability: Phase1UnsupportedCapability,
    expected_location: &str,
) {
    let PackageCompileError::BytecodeEmission {
        source:
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability,
                module_path,
                function_key,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 1 compiler containment, got {error:?}");
    };
    assert_eq!(capability, expected_capability);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, expected_location);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_request_freezes_started_all_zero_inventory() {
        let driven = drive_runtime_bytecode_request(BytecodeRequestExecutionInput {
            target: scalar_gateway_fixture().target("number"),
            request: scalar_gateway_request(
                "number",
                HttpAdapterKind::TypedJson,
                b"2",
                vec![http_body_argument()],
            ),
            observer: noop_observer(),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
            handles: BytecodeRequestExecutionHandles {
                request_heap_limits: RequestHeapLimits::default(),
            },
            http_client: None,
            heap: None,
        });

        match driven.owner_inventory {
            DrivenBytecodeRequestOwnerInventory::Started(snapshot) => {
                assert_synchronous_owner_inventory(&snapshot);
            }
            DrivenBytecodeRequestOwnerInventory::NotStarted(_) => {
                panic!("completed drive must freeze a Started owner inventory");
            }
        }
        let response = driven.result.expect("scalar gateway request must complete");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response_payload(response)).unwrap(),
            serde_json::json!(3.0)
        );
        drop(driven.retention);
    }

    #[test]
    fn malformed_typed_json_body_freezes_not_started_all_zero_inventory() {
        let driven = drive_runtime_bytecode_request(BytecodeRequestExecutionInput {
            target: scalar_gateway_fixture().target("number"),
            request: scalar_gateway_request(
                "number",
                HttpAdapterKind::TypedJson,
                b"{",
                vec![http_body_argument()],
            ),
            observer: noop_observer(),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
            handles: BytecodeRequestExecutionHandles {
                request_heap_limits: RequestHeapLimits::default(),
            },
            http_client: None,
            heap: None,
        });

        match driven.owner_inventory {
            DrivenBytecodeRequestOwnerInventory::NotStarted(snapshot) => {
                assert_synchronous_owner_inventory(&snapshot);
            }
            DrivenBytecodeRequestOwnerInventory::Started(_) => {
                panic!("start failure must freeze a NotStarted owner inventory");
            }
        }
        assert!(matches!(
            driven.result,
            Err(RequestError::Decode(ref message)) if message.contains("not valid JSON")
        ));
        drop(driven.retention);
    }

    #[test]
    fn typed_json_number_body_materializes_against_pinned_entry() {
        let response = execute_scalar_gateway(
            "number",
            HttpAdapterKind::TypedJson,
            b"2",
            vec![http_body_argument()],
        )
        .unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response_payload(response)).unwrap(),
            serde_json::json!(3.0)
        );
    }

    #[test]
    fn typed_json_bool_and_null_bodies_materialize_as_immediates() {
        for (name, body, expected) in [
            ("bool", b"true".as_slice(), serde_json::json!(true)),
            ("null", b"null".as_slice(), serde_json::Value::Null),
        ] {
            let response = execute_scalar_gateway(
                name,
                HttpAdapterKind::TypedJson,
                body,
                vec![http_body_argument()],
            )
            .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&response_payload(response)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn typed_json_malformed_body_fails_closed_before_vm_start() {
        let error = execute_scalar_gateway(
            "number",
            HttpAdapterKind::TypedJson,
            b"{",
            vec![http_body_argument()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Decode(message) if message.contains("not valid JSON")
        ));
    }

    #[test]
    fn typed_json_body_must_match_exact_pinned_parameter_type() {
        let error = execute_scalar_gateway(
            "number",
            HttpAdapterKind::TypedJson,
            b"true",
            vec![http_body_argument()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Decode(message)
                if message.contains("exact pinned number type for parameter 0")
        ));
    }

    #[test]
    fn typed_json_non_scalar_bodies_fail_closed() {
        for body in [b"[]".as_slice(), b"{\"value\":2}".as_slice()] {
            let error = execute_scalar_gateway(
                "number",
                HttpAdapterKind::TypedJson,
                body,
                vec![http_body_argument()],
            )
            .unwrap_err();
            assert!(matches!(
                error,
                RequestError::Unsupported(message) if message.contains("is non-scalar")
            ));
        }
    }

    #[test]
    fn typed_json_adapter_arity_must_match_exact_pinned_entry() {
        let error = execute_scalar_gateway("number", HttpAdapterKind::TypedJson, b"2", Vec::new())
            .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Decode(message)
                if message.contains("0 arguments") && message.contains("1 parameters")
        ));
    }

    #[test]
    fn raw_http_body_remains_heap_bytes() {
        let error = execute_scalar_gateway(
            "number",
            HttpAdapterKind::RawHttp,
            b"2",
            vec![http_body_argument()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Unsupported(message) if message.contains("expected number")
        ));
    }

    #[test]
    fn request_heap_scalar_returns_payload() {
        let (package, bytecode) = compile_scalar_package();
        let (contract, operation_id) = service_contract(package.package_id.as_str());
        let (deployment, deployment_reference) =
            service_deployment(&package, &contract, operation_id.clone());
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .unwrap();
        let image = execution_image(hydrated);
        let target = image.operation_entry(&operation_id).unwrap();
        let response = run_synchronous_request(BytecodeRequestExecutionInput {
            target,
            request: request_envelope(),
            observer: noop_observer(),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
            handles: BytecodeRequestExecutionHandles {
                request_heap_limits: RequestHeapLimits::default(),
            },
            http_client: None,
            heap: None,
        })
        .unwrap();

        let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload))) = response
        else {
            panic!("bytecode request returned a non-payload response: {response:?}");
        };
        assert_eq!(serde_json::from_slice::<f64>(&payload).unwrap(), 2.0);
    }

    #[test]
    fn sleep_pending_effect_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "import std
function run() -> number {
  std.time.sleep(Duration.milliseconds(1))
  return 1.0
}
",
        )
        .unwrap_err();

        assert_phase_1_compiler_rejection(
            error,
            Phase1UnsupportedCapability::PendingEffect,
            "callable pending effects",
        );
    }

    #[test]
    fn string_shape_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "function run() -> string {
  return \"disabled\"
}
",
        )
        .unwrap_err();

        assert_phase_1_compiler_rejection(
            error,
            Phase1UnsupportedCapability::ValueShape,
            "return type",
        );
    }

    #[test]
    fn aggregate_shape_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "function run() -> Array<string> {
  return [\"a\", \"b\"]
}
",
        )
        .unwrap_err();

        let PackageCompileError::BytecodeEmission {
            source:
                BytecodeEmissionError::UnsupportedConstruct {
                    construct,
                    location,
                    ..
                },
        } = error
        else {
            panic!("expected typed Phase 2 aggregate containment, got {error:?}");
        };
        assert_eq!(construct, "phase 2 record/array value shape");
        assert!(location.contains("return type"), "{location}");
        assert!(location.contains("ValueShape"), "{location}");
    }

    #[test]
    fn native_bytes_wrapper_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "function run() -> bytes {
  return bytes.fromUtf8(\"disabled\")
}
",
        )
        .unwrap_err();

        assert_phase_1_compiler_rejection(
            error,
            Phase1UnsupportedCapability::HostTarget,
            "native executable",
        );
    }
}
